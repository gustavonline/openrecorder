#[cfg(feature = "desktop-ui")]
mod event_log;
#[cfg(feature = "desktop-ui")]
mod devices;
mod camera_frame;
#[cfg(feature = "desktop-ui")]
mod audio;
#[cfg(feature = "desktop-ui")]
mod capability;
#[cfg(feature = "desktop-ui")]
mod connection;
#[cfg(feature = "desktop-ui")]
mod controls;
#[cfg(feature = "desktop-ui")]
mod desktop;
#[cfg(all(feature = "desktop-ui", target_os = "linux"))]
mod hyprland;
#[cfg(feature = "desktop-ui")]
mod library_host;
#[cfg(feature = "desktop-ui")]
mod installation;
#[cfg(feature = "desktop-ui")]
mod material;
#[cfg(feature = "desktop-ui")]
mod permissions;
mod pipeline;
#[cfg(feature = "desktop-ui")]
mod shares;
#[cfg(feature = "desktop-ui")]
mod tray;
#[cfg(all(test, feature = "desktop-ui"))]
mod throughput_bench;
static STOP_REQUEST: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "native-capture")]
mod capture;
use serde::Serialize;
use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};
#[derive(Serialize)]
struct Report {
    mode: &'static str,
    frames: u64,
    seconds: f64,
    width: usize,
    height: usize,
    outputs: Vec<String>,
    hardware: bool,
}
fn codec() -> &'static str {
    use std::sync::OnceLock;
    static CODEC: OnceLock<&'static str> = OnceLock::new();
    CODEC.get_or_init(|| {
        let explicit = env::var("OPENRECORDER_CODEC").unwrap_or_default();
        let candidate = if explicit == "libx264" {
            return "libx264";
        } else if cfg!(target_os = "macos") {
            "h264_videotoolbox"
        } else if cfg!(target_os = "linux") && explicit != "h264_nvenc" {
            "h264_vaapi"
        } else if explicit == "h264_nvenc" {
            "h264_nvenc"
        } else if cfg!(target_os = "windows") || explicit == "h264_mf" {
            "h264_mf"
        } else {
            return "libx264";
        };
        let exe = env::var("OPENRECORDER_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let mut cmd = Command::new(exe);
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=s=640x360:r=30",
            "-frames:v",
            "1",
            "-c:v",
            candidate,
        ]);
        if candidate == "h264_vaapi" {
            cmd.args([
                "-vaapi_device",
                &env::var("OPENRECORDER_GPU_DEVICE")
                    .unwrap_or_else(|_| "/dev/dri/renderD128".into()),
                "-vf",
                "format=nv12,hwupload",
            ]);
        }
        if candidate == "h264_mf" {
            cmd.args(["-hw_encoding", "1"]);
        }
        if candidate == "h264_videotoolbox" {
            cmd.args(["-allow_sw", "0"]);
        }
        let Ok(mut child) = cmd
            .args(["-f", "null", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return "libx264";
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return if status.success() {
                        candidate
                    } else {
                        "libx264"
                    }
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return "libx264";
                }
            }
        }
    })
}
fn main() {
    if let Err(e) = run() {
        eprintln!("openrecorder-core: {e}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if let Some(i) = args.iter().position(|s| s == "--config") {
        let path = args.get(i + 1).ok_or("Missing connection file")?;
        let c: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        let home = c["home"].as_str().filter(|p| std::path::Path::new(p).is_absolute()).ok_or("Invalid library home")?;
        env::set_var("OPENRECORDER_HOME", home);
    }
    #[cfg(feature = "desktop-ui")]
    if args.get(1).map(String::as_str) == Some("--doctor") {
        return installation::doctor().map_err(Into::into);
    }
    #[cfg(feature = "desktop-ui")]
    if args.get(1).map(String::as_str) == Some("--stdio") {
        return library_host::stdio().map_err(Into::into);
    }
    #[cfg(feature = "desktop-ui")]
    if args.get(1).map(String::as_str) == Some("--library-host") {
        let config = library_host::start()?;
        if config["pid"].as_u64() != Some(std::process::id() as u64) { return Ok(()); }
        while !library_host::should_quit() {
            thread::sleep(Duration::from_millis(200));
            library_host::maintain();
        }
        library_host::shutdown();
        return Ok(());
    }
    #[cfg(feature = "desktop-ui")]
    if desktop_entry(&args) {
        return desktop::start();
    }
    #[cfg(feature = "native-capture")]
    match args.get(1).map(String::as_str) {
        Some("--diagnose-capture") => return capture::diagnose(),
        Some("--screen") => return capture::record(&args),
        _ => {}
    }
    synthetic(&args, None)
}
/// The one normal way into the desktop app: no arguments (a double-click), or `--desktop` (what the library host
/// passes: `--desktop --config <file>`). `--toolbar` is the same entry under its old name, accepted so an app or
/// descriptor written by an earlier build still starts; it does **not** bring the retired toolbar back.
/// `--dev-toolbar` is the one development switch that does (see `desktop::toolbar_requested`).
#[cfg_attr(not(feature = "desktop-ui"), allow(dead_code))]
fn desktop_entry(args: &[String]) -> bool {
    args.len() == 1 || matches!(args.get(1).map(String::as_str), Some("--desktop" | "--toolbar" | "--dev-toolbar"))
}
fn synthetic(
    args: &[String],
    preview: Option<std::sync::Arc<std::sync::Mutex<crate::camera_frame::Frame>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    if args.get(1).map(String::as_str) != Some("--synthetic") {
        return Err("Native media-core qualification only. Use --synthetic OUTPUT_DIR SECONDS [WIDTH HEIGHT]. Use --desktop for the desktop recorder.".into());
    }
    #[cfg(feature = "desktop-ui")]
    let _activity = permissions::RecordingActivity::begin();
    let directory = PathBuf::from(args.get(2).ok_or("Output directory required")?);
    let seconds: f64 = args.get(3).ok_or("Seconds required")?.parse()?;
    if !(1.0..=1800.0).contains(&seconds) {
        return Err("Seconds must be between 1 and 1800".into());
    }
    let w: usize = args.get(4).map(|v| v.parse()).transpose()?.unwrap_or(1920);
    let h: usize = args.get(5).map(|v| v.parse()).transpose()?.unwrap_or(1080);
    if w < 2 || h < 2 || w > 3840 || h > 2160 || w % 2 != 0 || h % 2 != 0 {
        return Err("Invalid dimensions".into());
    }
    fs::create_dir_all(&directory)?;
    let ffmpeg = env::var("OPENRECORDER_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let (cw, ch) = (1280usize.min(w), 720usize.min(h));
    #[cfg(feature = "desktop-ui")]
    let tone = audio::TestTone::new();
    let mut encoder = pipeline::Pipeline::start_with_audio(&ffmpeg, &directory, w, h, cw, ch, {
        #[cfg(feature = "desktop-ui")]
        {
            Some(tone.shared.clone())
        }
        #[cfg(not(feature = "desktop-ui"))]
        {
            None
        }
    })?;
    let mut screen = vec![0u8; w * h * 4];
    let mut camera = vec![0u8; cw * ch * 4];
    let start = Instant::now();
    let mut frames = 0u64;
    let period = Duration::from_secs_f64(1. / 30.);
    let result = (|| -> std::io::Result<()> {
        while start.elapsed().as_secs_f64() < seconds && !STOP_REQUEST.load(Ordering::Relaxed) {
            for (y, row) in screen.chunks_exact_mut(w * 4).enumerate() {
                for (x, p) in row.chunks_exact_mut(4).enumerate() {
                    p.copy_from_slice(&[
                        ((x + frames as usize) % 256) as u8,
                        (y % 256) as u8,
                        54,
                        255,
                    ]);
                }
            }
            for (y, row) in camera.chunks_exact_mut(cw * 4).enumerate() {
                for (x, p) in row.chunks_exact_mut(4).enumerate() {
                    p.copy_from_slice(&[
                        64,
                        ((x + frames as usize * 3) % 256) as u8,
                        (y % 256) as u8,
                        255,
                    ]);
                }
            }
            if let Some(slot) = &preview {
                if let Ok(mut value) = slot.lock() {
                    value.width=cw;value.height=ch;value.bgra.copy_from_slice(&camera);value.sequence+=1;
                }
            }
            encoder.set_screen(&screen)?;
            encoder.set_camera(&camera)?;
            encoder.write()?;
            frames += 1;
            if start.elapsed().as_secs_f64() - frames as f64 / 30. > 0.75 {
                return Err(std::io::Error::other(
                    "Qualification cannot keep up with real time",
                ));
            }
            thread::sleep(
                (start + period.mul_f64(frames as f64)).saturating_duration_since(Instant::now()),
            );
        }
        Ok(())
    })();
    encoder.finish()?;
    result?;
    let report = Report {
        mode: "synthetic qualification; not hardware capture",
        frames,
        seconds: start.elapsed().as_secs_f64(),
        width: w,
        height: h,
        outputs: vec![
            "screen.mp4".into(),
            "camera.mp4".into(),
            "recording.mp4".into(),
        ],
        hardware: encoder.encoder_summary()["allHardware"].as_bool().unwrap_or(false),
    };
    fs::write(
        directory.join("qualification.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

#[cfg(test)]
mod entry_tests {
    use super::desktop_entry;
    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("OpenRecorder").chain(list.iter().copied()).map(String::from).collect()
    }
    #[test]
    fn one_normal_entry_and_no_other_argument_opens_the_desktop() {
        assert!(desktop_entry(&args(&[])), "a double-click");
        assert!(desktop_entry(&args(&["--desktop", "--config", "/c"])), "what the library host launches");
        assert!(desktop_entry(&args(&["--toolbar", "--config", "/c"])), "an earlier build's launcher still starts the app");
        assert!(desktop_entry(&args(&["--dev-toolbar"])));
        for other in [&["--library-host"][..], &["--stdio"], &["--doctor"], &["--synthetic", "/o", "5"], &["--screen", "/o", "5"], &["--idle-ui", "5"]] {
            assert!(!desktop_entry(&args(other)), "{other:?}");
        }
    }
}
