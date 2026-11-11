use cocoa::appkit::NSScreen;
use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use core_graphics_helmer_fork::display::{CGDirectDisplayID, CGDisplay, CGMainDisplayID};
use core_graphics_helmer_fork::window::CGWindowID;
use objc::{msg_send, sel, sel_impl};
use screencapturekit::sc_shareable_content::SCShareableContent;

use super::{Display, Target};

fn get_display_name(display_id: CGDirectDisplayID) -> String {
    unsafe {
        // Get all screens
        let screens: id = NSScreen::screens(nil);
        let count: u64 = msg_send![screens, count];

        for i in 0..count {
            let screen: id = msg_send![screens, objectAtIndex: i];
            let device_description: id = msg_send![screen, deviceDescription];
            let display_id_number: id = msg_send![device_description, objectForKey: NSString::alloc(nil).init_str("NSScreenNumber")];
            let display_id_number: u32 = msg_send![display_id_number, unsignedIntValue];

            if display_id_number == display_id {
                let localized_name: id = msg_send![screen, localizedName];
                let name: *const i8 = msg_send![localized_name, UTF8String];
                return std::ffi::CStr::from_ptr(name)
                    .to_string_lossy()
                    .into_owned();
            }
        }

        format!("Unknown Display {}", display_id)
    }
}

pub fn get_all_targets() -> Vec<Target> {
    let mut targets: Vec<Target> = Vec::new();

    let content = SCShareableContent::current();

    // Add displays to targets
    for display in content.displays {
        let id: CGDirectDisplayID = display.display_id;
        let raw_handle = CGDisplay::new(id);
        let title = get_display_name(id);

        let target = Target::Display(super::Display {
            id,
            title,
            raw_handle,
        });

        targets.push(target);
    }

    // Add windows to targets
    for window in content.windows {
        if window.title.is_some() {
            let id = window.window_id;
            // ScreenCaptureKit reports the owning application: ownership comes from the process, never the title.
            let owner_pid = window
                .owning_application
                .as_ref()
                .map(|app| app.process_id)
                .filter(|pid| *pid > 0)
                .map(|pid| pid as u32);
            let title = window.title.expect("Window title not found");
            let raw_handle: CGWindowID = id;

            let target = Target::Window(super::Window {
                id,
                title,
                owner_pid,
                raw_handle,
            });
            targets.push(target);
        }
    }

    targets
}

pub fn get_main_display() -> Display {
    let id = unsafe { CGMainDisplayID() };
    let title = get_display_name(id);

    Display {
        id,
        title,
        raw_handle: CGDisplay::new(id),
    }
}

pub fn get_scale_factor(target: &Target) -> f64 {
    match target {
        // Any window, the recorder's own or another application's: from ScreenCaptureKit's own metadata.
        // 1.0 is only returned for a window that no longer exists; capture creation rejects that case.
        Target::Window(window) => window_metrics(window.raw_handle).map_or(1.0, |m| m.scale),
        Target::Display(display) => {
            let mode = display.raw_handle.display_mode().unwrap();
            (mode.pixel_width() / mode.width()) as f64
        }
    }
}

/// Window size in points (what the capture configuration's source rectangle uses). (0, 0) for a window
/// that no longer exists.
pub fn get_target_dimensions(target: &Target) -> (u64, u64) {
    match target {
        Target::Window(window) => window_metrics(window.raw_handle)
            .map_or((0, 0), |m| (m.width.round() as u64, m.height.round() as u64)),
        Target::Display(display) => {
            let mode = display.raw_handle.display_mode().unwrap();
            (mode.width(), mode.height())
        }
    }
}

/// Axis-aligned frame in global points (origin top-left of the main display, y down), as ScreenCaptureKit reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Frame {
    fn overlap_area(&self, other: &Frame) -> f64 {
        let w = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let h = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        if w > 0. && h > 0. {
            w * h
        } else {
            0.
        }
    }
}
/// A window's size in points and the backing scale (pixels per point) of the display holding most of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMetrics {
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}
/// Pure geometry: `displays` are (frame, scale). A window with no finite positive size, no overlapping
/// display, or a display without a usable scale yields None: nothing is defaulted or guessed.
pub fn metrics_from(window: Frame, displays: &[(Frame, f64)]) -> Option<WindowMetrics> {
    let finite = [window.x, window.y, window.width, window.height].iter().all(|v| v.is_finite());
    if !finite || window.width <= 0. || window.height <= 0. {
        return None;
    }
    let (_, scale) = displays
        .iter()
        .map(|(frame, scale)| (window.overlap_area(frame), *scale))
        .filter(|(area, scale)| *area > 0. && scale.is_finite() && *scale > 0.)
        .max_by(|a, b| a.0.total_cmp(&b.0))?;
    Some(WindowMetrics { width: window.width, height: window.height, scale })
}
/// Metrics of one window by id, from ScreenCaptureKit's shareable content (a window of any application).
/// Reads geometry only: no titles beyond what the target list already holds, no capture.
pub fn window_metrics(window_id: u32) -> Option<WindowMetrics> {
    use screencapturekit_sys::shareable_content::UnsafeSCShareableContent;
    let content = UnsafeSCShareableContent::get().ok()?;
    let window = content.windows().into_iter().find(|w| w.get_window_id() == window_id)?;
    let frame = window.get_frame();
    let displays: Vec<(Frame, f64)> = content
        .displays()
        .into_iter()
        .filter_map(|display| {
            let f = display.get_frame();
            let mode = CGDisplay::new(display.get_display_id()).display_mode()?;
            let points = mode.width() as f64;
            (points > 0.).then(|| {
                (
                    Frame { x: f.origin.x, y: f.origin.y, width: f.size.width, height: f.size.height },
                    mode.pixel_width() as f64 / points,
                )
            })
        })
        .collect();
    metrics_from(
        Frame { x: frame.origin.x, y: frame.origin.y, width: frame.size.width, height: frame.size.height },
        &displays,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn f(x: f64, y: f64, w: f64, h: f64) -> Frame {
        Frame { x, y, width: w, height: h }
    }
    #[test]
    fn a_foreign_window_takes_its_size_and_the_scale_of_the_display_holding_most_of_it() {
        let main = (f(0., 0., 1512., 982.), 2.0);
        let external = (f(1512., -200., 2560., 1440.), 1.0);
        // Another application's 800 x 600 pt window, mostly on the external display.
        let m = metrics_from(f(1500., 100., 800., 600.), &[main, external]).unwrap();
        assert_eq!(m, WindowMetrics { width: 800., height: 600., scale: 1.0 });
        // Same size mostly on the Retina display: scale 2 and pixels = points x scale.
        let m = metrics_from(f(100., 100., 800., 600.), &[external, main]).unwrap();
        assert_eq!((m.width * m.scale, m.height * m.scale), (1600., 1200.));
    }
    #[test]
    fn missing_closed_or_unmeasurable_windows_are_none_never_a_default() {
        let main = (f(0., 0., 1512., 982.), 2.0);
        assert_eq!(metrics_from(f(5000., 5000., 400., 300.), &[main]), None, "on no display");
        assert_eq!(metrics_from(f(10., 10., 0., 300.), &[main]), None);
        assert_eq!(metrics_from(f(10., 10., -5., 300.), &[main]), None);
        assert_eq!(metrics_from(f(f64::NAN, 10., 100., 100.), &[main]), None);
        assert_eq!(metrics_from(f(10., 10., f64::INFINITY, 100.), &[main]), None);
        assert_eq!(metrics_from(f(10., 10., 100., 100.), &[]), None);
        assert_eq!(metrics_from(f(10., 10., 100., 100.), &[(f(0., 0., 1512., 982.), 0.)]), None, "unusable scale");
        assert_eq!(metrics_from(f(10., 10., 100., 100.), &[(f(0., 0., 1512., 982.), f64::NAN)]), None);
    }
    #[test]
    fn a_window_id_nothing_owns_has_no_metrics_and_zero_dimensions() {
        // Passive ScreenCaptureKit enumeration by id: nothing matches, nothing is invented.
        assert_eq!(window_metrics(u32::MAX), None);
    }
}
