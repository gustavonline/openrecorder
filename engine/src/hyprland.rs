//! Optional Hyprland window adapter. IPC is restricted to this process's own windows
//! (this pid, exact titles "OpenRecorder" and "OpenRecorder Camera"): other windows,
//! this app's pickers, pill and Settings, and every other client are never touched.
//! No window-manager configuration or rules are written.
//!
//! Everything that decides what to send is pure (`plan`, `parse_monitors`) and fed
//! with the compositor's JSON, so it is tested without Hyprland. Units are
//! Hyprland's layout units: monitor `width`/`height` are divided by `scale`, and a
//! client's `at`/`size` are compared against the monitor's `x`/`y` origin.
use crate::desktop::{bubble_base, bubble_dimensions, default_toolbar_position, TOOLBAR_H, TOOLBAR_W};
use serde_json::Value;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::{Duration, Instant},
};

const MAIN_TITLE: &str = "OpenRecorder";
const CAMERA_TITLE: &str = "OpenRecorder Camera";
/// A reported window size within this many units of the requested one counts as applied.
const SIZE_TOLERANCE: f64 = 1.5;
/// Gap between the camera bubble and the monitor edge.
const CAMERA_INSET: f64 = 24.;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Monitor {
    id: i64,
    x: f64,
    y: f64,
    /// Logical size: pixels divided by the monitor's scale.
    width: f64,
    height: f64,
}
/// Monitors with every field present. A monitor missing a field is skipped, never
/// completed with a made-up 1920 x 1080 / scale 1.
fn parse_monitors(value: &Value) -> Vec<Monitor> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let scale = m["scale"].as_f64().filter(|s| *s > 0.)?;
            Some(Monitor {
                id: m["id"].as_i64()?,
                x: m["x"].as_f64()?,
                y: m["y"].as_f64()?,
                width: m["width"].as_f64()? / scale,
                height: m["height"].as_f64()? / scale,
            })
        })
        .filter(|m| m.width > 0. && m.height > 0.)
        .collect()
}
fn own_clients(clients: &Value, pid: u64) -> Vec<&Value> {
    clients
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| {
            c["pid"].as_u64() == Some(pid)
                && c["title"].as_str().is_some_and(|t| t == MAIN_TITLE || t == CAMERA_TITLE)
        })
        .collect()
}

/// Camera layout read back from the bubble's own window. `size` is the *base*
/// (a wide window is 4/3 of it) as a fraction of the monitor width, and is only
/// given once the window actually has the size that was asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LayoutUpdate {
    x: f32,
    y: f32,
    size: Option<f32>,
}
#[derive(Debug, Default, PartialEq)]
struct Plan {
    commands: Vec<String>,
    layout: Option<LayoutUpdate>,
}
/// What has already been sent, so an unchanged state sends nothing.
#[derive(Default)]
struct Placed {
    flagged: Vec<String>,
    placed: Vec<String>,
    camera_target: Option<(String, (i32, i32))>,
}
fn resize(address: &str, w: i32, h: i32) -> String {
    format!("dispatch resizewindowpixel exact {w} {h},address:{address}")
}
fn place(address: &str, x: i32, y: i32) -> String {
    format!("dispatch movewindowpixel exact {x} {y},address:{address}")
}
fn plan(placed: &mut Placed, clients: &Value, monitors: &[Monitor], pid: u64, base: f32, shape: u8, toolbar: bool) -> Plan {
    let mut plan = Plan::default();
    let (target_w, target_h) = bubble_dimensions(base, shape);
    let target = (target_w as i32, target_h as i32);
    for client in own_clients(clients, pid) {
        let Some(address) = client["address"].as_str() else { continue };
        let title = client["title"].as_str().unwrap_or_default();
        if !placed.flagged.iter().any(|a| a == address) {
            placed.flagged.push(address.to_owned());
            if client["floating"] == false {
                plan.commands.push(format!("dispatch setfloating address:{address}"));
            }
            if client["pinned"] == false {
                plan.commands.push(format!("dispatch pin address:{address}"));
            }
        }
        // No monitor data for this window: nothing is sized, placed or read back.
        let Some(m) = monitors.iter().find(|m| client["monitor"].as_i64() == Some(m.id)).copied() else { continue };
        let known = placed.placed.iter().any(|a| a == address);
        if title == MAIN_TITLE {
            // Without the toolbar this window is a one-pixel host: it is only kept floating and pinned (above),
            // never sized or placed.
            if !known && toolbar {
                let origin = default_toolbar_position(egui_vec(m.width, m.height));
                plan.commands.push(resize(address, TOOLBAR_W as i32, TOOLBAR_H as i32));
                plan.commands.push(place(address, (m.x + origin.x as f64) as i32, (m.y + origin.y as f64) as i32));
                placed.placed.push(address.to_owned());
            }
            continue;
        }
        let mut moved = false;
        if !known {
            plan.commands.push(resize(address, target.0, target.1));
            plan.commands.push(place(
                address,
                (m.x + m.width - target.0 as f64 - CAMERA_INSET) as i32,
                (m.y + m.height - target.1 as f64 - 60.) as i32,
            ));
            placed.placed.push(address.to_owned());
            placed.camera_target = Some((address.to_owned(), target));
            moved = true;
        } else if placed.camera_target.as_ref() != Some(&(address.to_owned(), target)) {
            // Shape or size changed: resize once, and pull the window back onto its monitor if it now overhangs.
            plan.commands.push(resize(address, target.0, target.1));
            placed.camera_target = Some((address.to_owned(), target));
            if let (Some(ax), Some(ay)) = (client["at"][0].as_f64(), client["at"][1].as_f64()) {
                let nx = ax.clamp(m.x, (m.x + m.width - target.0 as f64).max(m.x));
                let ny = ay.clamp(m.y, (m.y + m.height - target.1 as f64).max(m.y));
                if (nx - ax).abs() > 1. || (ny - ay).abs() > 1. {
                    plan.commands.push(place(address, nx as i32, ny as i32));
                    moved = true;
                }
            }
        }
        // The window's own position drives the composite, but not on a pass that just moved it.
        if moved {
            continue;
        }
        let (Some(ax), Some(ay)) = (client["at"][0].as_f64(), client["at"][1].as_f64()) else { continue };
        let applied = matches!(
            (client["size"][0].as_f64(), client["size"][1].as_f64()),
            (Some(w), Some(h)) if (w - target.0 as f64).abs() <= SIZE_TOLERANCE && (h - target.1 as f64).abs() <= SIZE_TOLERANCE
        );
        plan.layout = Some(LayoutUpdate {
            x: ((ax - m.x) / m.width) as f32,
            y: ((ay - m.y) / m.height) as f32,
            size: applied.then(|| bubble_base(client["size"][0].as_f64().unwrap_or_default() as f32, shape) / m.width as f32),
        });
    }
    plan
}
fn egui_vec(width: f64, height: f64) -> eframe::egui::Vec2 {
    eframe::egui::vec2(width as f32, height as f32)
}

#[derive(Default)]
pub struct Desktop {
    socket: Option<PathBuf>,
    looked: bool,
    last: Option<Instant>,
    placed: Placed,
}
impl Desktop {
    pub fn ready(&self) -> bool {
        !self.placed.flagged.is_empty() || !self.placed.placed.is_empty()
    }
    pub fn available(&self) -> bool {
        self.socket.is_some()
    }
    fn request(&self, command: &str) -> Option<String> {
        let mut socket = UnixStream::connect(self.socket.as_ref()?).ok()?;
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .ok()?;
        socket
            .set_write_timeout(Some(Duration::from_millis(100)))
            .ok()?;
        socket.write_all(command.as_bytes()).ok()?;
        let mut data = String::new();
        socket.take(1024 * 1024).read_to_string(&mut data).ok()?;
        Some(data)
    }
    /// `camera_base` is the bubble's base size in points (S/M/L = 120/168/224); the
    /// shape comes from the shared Layout, so a wide window is `base * 4/3` by `base * 3/4`.
    pub fn refresh(
        &mut self,
        layout: &std::sync::Arc<std::sync::Mutex<crate::pipeline::Layout>>,
        camera_base: f32,
        toolbar: bool,
    ) -> bool {
        if !self.looked {
            self.looked = true;
            let base = PathBuf::from(
                std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".into()),
            )
            .join("hypr");
            self.socket = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
                .ok()
                .map(|s| base.join(s).join(".socket.sock"))
                .filter(|p| p.exists())
                .or_else(|| {
                    let sockets = std::fs::read_dir(base)
                        .ok()?
                        .filter_map(|e| e.ok().map(|e| e.path().join(".socket.sock")))
                        .filter(|p| p.exists())
                        .collect::<Vec<_>>();
                    if sockets.len() == 1 {
                        Some(sockets[0].clone())
                    } else {
                        None
                    }
                });
        }
        if self.socket.is_none() {
            return false;
        }
        if self
            .last
            .is_some_and(|t| t.elapsed() < Duration::from_millis(100))
        {
            return true;
        }
        self.last = Some(Instant::now());
        let clients: Value = match self
            .request("j/clients")
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(v) => v,
            None => return false,
        };
        let pid = std::process::id() as u64;
        let own = own_clients(&clients, pid);
        let needs_monitors = own.iter().any(|c| {
            c["title"] == CAMERA_TITLE
                || !self.placed.placed.iter().any(|a| Some(a.as_str()) == c["address"].as_str())
        });
        let monitors = if needs_monitors {
            self.request("j/monitors")
                .and_then(|s| serde_json::from_str(&s).ok())
                .map(|v| parse_monitors(&v))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let shape = layout.lock().map(|l| l.shape).unwrap_or(1);
        let plan = plan(&mut self.placed, &clients, &monitors, pid, camera_base, shape, toolbar);
        for command in &plan.commands {
            let _ = self.request(command);
        }
        if let (Some(update), Ok(mut l)) = (plan.layout, layout.lock()) {
            l.x = update.x;
            l.y = update.y;
            if let Some(size) = update.size {
                l.size = size;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PID: u64 = 4242;
    fn client(address: &str, title: &str, pid: u64, monitor: i64, at: (f64, f64), size: (f64, f64), floating: bool, pinned: bool) -> Value {
        json!({"address":address,"title":title,"pid":pid,"monitor":monitor,"at":[at.0,at.1],"size":[size.0,size.1],"floating":floating,"pinned":pinned})
    }
    /// Monitor 0: 1920 x 1080 at the origin. Monitor 1: 3840 x 2160 pixels at scale 2 (1920 x 1080 logical), at x = 1920.
    fn monitors() -> Vec<Monitor> {
        parse_monitors(&json!([
            {"id":0,"x":0,"y":0,"width":1920,"height":1080,"scale":1.0},
            {"id":1,"x":1920,"y":0,"width":3840,"height":2160,"scale":2.0}
        ]))
    }
    fn camera(address: &str, monitor: i64, at: (f64, f64), size: (f64, f64)) -> Value {
        client(address, CAMERA_TITLE, PID, monitor, at, size, true, true)
    }

    #[test]
    fn only_this_process_windows_with_the_two_exact_titles_are_ever_touched() {
        let clients = json!([
            client("0xa", MAIN_TITLE, PID, 0, (0., 0.), (580., 72.), false, false),
            client("0xb", MAIN_TITLE, PID + 1, 0, (0., 0.), (100., 100.), false, false),
            client("0xc", "OpenRecorder Settings", PID, 0, (0., 0.), (760., 600.), false, false),
            client("0xd", "OpenRecorder Source", PID, 0, (0., 0.), (520., 460.), false, false),
            client("0xe", "OpenRecorder Notice", PID, 0, (0., 0.), (392., 40.), false, false),
            client("0xf", "Terminal", PID, 0, (0., 0.), (800., 600.), false, false),
            client("0x10", "openrecorder", PID, 0, (0., 0.), (800., 600.), false, false),
            camera("0x11", 0, (10., 10.), (168., 168.)),
        ]);
        let mut placed = Placed::default();
        let plan = plan(&mut placed, &clients, &monitors(), PID, 168., 0, true);
        let touched: std::collections::BTreeSet<&str> = plan.commands.iter().filter_map(|c| c.rsplit("address:").next()).collect();
        assert_eq!(touched, ["0x11", "0xa"].into_iter().collect());
        assert_eq!(own_clients(&clients, PID).len(), 2);
    }
    #[test]
    fn without_the_toolbar_the_root_window_is_only_floated_and_pinned_never_sized_or_placed() {
        let clients = json!([client("0xa", MAIN_TITLE, PID, 0, (0., 0.), (1., 1.), false, false)]);
        let mut placed = Placed::default();
        let first = plan(&mut placed, &clients, &monitors(), PID, 168., 0, false);
        assert_eq!(first.commands, vec!["dispatch setfloating address:0xa", "dispatch pin address:0xa"]);
        assert!(!first.commands.iter().any(|c| c.contains("resizewindowpixel") || c.contains("movewindowpixel")));
        assert_eq!(plan(&mut placed, &clients, &monitors(), PID, 168., 0, false), Plan::default(), "nothing is repeated");
        // The camera window is still managed exactly as before.
        let both = json!([clients[0].clone(), camera("0xc", 0, (10., 10.), (168., 168.))]);
        let plan2 = plan(&mut Placed::default(), &both, &monitors(), PID, 168., 4, false);
        assert!(plan2.commands.iter().any(|c| c.contains("0xc") && c.contains("resizewindowpixel exact 126 224")), "{:?}", plan2.commands);
    }
    #[test]
    fn the_main_window_is_the_fixed_427_by_44_toolbar_placed_once() {
        assert_eq!((TOOLBAR_W, TOOLBAR_H), (427., 44.));
        let clients = json!([client("0xa", MAIN_TITLE, PID, 0, (0., 0.), (580., 72.), false, false)]);
        let mut placed = Placed::default();
        let first = plan(&mut placed, &clients, &monitors(), PID, 168., 0, true);
        assert_eq!(first.commands, vec![
            "dispatch setfloating address:0xa",
            "dispatch pin address:0xa",
            "dispatch resizewindowpixel exact 427 44,address:0xa",
            // (1920 - 427) / 2 = 746.5 and 1080 - 68 from the shared default position.
            "dispatch movewindowpixel exact 746 1012,address:0xa",
        ]);
        assert!(first.layout.is_none(), "the toolbar never drives the bubble layout");
        // Same compositor state, the window still reports its old size: nothing is sent again.
        for _ in 0..3 {
            assert_eq!(plan(&mut placed, &clients, &monitors(), PID, 168., 0, true), Plan::default());
        }
        // A second monitor: logical size (pixels / scale) and the monitor origin are used.
        let clients = json!([client("0xb", MAIN_TITLE, PID, 1, (0., 0.), (580., 72.), true, true)]);
        let on_second = plan(&mut Placed::default(), &clients, &monitors(), PID, 168., 0, true);
        assert_eq!(on_second.commands, vec![
            "dispatch resizewindowpixel exact 427 44,address:0xb",
            "dispatch movewindowpixel exact 2666 1012,address:0xb",
        ]);
    }
    #[test]
    fn the_camera_window_follows_shape_and_size_with_one_dispatch_per_change() {
        let mut placed = Placed::default();
        let state = |shape: u8, base: f32, at: (f64, f64), size: (f64, f64), placed: &mut Placed| {
            plan(placed, &json!([camera("0x1", 0, at, size)]), &monitors(), PID, base, shape, true)
        };
        let first = state(0, 168., (5., 5.), (200., 200.), &mut placed);
        assert_eq!(first.commands, vec![
            "dispatch resizewindowpixel exact 168 168,address:0x1",
            "dispatch movewindowpixel exact 1728 852,address:0x1",
        ]);
        assert!(first.layout.is_none(), "no read-back on the pass that moved it");
        // Applied and unchanged, over and over: silence.
        for _ in 0..4 {
            assert!(state(0, 168., (1728., 852.), (168., 168.), &mut placed).commands.is_empty());
        }
        // Wide at base 168 is 224 x 126; one resize, then silence again.
        let wide = state(3, 168., (1728., 852.), (168., 168.), &mut placed);
        assert_eq!(wide.commands, vec![
            "dispatch resizewindowpixel exact 224 126,address:0x1",
            "dispatch movewindowpixel exact 1696 852,address:0x1",
        ]);
        assert!(state(3, 168., (1696., 852.), (224., 126.), &mut placed).commands.is_empty());
        // Size L wide is 299 x 168; small stays inside its monitor; square shapes use the base alone.
        assert_eq!(state(3, 224., (1696., 852.), (224., 126.), &mut placed).commands[0], "dispatch resizewindowpixel exact 299 168,address:0x1");
        assert_eq!(state(0, 120., (1696., 852.), (299., 168.), &mut placed).commands[0], "dispatch resizewindowpixel exact 120 120,address:0x1");
        assert_eq!(state(2, 120., (1696., 852.), (120., 120.), &mut placed).commands.len(), 0, "legacy square at base 120 is 120 x 120, unchanged");
        assert_eq!(state(1, 224., (1696., 852.), (120., 120.), &mut placed).commands[0], "dispatch resizewindowpixel exact 224 224,address:0x1");
        // Only an actual change re-dispatches: the same target again is silent even if the compositor ignored it.
        assert!(state(1, 224., (1696., 852.), (120., 120.), &mut placed).commands.is_empty());
    }
    #[test]
    fn layout_is_read_from_the_window_with_origin_scale_and_the_wide_base_conversion() {
        // Bubble on the scaled second monitor at x = 1920 + 100: origin is subtracted, units are logical.
        let mut placed = Placed::default();
        let place_it = |placed: &mut Placed, shape: u8, base: f32, size: (f64, f64)| {
            let clients = json!([camera("0x1", 1, (2020., 540.), size)]);
            plan(placed, &clients, &monitors(), PID, base, shape, true)
        };
        place_it(&mut placed, 0, 168., (168., 168.)); // first sight: placement only
        let update = place_it(&mut placed, 0, 168., (168., 168.)).layout.unwrap();
        assert!((update.x - 100. / 1920.).abs() < 1e-6 && (update.y - 0.5).abs() < 1e-6, "{update:?}");
        assert!((update.size.unwrap() - 168. / 1920.).abs() < 1e-6);
        // Dividing by a zero origin would give 2020 / 1920 > 1, the old failure.
        assert!(2020. / 1920. > 1.);
        // Wide: the window is 224 wide for base 168, so Layout.size is 168 / width, not 224 / width.
        place_it(&mut placed, 3, 168., (168., 168.)); // resize dispatched
        let wide = place_it(&mut placed, 3, 168., (224., 126.)).layout.unwrap();
        assert!((wide.size.unwrap() - 168. / 1920.).abs() < 1e-6, "{wide:?}");
        assert!((wide.size.unwrap() - 224. / 1920.).abs() > 0.01, "the old width/monitor value would enlarge the composite");
        // Right after a shape change the window still has the old size: position follows, size is left alone.
        place_it(&mut placed, 0, 120., (224., 126.));
        let stale = place_it(&mut placed, 0, 120., (224., 126.)).layout.unwrap();
        assert_eq!(stale.size, None);
        assert!((stale.x - 100. / 1920.).abs() < 1e-6);
        let applied = place_it(&mut placed, 0, 120., (120., 120.)).layout.unwrap();
        assert!((applied.size.unwrap() - 120. / 1920.).abs() < 1e-6);
    }
    #[test]
    fn an_overhanging_window_is_pulled_back_onto_its_monitor_after_a_resize() {
        let mut placed = Placed::default();
        let at = |placed: &mut Placed, shape: u8, base: f32, at: (f64, f64), size: (f64, f64)| {
            plan(placed, &json!([camera("0x1", 0, at, size)]), &monitors(), PID, base, shape, true)
        };
        at(&mut placed, 0, 120., (0., 0.), (120., 120.));
        // Dragged against the right edge, then switched to wide L (299 wide): it would hang off the monitor.
        let grown = at(&mut placed, 3, 224., (1790., 700.), (120., 120.));
        assert_eq!(grown.commands, vec![
            "dispatch resizewindowpixel exact 299 168,address:0x1",
            "dispatch movewindowpixel exact 1621 700,address:0x1",
        ]);
        assert!(grown.layout.is_none());
        // Inside the monitor already: no move.
        let fits = at(&mut placed, 0, 120., (500., 500.), (299., 168.));
        assert_eq!(fits.commands, vec!["dispatch resizewindowpixel exact 120 120,address:0x1"]);
    }
    #[test]
    fn unknown_monitor_data_means_no_sizing_no_placing_and_no_layout() {
        // A monitor missing width is skipped rather than defaulted to 1920 x 1080.
        let partial = parse_monitors(&json!([{"id":0,"x":0,"y":0,"scale":1.0}, {"id":1,"x":0,"y":0,"width":1920,"height":1080}, {"id":2,"x":0,"y":0,"width":0,"height":1080,"scale":1.0}]));
        assert!(partial.is_empty());
        let clients = json!([
            client("0xa", MAIN_TITLE, PID, 0, (0., 0.), (580., 72.), false, true),
            camera("0xb", 9, (300., 300.), (168., 168.)),
        ]);
        let mut placed = Placed::default();
        let first = plan(&mut placed, &clients, &partial, PID, 168., 0, true);
        assert_eq!(first.commands, vec!["dispatch setfloating address:0xa"], "only the flags, once");
        assert!(first.layout.is_none());
        // A client on a monitor that is not in the list is equally unknown.
        assert!(plan(&mut placed, &clients, &monitors(), PID, 168., 0, true).layout.is_none());
        // Not placed (it has no monitor), and nothing is sent for it.
        assert!(plan(&mut Placed::default(), &json!([camera("0xb", 9, (1., 1.), (168., 168.))]), &monitors(), PID, 168., 0, true).commands.is_empty());
        assert!(placed.placed.iter().all(|a| a != "0xb"));
        // Garbage in, nothing out.
        assert_eq!(plan(&mut Placed::default(), &json!("not a list"), &monitors(), PID, 168., 0, true), Plan::default());
        assert!(own_clients(&json!({"clients":[]}), PID).is_empty());
    }
}
