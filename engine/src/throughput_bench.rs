//! Input-free reproduction of the owner's failing workload (preview9 evidence): a 2560x1440 display at 60 fps, a
//! 900x720 camera, originals and audio, encoded by the real `Pipeline` and the real helper, driven by the same fixed
//! clock sampler as `capture::record_session`. No screen, camera or microphone is opened: the "display" and "camera"
//! are threads that publish synthetic frames at configurable cadences, the audio is the app's own test tone.
//!
//! Ignored by default (needs the helper and takes up to 30 s):
//!   OPENRECORDER_FFMPEG=/path/to/ffmpeg P9_SECONDS=15 cargo test --release --features desktop-ui -- --ignored --nocapture p9_
//! It prints one JSON line (and appends it to $P9_OUT when set) with output cadence, source cadence, ticks the
//! sampler skipped, per-stage times, the helper's CPU and peak memory, and the codec used. P9_KEEP=1 keeps the files
//! (their path is printed) so their timestamps can be inspected.
use crate::{audio::TestTone, camera_frame::Frame, pipeline::Pipeline};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const W: usize = 2560;
const H: usize = 1440;
const CW: usize = 900;
const CH: usize = 720;

fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// `ps` reading of the helper: (cumulative CPU seconds, resident MB). Best effort; None where `ps` is unavailable.
fn helper_usage(pid: u32) -> Option<(f64, f64)> {
    let out = std::process::Command::new("ps").args(["-o", "cputime=,rss=", "-p", &pid.to_string()]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut parts = text.split_whitespace();
    let cpu = parts.next()?;
    let rss: f64 = parts.next()?.parse().ok()?;
    let seconds = cpu.split(':').try_fold(0.0, |acc, p| p.parse::<f64>().ok().map(|v| acc * 60.0 + v))?;
    Some((seconds, rss / 1024.0))
}

pub struct Report(pub serde_json::Value);

/// One run. `drive` receives the encoder and the two shared sources and submits one frame (the caller decides how).
pub fn run(label: &str, mut drive: impl FnMut(&mut Pipeline, &Mutex<Arc<Vec<u8>>>, &Mutex<Frame>) -> std::io::Result<()>) -> Report {
    let helper = std::env::var("OPENRECORDER_FFMPEG").expect("Set OPENRECORDER_FFMPEG to the bundled helper");
    let seconds = env_f64("P9_SECONDS", 15.).clamp(2., 30.);
    let fps = env_f64("P9_FPS", 60.).clamp(24., 120.) as u32;
    let source_fps = env_f64("P9_SOURCE_FPS", 57.);
    let camera_fps = env_f64("P9_CAMERA_FPS", 24.);
    let dir = std::env::temp_dir().join(format!("openrecorder-p9-bench-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let tone = TestTone::new();
    let mut encoder = Pipeline::start_config(&helper, &dir, W, H, CW, CH, Some(tone.shared.clone()), true, fps).expect("encoder starts");

    // The "display": publishes a fresh frame at the source cadence, allocating each one like the capture adapter does.
    let base: Vec<u8> = (0..W * H * 4).map(|i| ((i / 4 % W) as u8).wrapping_add((i / (4 * W) / 8) as u8)).collect();
    let slot = Arc::new(Mutex::new(Arc::new(base.clone())));
    let camera = Arc::new(Mutex::new(Frame { width: CW, height: CH, bgra: vec![90; CW * CH * 4], sequence: 0 }));
    let stop = Arc::new(AtomicBool::new(false));
    let source_frames = Arc::new(AtomicU64::new(0));
    let mut threads = Vec::new();
    {
        let (slot, stop, count) = (slot.clone(), stop.clone(), source_frames.clone());
        threads.push(thread::spawn(move || {
            let period = Duration::from_secs_f64(1. / source_fps);
            let mut next = Instant::now();
            while !stop.load(Ordering::Relaxed) {
                let mut fresh = base.clone();
                let n = count.fetch_add(1, Ordering::Relaxed);
                fresh[(n as usize * 4) % (W * 4)] ^= 0x55; // distinct content per frame
                *slot.lock().unwrap() = Arc::new(fresh);
                next += period;
                thread::sleep(next.saturating_duration_since(Instant::now()));
            }
        }));
    }
    {
        let (camera, stop) = (camera.clone(), stop.clone());
        threads.push(thread::spawn(move || {
            let period = Duration::from_secs_f64(1. / camera_fps);
            let mut next = Instant::now();
            let mut n = 0u8;
            while !stop.load(Ordering::Relaxed) {
                n = n.wrapping_add(1);
                {
                    let mut f = camera.lock().unwrap();
                    f.bgra.iter_mut().step_by(4096).for_each(|b| *b = n);
                    f.sequence += 1;
                }
                next += period;
                thread::sleep(next.saturating_duration_since(Instant::now()));
            }
        }));
    }
    thread::sleep(Duration::from_millis(300)); // sources are live before the sampler starts, as in a real take

    // The fixed-grid sampler, identical in structure to capture::record_session: a tick missed while the encoder was
    // busy is skipped.
    let epoch = Instant::now();
    let period = Duration::from_secs_f64(1. / f64::from(fps));
    let (mut count, mut tick) = (0u32, 0u64);
    let (mut drive_total, mut drive_max) = (Duration::ZERO, Duration::ZERO);
    let mut error = None;
    while epoch.elapsed().as_secs_f64() < seconds {
        let began = Instant::now();
        if let Err(e) = drive(&mut encoder, &slot, &camera) {
            error = Some(e.to_string());
            break;
        }
        let took = began.elapsed();
        drive_total += took;
        drive_max = drive_max.max(took);
        count += 1;
        tick = crate::capture::next_tick(tick, epoch.elapsed(), period);
        thread::sleep((epoch + period.mul_f64(tick as f64)).saturating_duration_since(Instant::now()));
    }
    let loop_seconds = epoch.elapsed().as_secs_f64();
    let usage = encoder.child.lock().ok().and_then(|c| helper_usage(c.id()));
    stop.store(true, Ordering::Relaxed);
    let finish_began = Instant::now();
    let finished = encoder.finish();
    let finish_ms = finish_began.elapsed().as_secs_f64() * 1e3;
    for t in threads {
        let _ = t.join();
    }
    drop(tone);
    let sizes: serde_json::Value = ["screen", "camera", "recording"].iter().map(|n| (n.to_string(), serde_json::json!(std::fs::metadata(dir.join(format!("{n}.mp4"))).map(|m| m.len()).unwrap_or(0)))).collect::<serde_json::Map<_, _>>().into();
    let timing: serde_json::Value = std::fs::read(dir.join("encoder-timing.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let report = serde_json::json!({
        "label": label,
        "workload": {"screen": format!("{W}x{H}"), "camera": format!("{CW}x{CH}"), "originals": true, "audio": "synthetic tone", "requestedFps": fps,
                     "codec": timing["codec"], "sourceFps": source_fps, "cameraFps": camera_fps, "physicalCapture": false},
        "loopSeconds": loop_seconds, "outputFrames": count, "outputFps": f64::from(count) / loop_seconds,
        "sourceFramesPublished": source_frames.load(Ordering::Relaxed),
        "skippedTicks": tick - u64::from(count),
        "submitMsMean": drive_total.as_secs_f64() * 1e3 / f64::from(count.max(1)), "submitMsMax": drive_max.as_secs_f64() * 1e3,
        "helperCpuSeconds": usage.map(|u| u.0), "helperCpuCores": usage.map(|u| u.0 / loop_seconds), "helperRssMb": usage.map(|u| u.1),
        "finishMs": finish_ms, "finishOk": finished.is_ok(), "error": error,
        "outputBytes": sizes, "encoderTiming": timing,
    });
    println!("P9-BENCH {report}");
    if let Ok(path) = std::env::var("P9_OUT") {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{report}");
        }
    }
    if std::env::var("P9_KEEP").is_ok_and(|v| v == "1") {
        println!("P9-BENCH kept {}", dir.display());
    } else {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Report(report)
}

#[test]
#[ignore = "needs the bundled helper; up to 30 s"]
fn p9_actual_workload_pipelined() {
    // The recorder's own hand-off: the camera is held only while its band and the bubble are copied, never while
    // waiting for the encoder.
    run("pipelined", |encoder, slot, camera| {
        let screen = slot.lock().unwrap().clone();
        encoder.submit_shared_camera(&screen, camera, |_| Ok(())).map_err(|e| std::io::Error::other(e.to_string()))
    });
}
