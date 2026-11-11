use scap::{
    capturer::{Capturer, Options, Resolution},
    frame::Frame,
};
use std::{
    env,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct RecordOptions {
    pub selection: Option<(bool, u32)>,
    pub fps: u32,
    pub quality: u32,
    pub originals: bool,
    pub microphone: Option<super::pipeline::AudioHandle>,
    pub source_error: Option<Arc<Mutex<Option<String>>>>,
    pub progress: Arc<Progress>,
}
impl Default for RecordOptions {
    fn default() -> Self {
        Self {
            selection: None,
            fps: 30,
            quality: 1920,
            originals: true,
            microphone: None,
            source_error: None,
            progress: Arc::new(Progress::default()),
        }
    }
}
#[derive(Default)]
pub struct Progress {
    pub phase: std::sync::atomic::AtomicU8,
    pub frames: std::sync::atomic::AtomicU64,
    pub fps: std::sync::atomic::AtomicU32,
    /// Frames the screen source delivered and the checker accepted. Source cadence, not output.
    pub source_frames: std::sync::atomic::AtomicU64,
    /// Frames the checker refused (tolerated until a bounded run, see `SourceStats::reject`).
    pub rejected_frames: std::sync::atomic::AtomicU64,
    /// Measured output cadence in milli-frames per second: frames written / loop seconds.
    pub delivered_milli_fps: std::sync::atomic::AtomicU32,
    /// The finished take's capture.json content, kept for the app to learn from. Nothing else reads files back.
    pub summary: Mutex<Option<serde_json::Value>>,
}
struct Pixels {
    width: usize,
    height: usize,
    /// Shared so the sampler hands the very buffer the capture produced to the encoder: no copy, and the capture
    /// thread can publish the next picture without waiting for the previous one to be written.
    data: Arc<Vec<u8>>,
}
/// Why a source frame was refused. Every variant carries the values that were seen, so a failed take
/// says what arrived instead of only that something was unexpected.
#[derive(Clone, Debug, PartialEq)]
pub enum FrameReject {
    /// The adapter delivered a picture with no pixels (zero width or height): a status tick, not a picture. Counted
    /// apart, never part of a refusal run, and not evidence of anything wrong with the source.
    Empty,
    Format,
    /// Width or height outside what this recorder accepts (a sanity bound on memory, not on geometry).
    Bounds { w: i32, h: i32 },
    /// The bounds were fine but scaling would leave less than two pixels on an axis.
    TooSmall { w: i32, h: i32 },
    /// The buffer is not exactly width x height x 4 bytes: a stride or padding difference.
    Length { w: i32, h: i32, len: usize },
}
/// Prefix kept stable so Home can map it to its plain-language copy.
const REJECT_PREFIX: &str = "Unexpected native pixel dimensions or stride";
impl FrameReject {
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "The capture source returned an empty picture".into(),
            Self::Format => "The capture source returned an unsupported pixel format".into(),
            Self::Bounds { w, h } => format!("{REJECT_PREFIX}: {w}x{h} is outside the supported size"),
            Self::TooSmall { .. } => "Capture area is too small".into(),
            Self::Length { w, h, len } => format!(
                "{REJECT_PREFIX}: {len} bytes for {w}x{h} (expected {})",
                (*w).max(0) as usize * (*h).max(0) as usize * 4
            ),
        }
    }
    fn sample(&self) -> serde_json::Value {
        match self {
            Self::Empty => serde_json::json!({"reason":"empty"}),
            Self::Format => serde_json::json!({"reason":"format"}),
            Self::Bounds { w, h } => serde_json::json!({"reason":"bounds","width":w,"height":h}),
            Self::TooSmall { w, h } => serde_json::json!({"reason":"too-small","width":w,"height":h}),
            Self::Length { w, h, len } => serde_json::json!({"reason":"length","width":w,"height":h,"bytes":len,"expectedBytes":(*w).max(0) as usize * (*h).max(0) as usize * 4}),
        }
    }
}
// Resolution is a maximum long edge. Round both axes down to even values
// after applying one scale, retaining aspect ratio to pixel precision.
fn check_dimensions(w: i32, h: i32, maximum_long_edge: usize) -> Result<(usize, usize), FrameReject> {
    if w == 0 || h == 0 {
        return Err(FrameReject::Empty);
    }
    if w < 2 || h < 2 || w > 3840 || h > 3840 || w as usize * h as usize > 3840 * 2400 {
        return Err(FrameReject::Bounds { w, h });
    }
    let (sw, sh) = (w as usize, h as usize);
    let long_edge = sw.max(sh);
    let target = long_edge.min(maximum_long_edge);
    let width = (sw * target / long_edge) & !1;
    let height = (sh * target / long_edge) & !1;
    if width < 2 || height < 2 {
        return Err(FrameReject::TooSmall { w, h });
    }
    Ok((width, height))
}
#[cfg_attr(not(test), allow(dead_code))]
fn screen_dimensions(w: i32, h: i32, maximum_long_edge: usize) -> Result<(usize, usize), String> {
    check_dimensions(w, h, maximum_long_edge).map_err(|e| e.message())
}
/// Longest run of refused frames tolerated before the take fails. A run this long is a source that
/// is not producing usable pictures, not a single odd buffer.
const REJECT_RUN_FRAMES: u64 = 120;
const REJECT_RUN_TIME: Duration = Duration::from_secs(2);
/// Source-side accounting kept separately from output pacing: how many pictures the screen source
/// delivered, how often, and which ones were refused and why.
#[derive(Default)]
struct SourceStats {
    accepted: u64,
    rejected: u64,
    first: Option<Instant>,
    last: Option<Instant>,
    max_gap: Duration,
    run: u64,
    run_since: Option<Instant>,
    empty: u64,
    samples: Vec<serde_json::Value>,
    last_reject: Option<FrameReject>,
}
impl SourceStats {
    fn accept(&mut self, now: Instant) {
        if let Some(last) = self.last {
            self.max_gap = self.max_gap.max(now.saturating_duration_since(last));
        }
        self.first.get_or_insert(now);
        self.last = Some(now);
        self.accepted += 1;
        self.run = 0;
        self.run_since = None;
    }
    /// Records a refused frame. Returns true when the run is long enough that the take must fail. An empty
    /// picture is only counted: it neither extends nor ends a run, and never fails a take.
    fn reject(&mut self, now: Instant, why: &FrameReject) -> bool {
        if *why == FrameReject::Empty {
            self.empty += 1;
            return false;
        }
        self.rejected += 1;
        self.run += 1;
        let since = *self.run_since.get_or_insert(now);
        if self.samples.len() < 5 {
            let mut sample = why.sample();
            sample["acceptedBefore"] = self.accepted.into();
            self.samples.push(sample);
        }
        self.last_reject = Some(why.clone());
        self.run >= REJECT_RUN_FRAMES || now.saturating_duration_since(since) >= REJECT_RUN_TIME
    }
    fn json(&self) -> serde_json::Value {
        let seconds = match (self.first, self.last) {
            (Some(a), Some(b)) => b.saturating_duration_since(a).as_secs_f64(),
            _ => 0.,
        };
        serde_json::json!({
            "acceptedFrames": self.accepted,
            "rejectedFrames": self.rejected,
            "emptyFrames": self.empty,
            // Arrival rate of the source itself; says nothing about what was encoded.
            "arrivalFps": if seconds > 0. && self.accepted > 1 { serde_json::json!((self.accepted - 1) as f64 / seconds) } else { serde_json::Value::Null },
            "maxGapMs": self.max_gap.as_secs_f64() * 1000.,
            "rejectedSamples": self.samples,
        })
    }
}
fn pixels(frame: Frame, maximum_long_edge: usize) -> Result<Pixels, FrameReject> {
    let (w, h, mut data, kind) = match frame {
        Frame::BGRA(f) => (f.width, f.height, f.data, 0),
        Frame::BGRx(f) => (f.width, f.height, f.data, 1),
        Frame::RGBx(f) => (f.width, f.height, f.data, 2),
        Frame::XBGR(f) => (f.width, f.height, f.data, 3),
        _ => return Err(FrameReject::Format),
    };
    let (width, height) = check_dimensions(w, h, maximum_long_edge)?;
    if data.len() != w as usize * h as usize * 4 {
        return Err(FrameReject::Length { w, h, len: data.len() });
    }
    if kind != 0 {
        for p in data.chunks_exact_mut(4) {
            match kind {
                1 => p[3] = 255,
                2 => {
                    p.swap(0, 2);
                    p[3] = 255
                }
                3 => {
                    p[0] = p[1];
                    p[1] = p[2];
                    p[2] = p[3];
                    p[3] = 255
                }
                _ => {}
            }
        }
    }
    let sw = w as usize;
    let sh = h as usize;
    if width == sw && height == sh {
        return Ok(Pixels {
            width,
            height,
            data: Arc::new(data),
        });
    }
    let mut scaled = vec![0u8; width * height * 4];
    for y in 0..height {
        let sy = y * sh / height;
        for x in 0..width {
            let src = (sy * sw + x * sw / width) * 4;
            let dst = (y * width + x) * 4;
            scaled[dst..dst + 4].copy_from_slice(&data[src..src + 4]);
        }
    }
    Ok(Pixels {
        width,
        height,
        data: Arc::new(scaled),
    })
}
pub fn diagnose() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        serde_json::json!({"supported":scap::is_supported(),"permission":scap::has_permission(),"requested_permission":false})
    );
    Ok(())
}
pub fn record(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(args.get(2).ok_or("Output file required")?);
    if output.exists() {
        return Err("Output already exists".into());
    }
    let seconds: f64 = args.get(3).ok_or("Duration required")?.parse()?;
    if !(1.0..=1800.0).contains(&seconds) {
        return Err("Invalid duration".into());
    }
    let directory = output.with_extension("capture");
    record_session(
        directory.clone(),
        None,
        Some(Duration::from_secs_f64(seconds)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(Mutex::new(super::pipeline::Layout::default())),
        RecordOptions::default(),
    )?;
    std::fs::rename(directory.join("recording.mp4"), &output)?;
    println!(
        "{}",
        serde_json::json!({"capture":"physical screen","path":output,"camera":false,"clean_original_exclusion_qualified":false})
    );
    Ok(())
}
pub fn record_shared(
    directory: PathBuf,
    camera: Option<Arc<Mutex<crate::camera_frame::Frame>>>,
    layout: Arc<Mutex<super::pipeline::Layout>>,
    options: RecordOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    record_session(
        directory,
        camera,
        None,
        Arc::new(AtomicBool::new(false)),
        layout,
        options,
    )
}
// A live camera advances `CameraFrame.sequence` once per decoded frame, so a
// sequence that stops changing means the stream stopped, whatever the pixels
// show or whether any window is visible. Any change counts as activity, which
// includes a reset to a lower value from a restarted stream. The clock is a
// caller-supplied time since some fixed origin, so this stays a pure type.
const CAMERA_STALE_AFTER: Duration = Duration::from_millis(2500);
#[derive(Debug)]
struct CameraStopped;
impl std::fmt::Display for CameraStopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Camera stopped sending video for more than {} seconds. Recording stopped at that point.",
            CAMERA_STALE_AFTER.as_secs_f32()
        )
    }
}
impl std::error::Error for CameraStopped {}
struct CameraLiveness {
    sequence: Option<u64>,
    since: Duration,
}
impl CameraLiveness {
    // `sequence` is `None` when no camera is enabled; such a take never stalls.
    fn new(sequence: Option<u64>, now: Duration) -> Self {
        Self {
            sequence,
            since: now,
        }
    }
    fn check(&mut self, sequence: Option<u64>, now: Duration) -> Result<(), CameraStopped> {
        if sequence.is_none() {
            return Ok(());
        }
        if sequence != self.sequence {
            self.sequence = sequence;
            self.since = now;
            return Ok(());
        }
        if now.saturating_sub(self.since) > CAMERA_STALE_AFTER {
            return Err(CameraStopped);
        }
        Ok(())
    }
}
fn set_camera_frame(
    encoder: &mut super::pipeline::Pipeline,
    frame: &crate::camera_frame::Frame,
) -> std::io::Result<()> {
    if (frame.width, frame.height) != (encoder.camera_width, encoder.camera_height) {
        return Err(std::io::Error::other("Camera dimensions changed"));
    }
    encoder.set_camera(&frame.bgra)
}
// Always close encoder input, even when the recording loop failed. The caller
// keeps the primary source/loop error and reports finalization independently.
fn finalize_take(
    encoder: &mut super::pipeline::Pipeline,
    primary: Result<(), Box<dyn std::error::Error>>,
) -> (Result<(), Box<dyn std::error::Error>>, serde_json::Value) {
    let finalization = encoder.finish();
    let outcome = match &finalization {
        Ok(()) => serde_json::json!({"status":"completed","error":null}),
        Err(error) => serde_json::json!({"status":"failed","error":error.to_string()}),
    };
    (
        primary.and_then(|()| finalization.map_err(Into::into)),
        outcome,
    )
}
pub fn record_session(
    directory: PathBuf,
    camera: Option<Arc<Mutex<crate::camera_frame::Frame>>>,
    limit: Option<Duration>,
    cancel: Arc<AtomicBool>,
    layout: Arc<Mutex<super::pipeline::Layout>>,
    options: RecordOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "desktop-ui")]
    let _activity = super::permissions::RecordingActivity::begin();
    if ![24, 30, 60].contains(&options.fps) || ![1280, 1920, 2560].contains(&options.quality) {
        return Err("Unsupported recording quality or frame rate".into());
    }
    options.progress.phase.store(0, Ordering::Relaxed);
    options.progress.frames.store(0, Ordering::Relaxed);
    options.progress.fps.store(options.fps, Ordering::Relaxed);
    options.progress.source_frames.store(0, Ordering::Relaxed);
    options.progress.rejected_frames.store(0, Ordering::Relaxed);
    options.progress.delivered_milli_fps.store(0, Ordering::Relaxed);
    if let Ok(mut summary) = options.progress.summary.lock() {
        *summary = None;
    }
    if !scap::is_supported() {
        return Err("Native capture is not supported on this session".into());
    }
    if !scap::has_permission() {
        return Err(
            "Screen access is not granted; no permission was requested automatically".into(),
        );
    }
    std::fs::create_dir_all(&directory)?;
    let slot: Arc<Mutex<Result<Option<Pixels>, String>>> = Arc::new(Mutex::new(Ok(None)));
    let stats: Arc<Mutex<SourceStats>> = Arc::default();
    let producer_stats = stats.clone();
    let shared = slot.clone();
    let stop = cancel.clone();
    let producer_options = options.clone();
    let producer = thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<(), String> {
                let targets = scap::get_all_targets();
                let selected = producer_options.selection.and_then(|(window, id)| {
                    targets
                        .iter()
                        .find(|t| match t {
                            scap::Target::Window(w) => window && w.id == id,
                            scap::Target::Display(d) => !window && d.id == id,
                        })
                        .cloned()
                });
                if producer_options.selection.is_some() && selected.is_none() {
                    return Err("The selected screen or window is no longer available".into());
                }
                // This recorder's own windows (toolbar, bubble, pickers) are excluded by the process that owns
                // them, never by title: a foreign page named "OpenRecorder…" stays in the recording.
                let own_pid = std::process::id();
                let excluded = targets
                    .into_iter()
                    .filter(|target| matches!(target, scap::Target::Window(w) if window_ownership(w.owner_pid, own_pid) == Ownership::Own))
                    .collect::<Vec<_>>();
                let mut cap = Capturer::build(Options {
                    target: selected,
                    show_cursor: true,
                    fps: producer_options.fps,
                    output_type: scap::frame::FrameType::BGRAFrame,
                    output_resolution: match producer_options.quality {
                        1280 => Resolution::_720p,
                        2560 => Resolution::_1440p,
                        _ => Resolution::_1080p,
                    },
                    excluded_targets: Some(excluded),
                    ..Default::default()
                })
                .map_err(|e| e.to_string())?;
                cap.start_capture();
                let capture_result = (|| -> Result<(), String> {
                    while !stop.load(Ordering::Relaxed) {
                        match cap.get_next_frame_timeout(Duration::from_millis(250)) {
                            Ok(frame) => match pixels(frame, producer_options.quality as usize) {
                                Ok(frame) => {
                                    let accepted = {
                                        let mut held = producer_stats.lock().map_err(|_| "Capture statistics poisoned")?;
                                        held.accept(Instant::now());
                                        held.accepted
                                    };
                                    producer_options.progress.source_frames.store(accepted, Ordering::Relaxed);
                                    *shared.lock().map_err(|_| "Capture slot poisoned")? =
                                        Ok(Some(frame));
                                }
                                // A refused picture keeps the previous good one in place. Only a sustained run fails
                                // the take, and then the message says exactly what arrived.
                                Err(reject) => {
                                    let (fatal, total) = {
                                        let mut held = producer_stats.lock().map_err(|_| "Capture statistics poisoned")?;
                                        (held.reject(Instant::now(), &reject), held.rejected)
                                    };
                                    if reject == FrameReject::Empty {
                                        continue;
                                    }
                                    producer_options.progress.rejected_frames.store(total, Ordering::Relaxed);
                                    if fatal {
                                        return Err(format!("{} ({total} refused in a row or within two seconds)", reject.message()));
                                    }
                                }
                            },
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                            Err(e) => {
                                return Err(e.to_string());
                            }
                        }
                    }
                    Ok(())
                })();
                cap.stop_capture();
                capture_result
            },
        ));
        let message = match result {
            Ok(Err(e)) => Some(e),
            Err(_) => Some("The capture adapter failed or source selection was cancelled".into()),
            _ => None,
        };
        if let Some(e) = message {
            if let Ok(mut value) = shared.lock() {
                *value = Err(e);
            }
        }
    });
    let mut metadata = serde_json::json!({
        "engine":"native", "frames":0, "fps":options.fps, "duration":0.0,
        "wallSeconds":0.0, "hardware":super::codec()!="libx264",
        "width":null, "height":null, "camera":camera.is_some(),
        "cameraWidth":null, "cameraHeight":null, "cameraSourceFrames":null,
        "cameraStopped":false,
        "audioDiagnostics":null, "clean_original_exclusion_qualified":false,
        // What made this take heavy or light, so evidence from it applies only to the same kind of take.
        "workload":{"quality":options.quality,"audio":options.microphone.is_some(),"originals":options.originals && camera.is_some(),
            // The target this take was told to record, so its result is attributed to that display and no other.
            "source":target_name(options.selection)},
        "finalization":{"status":"not_started","error":null}
    });
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let wait = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) || super::STOP_REQUEST.load(Ordering::Relaxed) {
                return Err("Source selection cancelled".into());
            }
            let ready = {
                let value = slot.lock().map_err(|_| "Capture slot poisoned")?;
                match &*value {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(e) => return Err(e.clone().into()),
                }
            };
            if ready {
                break;
            }
            if wait.elapsed() > Duration::from_secs(60) {
                return Err("Timed out waiting for screen selection or frames".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        let (w, h) = {
            let value = slot.lock().map_err(|_| "Capture slot poisoned")?;
            let frame = value
                .as_ref()
                .map_err(|e| e.clone())?
                .as_ref()
                .ok_or("No frame")?;
            (frame.width, frame.height)
        };
        let camera_sequence_start = camera
            .as_ref()
            .and_then(|c| c.lock().ok().map(|f| f.sequence));
        let (cw, ch) = if let Some(camera) = &camera {
            let frame = camera.lock().map_err(|_| "Camera source poisoned")?;
            (frame.width, frame.height)
        } else {
            (0, 0)
        };
        metadata["width"] = w.into();
        metadata["height"] = h.into();
        metadata["cameraWidth"] = cw.into();
        metadata["cameraHeight"] = ch.into();
        let ffmpeg = env::var("OPENRECORDER_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let mut encoder = super::pipeline::Pipeline::start_config(
            &ffmpeg,
            &directory,
            w,
            h,
            cw,
            ch,
            options.microphone.clone(),
            options.originals,
            options.fps,
        )?;
        // Honest per-output encoders: a take with one stream on the CPU is not an all-hardware take.
        let summary = encoder.encoder_summary();
        metadata["hardware"] = summary["allHardware"].clone();
        metadata["encoders"] = summary["encoders"].clone();
        options.progress.phase.store(1, Ordering::Relaxed);
        let epoch = Instant::now();
        let mut liveness = CameraLiveness::new(camera_sequence_start, Duration::ZERO);
        let period = Duration::from_secs_f64(1. / options.fps as f64);
        let mut count = 0u32;
        let mut max_lag = 0f64;
        let loop_result = (|| -> Result<(), Box<dyn std::error::Error>> {
            while !cancel.load(Ordering::Relaxed)
                && !super::STOP_REQUEST.load(Ordering::Relaxed)
                && !directory.join(".stop").exists()
                && limit.is_none_or(|duration| epoch.elapsed() < duration)
            {
                if let Some(error) = &options.source_error {
                    if let Some(message) = &*error.lock().map_err(|_| "Camera state unavailable")? {
                        return Err(message.clone().into());
                    }
                }
                // Only the handle to the latest picture is taken under the lock; it is not copied.
                let screen = {
                    let source = slot.lock().map_err(|_| "Capture slot poisoned")?;
                    let frame = source
                        .as_ref()
                        .map_err(|e| e.clone())?
                        .as_ref()
                        .ok_or("Source ended")?;
                    if (frame.width, frame.height) != (w, h) {
                        return Err("Screen dimensions changed".into());
                    }
                    frame.data.clone()
                };
                if let Ok(value) = layout.lock() {
                    encoder.layout = *value;
                }
                if let Some(cam) = &camera {
                    // The camera is locked only while its band and the bubble are copied; waiting for the encoder
                    // happens after it is released, so the camera and its preview never queue behind the encoder.
                    encoder.submit_shared_camera(&screen, cam, |frame| liveness.check(Some(frame.sequence), epoch.elapsed()).map_err(Into::into))?;
                } else {
                    encoder.submit(&screen, None)?;
                }
                count += 1;
                options
                    .progress
                    .frames
                    .store(count as u64, Ordering::Relaxed);
                if count % 15 == 0 {
                    let secs = epoch.elapsed().as_secs_f64();
                    if secs > 0. {
                        options.progress.delivered_milli_fps.store((count as f64 / secs * 1000.) as u32, Ordering::Relaxed);
                    }
                }
                max_lag = max_lag.max(epoch.elapsed().as_secs_f64() - count as f64 / options.fps as f64);
                if epoch.elapsed().as_secs_f64() - count as f64 / options.fps as f64 > 0.75 {
                    return Err(
                        "Recording cannot keep up. Choose a lower resolution or frame rate.".into(),
                    );
                }
                thread::sleep(
                    (epoch + period.mul_f64(count as f64))
                        .saturating_duration_since(Instant::now()),
                );
            }
            Ok(())
        })();
        // Stop the screen producer before potentially slow encoder cleanup.
        cancel.store(true, Ordering::Relaxed);
        options.progress.phase.store(2, Ordering::Relaxed);
        let loop_seconds = epoch.elapsed().as_secs_f64();
        metadata["loopSeconds"] = loop_seconds.into();
        metadata["maxLagSeconds"] = max_lag.into();
        metadata["frames"] = count.into();
        metadata["duration"] = (count as f64 / options.fps as f64).into();
        metadata["cameraStopped"] = loop_result
            .as_ref()
            .err()
            .is_some_and(|error| error.is::<CameraStopped>())
            .into();
        metadata["cameraSourceFrames"] =
            serde_json::json!(camera.as_ref().and_then(|c| c.lock().ok().map(|f| f
                .sequence
                .saturating_sub(camera_sequence_start.unwrap_or(0)))));
        let (result, finalization) = finalize_take(&mut encoder, loop_result);
        metadata["wallSeconds"] = epoch.elapsed().as_secs_f64().into();
        metadata["finalization"] = finalization;
        metadata["audioDiagnostics"] = encoder.audio_diagnostics();
        result
    })();
    cancel.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !producer.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    if producer.is_finished() {
        let _ = producer.join();
    }
    // Output and source cadence side by side and never merged: the nominal setting is a request, the
    // output figure is frames written over the loop, the source figures are what the screen delivered.
    let loop_seconds = metadata["loopSeconds"].as_f64().unwrap_or(0.);
    let written = metadata["frames"].as_u64().unwrap_or(0);
    metadata["cadence"] = serde_json::json!({
        "requestedFps": options.fps,
        "outputFrames": written,
        "outputSeconds": loop_seconds,
        "outputFps": if loop_seconds > 0. { serde_json::json!(written as f64 / loop_seconds) } else { serde_json::Value::Null },
        // How far the sampler fell behind its fixed clock at worst (the real-time gate trips at 0.75 s).
        "maxLagSeconds": metadata["maxLagSeconds"].clone(),
    });
    if let Ok(held) = stats.lock() {
        metadata["source"] = held.json();
    }
    metadata["status"] = if result.is_ok() {
        "completed"
    } else {
        "failed"
    }
    .into();
    metadata["error"] = serde_json::json!(result.as_ref().err().map(|e| e.to_string()));
    if let Ok(mut summary) = options.progress.summary.lock() {
        *summary = Some(metadata.clone());
    }
    let metadata_result = serde_json::to_vec_pretty(&metadata)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(directory.join("capture.json"), bytes));
    match (result, metadata_result) {
        (Err(primary), Err(_)) => {
            Err(format!("{primary}; capture diagnostics could not be saved").into())
        }
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(error)) => Err(error.into()),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// How a take's target is named in its metadata. The same name the app keyed the take's measurements on.
pub fn target_name(selection: Option<(bool, u32)>) -> String {
    match selection {
        Some((false, id)) => format!("display:{id}"),
        Some((true, _)) => "window".to_owned(),
        None => "default".to_owned(),
    }
}

/// Who owns a listed window, from the backend's process metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    Own,
    Foreign,
    /// The backend gave no owner (Linux portals, a failed query). Treated conservatively: never excluded
    /// or hidden as if it were ours, and no clean or de-duplicated capture is promised for it.
    Unknown,
}
/// Ownership policy shared by the recorder's exclusion list and the window picker. Title is not an input.
pub fn window_ownership(owner_pid: Option<u32>, own_pid: u32) -> Ownership {
    match owner_pid {
        Some(pid) if pid == own_pid => Ownership::Own,
        Some(_) => Ownership::Foreign,
        None => Ownership::Unknown,
    }
}

#[cfg(test)]
mod screen_scaling_tests {
    use super::*;

    fn frame(w: i32, h: i32, data: Vec<u8>) -> Frame {
        Frame::BGRA(scap::frame::BGRAFrame {
            width: w,
            height: h,
            data,
            display_time: 0,
        })
    }

    #[test]
    fn landscape_and_portrait_scale_the_same_content_by_long_edge() {
        // Distinct pixels expose row/column addressing errors as well as size errors.
        for (w, h, expected) in [(8, 4, (4, 2)), (4, 8, (2, 4))] {
            let data: Vec<u8> = (0..w * h).flat_map(|n| [n as u8, 20, 30, 255]).collect();
            let scaled = pixels(frame(w, h, data), 4).unwrap();
            assert_eq!((scaled.width, scaled.height), expected);
            for y in 0..scaled.height {
                for x in 0..scaled.width {
                    let original_index = (y * 2 * w as usize + x * 2) as u8;
                    let offset = (y * scaled.width + x) * 4;
                    assert_eq!(
                        &scaled.data[offset..offset + 4],
                        &[original_index, 20, 30, 255]
                    );
                }
            }
        }
        // Exercise the full native portrait budget without allocating its pixels.
        assert_eq!(screen_dimensions(2400, 3840, 2560).unwrap(), (1600, 2560));
        assert_eq!(screen_dimensions(3840, 2400, 2560).unwrap(), (2560, 1600));
    }

    #[test]
    fn small_frames_never_upscale_and_outputs_are_even() {
        let data: Vec<u8> = (0..8 * 4 * 4).map(|n| n as u8).collect();
        let unchanged = pixels(frame(8, 4, data.clone()), 1280).unwrap();
        assert_eq!((unchanged.width, unchanged.height), (8, 4));
        assert_eq!(*unchanged.data, data);
        for (w, h) in [(7, 5), (5, 7)] {
            let scaled = pixels(frame(w, h, vec![255; (w * h * 4) as usize]), 1280).unwrap();
            assert_eq!(
                (scaled.width, scaled.height),
                ((w as usize) & !1, (h as usize) & !1)
            );
        }
        assert_eq!(screen_dimensions(9, 5, 7).unwrap(), (6, 2));
    }

    #[test]
    #[ignore = "requires explicitly supplied bundled helper and development ffprobe"]
    fn portrait_screen_above_2160_survives_real_encoder() {
        let helper = env::var("OPENRECORDER_FFMPEG").expect("Set bundled fixture helper");
        let probe = env::var("OPENRECORDER_TEST_FFPROBE").expect("Set development ffprobe");
        let dir =
            env::temp_dir().join(format!("recorder-portrait-screen-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        // A narrow portrait frame exercises the height limit with only 160 KiB.
        let screen = pixels(frame(16, 2560, [40u8, 50, 60, 255].repeat(16 * 2560)), 2560).unwrap();
        let mut encoder = super::super::pipeline::Pipeline::start_config(
            &helper,
            &dir,
            screen.width,
            screen.height,
            0,
            0,
            None,
            false,
            30,
        )
        .unwrap();
        for _ in 0..2 {
            encoder.set_screen(&screen.data).unwrap();
            encoder.write().unwrap();
        }
        encoder.finish().unwrap();
        let out = std::process::Command::new(&probe)
            .args([
                "-v",
                "error",
                "-count_frames",
                "-show_entries",
                "stream=width,height,nb_read_frames",
                "-of",
                "json",
            ])
            .arg(dir.join("recording.mp4"))
            .output()
            .unwrap();
        assert!(out.status.success());
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let stream = &json["streams"][0];
        assert_eq!(stream["width"], 16);
        assert_eq!(stream["height"], 2560);
        assert_eq!(stream["nb_read_frames"], "2");
        drop(encoder);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_dimensions_or_pixel_lengths_are_rejected_before_allocation() {
        for (w, h) in [
            (0, 4),
            (-1, 4),
            (4, 1),
            (3841, 2),
            (2, 3841),
            (3840, 2401),
            (2401, 3840),
            (i32::MAX, i32::MAX),
        ] {
            assert!(screen_dimensions(w, h, 2560).is_err());
            assert!(pixels(frame(w, h, vec![]), 2560).is_err());
        }
        for length in [0, 15, 17] {
            assert!(pixels(frame(2, 2, vec![0; length]), 1280).is_err());
        }
        for maximum in [0, 1] {
            assert!(pixels(frame(2, 2, vec![0; 16]), maximum).is_err());
        }
        assert!(screen_dimensions(3840, 2, 1280).is_err());
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    #[ignore = "requires explicitly supplied bundled helper"]
    fn primary_failure_survives_encoder_finalization_failure() {
        let helper = env::var("OPENRECORDER_FFMPEG").expect("Set bundled fixture helper");
        let dir = env::temp_dir().join(format!("recorder-finalization-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let mut encoder = super::super::pipeline::Pipeline::start_config(
            &helper, &dir, 640, 360, 0, 0, None, false, 30,
        )
        .unwrap();
        encoder
            .set_screen(&[40u8, 50, 60, 255].repeat(640 * 360))
            .unwrap();
        encoder.write().unwrap();
        {
            let mut child = encoder.child.lock().unwrap();
            child.kill().unwrap();
            child.wait().unwrap();
        }
        let (result, finalization) =
            finalize_take(&mut encoder, Err("Synthetic source failure".into()));
        assert_eq!(result.unwrap_err().to_string(), "Synthetic source failure");
        assert_eq!(finalization["status"], "failed");
        assert!(finalization["error"]
            .as_str()
            .unwrap()
            .contains("Encoder failed"));
        drop(encoder);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "requires explicitly supplied bundled helper and development ffprobe"]
    fn failed_take_preserves_partial_real_encoder_outputs() {
        let helper = env::var("OPENRECORDER_FFMPEG").expect("Set bundled fixture helper");
        let probe = env::var("OPENRECORDER_TEST_FFPROBE").expect("Set development ffprobe");
        for shape_change in [false, true] {
            let dir = env::temp_dir().join(format!("recorder-partial-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).unwrap();
            let mut encoder = super::super::pipeline::Pipeline::start_config(
                &helper, &dir, 640, 360, 144, 240, None, true, 30,
            )
            .unwrap();
            let screen = [40u8, 50, 60, 255].repeat(640 * 360);
            let mut camera = crate::camera_frame::Frame {
                width: 144,
                height: 240,
                bgra: [60u8, 80, 100, 255].repeat(144 * 240),
                sequence: 0,
            };
            for _ in 0..12 {
                encoder.set_screen(&screen).unwrap();
                set_camera_frame(&mut encoder, &camera).unwrap();
                encoder.write().unwrap();
            }
            let primary = if shape_change {
                // Same byte count, different shape: reject before copying pixels.
                camera.width = 240;
                camera.height = 144;
                set_camera_frame(&mut encoder, &camera).map_err(Into::into)
            } else {
                Err("Synthetic camera source ended".into())
            };
            let expected = if shape_change {
                "Camera dimensions changed"
            } else {
                "Synthetic camera source ended"
            };
            let (result, finalization) = finalize_take(&mut encoder, primary);
            assert_eq!(result.unwrap_err().to_string(), expected);
            assert_eq!(finalization["status"], "completed");
            for (name, w, h) in [
                ("camera", 144, 240),
                ("screen", 640, 360),
                ("recording", 640, 360),
            ] {
                let out = std::process::Command::new(&probe)
                    .args([
                        "-v",
                        "error",
                        "-count_frames",
                        "-show_entries",
                        "stream=width,height,nb_read_frames",
                        "-of",
                        "json",
                    ])
                    .arg(dir.join(format!("{name}.mp4")))
                    .output()
                    .unwrap();
                assert!(out.status.success());
                let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
                let stream = &json["streams"][0];
                assert_eq!(stream["width"], w);
                assert_eq!(stream["height"], h);
                assert_eq!(stream["nb_read_frames"], "12");
            }
            drop(encoder);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[cfg(test)]
mod camera_liveness_tests {
    use super::*;

    fn at(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn unchanged_pixels_with_advancing_sequence_never_stall() {
        // A still face decodes to identical pixels; only the sequence counts.
        let still = [90u8, 80, 70, 255].repeat(16);
        let mut liveness = CameraLiveness::new(Some(0), at(0));
        for tick in 1..=600u64 {
            let frame = crate::camera_frame::Frame {
                width: 4,
                height: 4,
                bgra: still.clone(),
                sequence: tick,
            };
            assert!(liveness.check(Some(frame.sequence), at(tick * 33)).is_ok());
        }
    }

    #[test]
    fn a_stalled_sequence_stops_only_after_the_threshold() {
        let mut liveness = CameraLiveness::new(Some(5), at(1000));
        assert!(liveness.check(Some(5), at(1000)).is_ok());
        assert!(liveness.check(Some(5), at(3500)).is_ok());
        let error = liveness.check(Some(5), at(3501)).unwrap_err();
        assert!(error
            .to_string()
            .starts_with("Camera stopped sending video"));
        assert!(error.to_string().contains("2.5 seconds"));
        // The error is a typed value the take can recognise in its metadata.
        let boxed: Box<dyn std::error::Error> = error.into();
        assert!(boxed.is::<CameraStopped>());
    }

    #[test]
    fn resumed_frames_and_sequence_resets_restart_the_window() {
        let mut liveness = CameraLiveness::new(Some(10), at(0));
        assert!(liveness.check(Some(10), at(2400)).is_ok());
        assert!(liveness.check(Some(11), at(2500)).is_ok()); // resumed
        assert!(liveness.check(Some(11), at(4900)).is_ok());
        assert!(liveness.check(Some(0), at(5000)).is_ok()); // restarted stream
        assert!(liveness.check(Some(0), at(7500)).is_ok());
        assert!(liveness.check(Some(0), at(7501)).is_err());
        assert!(liveness.check(Some(1), at(7600)).is_ok());
    }

    #[test]
    fn no_camera_never_stalls() {
        let mut liveness = CameraLiveness::new(None, at(0));
        for seconds in [0, 3, 60, 3600] {
            assert!(liveness.check(None, Duration::from_secs(seconds)).is_ok());
        }
    }
}
#[cfg(test)]
mod ownership_tests {
    use super::*;
    // (id, title, owner pid): the title is carried only to prove it never decides anything.
    const OWN: u32 = 4242;
    const WINDOWS: [(u32, &str, Option<u32>); 6] = [
        (1, "OpenRecorder", Some(OWN)),
        (2, "OpenRecorder Camera", Some(OWN)),
        (3, "Renamed by a future build", Some(OWN)),
        (4, "OpenRecorder roadmap — Notes", Some(777)),
        (5, "OpenRecorder", Some(778)),
        (6, "OpenRecorder (portal window)", None),
    ];
    fn excluded() -> Vec<u32> {
        WINDOWS.iter().filter(|(_, _, pid)| window_ownership(*pid, OWN) == Ownership::Own).map(|(id, ..)| *id).collect()
    }
    fn listed_in_picker() -> Vec<u32> {
        WINDOWS.iter().filter(|(_, _, pid)| window_ownership(*pid, OWN) != Ownership::Own).map(|(id, ..)| *id).collect()
    }
    #[test]
    fn own_windows_are_excluded_by_process_even_when_renamed() {
        assert_eq!(excluded(), vec![1, 2, 3], "the renamed own window is excluded too");
    }
    #[test]
    fn a_foreign_window_with_the_same_title_prefix_is_kept_in_the_recording_and_the_picker() {
        for foreign in [4, 5] {
            assert!(!excluded().contains(&foreign));
            assert!(listed_in_picker().contains(&foreign));
        }
        assert_eq!(window_ownership(Some(777), OWN), Ownership::Foreign);
    }
    #[test]
    fn unknown_ownership_is_never_treated_as_ours() {
        assert_eq!(window_ownership(None, OWN), Ownership::Unknown);
        assert!(!excluded().contains(&6) && listed_in_picker().contains(&6));
    }
    #[test]
    fn capture_and_picker_use_this_one_policy_and_no_title_prefix_filter_remains() {
        let production = include_str!("capture.rs").split("#[cfg(test)]").next().unwrap();
        assert!(production.contains("window_ownership(w.owner_pid, own_pid)"));
        let desktop = include_str!("desktop.rs").split("#[cfg(test)]\nmod fixtures").next().unwrap();
        assert!(desktop.contains("capture::window_ownership(w.owner_pid, own_pid)"));
        for (name, text) in [("capture.rs", production), ("desktop.rs", desktop)] {
            assert!(!text.contains("title.starts_with(\"OpenRecorder"), "{name} still filters by title");
        }
    }
}

#[cfg(test)]
mod frame_check_tests {
    use super::*;
    fn bgra(w: i32, h: i32, len: usize) -> Frame {
        Frame::BGRA(scap::frame::BGRAFrame { display_time: 0, width: w, height: h, data: vec![0; len] })
    }
    #[test]
    fn every_refusal_says_what_arrived_and_keeps_the_stable_prefix() {
        let bounds = pixels(bgra(5120, 2880, 0), 2560).err().unwrap();
        assert_eq!(bounds, FrameReject::Bounds { w: 5120, h: 2880 });
        assert!(bounds.message().starts_with(REJECT_PREFIX) && bounds.message().contains("5120x2880"));
        let length = pixels(bgra(2560, 1440, 2560 * 1440 * 4 + 64 * 4), 2560).err().unwrap();
        assert!(length.message().starts_with(REJECT_PREFIX));
        assert!(length.message().contains("14745856 bytes") && length.message().contains("expected 14745600"), "{}", length.message());
        assert_eq!(length.sample()["expectedBytes"], 2560 * 1440 * 4);
        assert_ne!(bounds.message(), length.message(), "dimension and stride failures are distinguishable");
        assert_eq!(FrameReject::Format.message(), "The capture source returned an unsupported pixel format");
        // A correct 2560x1440 buffer is accepted exactly as before; nothing about the bounds changed.
        assert!(pixels(bgra(2560, 1440, 2560 * 1440 * 4), 2560).is_ok());
    }
    #[test]
    fn an_odd_picture_is_tolerated_but_a_sustained_run_fails_with_evidence() {
        let t0 = Instant::now();
        let odd = FrameReject::Length { w: 2560, h: 1440, len: 7 };
        let mut stats = SourceStats::default();
        stats.accept(t0);
        assert!(!stats.reject(t0 + Duration::from_millis(16), &odd));
        stats.accept(t0 + Duration::from_millis(33));
        // A good picture ends the run.
        assert_eq!(stats.run, 0);
        for i in 0..(REJECT_RUN_FRAMES - 1) {
            assert!(!stats.reject(t0 + Duration::from_millis(40 + i), &odd), "{i}");
        }
        assert!(stats.reject(t0 + Duration::from_millis(200), &odd), "the {REJECT_RUN_FRAMES}th in a row fails");
        // Time also bounds the run, for a slow source.
        let mut slow = SourceStats::default();
        assert!(!slow.reject(t0, &odd));
        assert!(!slow.reject(t0 + Duration::from_millis(1999), &odd));
        assert!(slow.reject(t0 + Duration::from_millis(2000), &odd));
        let json = stats.json();
        assert_eq!(json["acceptedFrames"], 2);
        assert_eq!(json["rejectedFrames"], REJECT_RUN_FRAMES + 1);
        assert_eq!(json["rejectedSamples"].as_array().unwrap().len(), 5, "samples are bounded");
        assert_eq!(json["rejectedSamples"][0]["acceptedBefore"], 1);
        assert_eq!(json["rejectedSamples"][0]["bytes"], 7);
    }
    #[test]
    fn an_empty_picture_is_a_status_tick_counted_apart_and_never_part_of_a_refusal_run() {
        // The owner's take delivered zero-sized pictures between good ones (7 in 820): they are not refusals.
        for (w, h) in [(0, 0), (0, 1440), (2560, 0)] {
            assert_eq!(pixels(bgra(w, h, 0), 2560).err().unwrap(), FrameReject::Empty, "{w}x{h}");
        }
        assert_eq!(FrameReject::Empty.message(), "The capture source returned an empty picture");
        assert_eq!(FrameReject::Empty.sample()["reason"], "empty");
        // A tiny or oversized picture is still a real refusal.
        assert!(matches!(pixels(bgra(1, 1, 4), 2560).err().unwrap(), FrameReject::Bounds { .. }));
        let t0 = Instant::now();
        let mut stats = SourceStats::default();
        stats.accept(t0);
        // However many arrive, and however long they last, they never fail a take.
        for i in 0..(REJECT_RUN_FRAMES * 3) {
            assert!(!stats.reject(t0 + Duration::from_millis(10 * i), &FrameReject::Empty));
        }
        assert!(!stats.reject(t0 + REJECT_RUN_TIME * 5, &FrameReject::Empty), "nor does time");
        assert_eq!((stats.rejected, stats.run), (0, 0), "they are not refusals and do not extend a run");
        let odd = FrameReject::Length { w: 2560, h: 1440, len: 7 };
        // A real refusal run in progress is neither advanced nor reset by empty ticks in between.
        for i in 0..(REJECT_RUN_FRAMES - 1) {
            assert!(!stats.reject(t0 + Duration::from_millis(i), &odd));
            assert!(!stats.reject(t0 + Duration::from_millis(i), &FrameReject::Empty));
        }
        assert_eq!(stats.run, REJECT_RUN_FRAMES - 1);
        assert!(stats.reject(t0 + Duration::from_millis(500), &odd), "the real run still fails at its own length");
        let json = stats.json();
        assert_eq!(json["emptyFrames"], REJECT_RUN_FRAMES * 3 + 1 + (REJECT_RUN_FRAMES - 1));
        assert_eq!(json["rejectedFrames"], REJECT_RUN_FRAMES);
        assert!(json["rejectedSamples"].as_array().unwrap().iter().all(|s| s["reason"] != "empty"), "empty ticks take no sample slot");
    }
    #[test]
    fn the_producer_skips_empty_pictures_without_touching_the_slot_or_the_refusal_counters() {
        let source = include_str!("capture.rs").split("#[cfg(test)]").next().unwrap();
        assert!(source.contains("if reject == FrameReject::Empty {\n                                        continue;"));
        // The sampler hands the encoder the capture's own buffer and records how far behind its clock it fell.
        assert!(source.contains("frame.data.clone()") && source.contains("encoder.submit(&screen, None)?"));
        assert!(source.contains("\"maxLagSeconds\": metadata[\"maxLagSeconds\"].clone()"));
        // The gate is exactly what it was.
        assert!(source.contains("if epoch.elapsed().as_secs_f64() - count as f64 / options.fps as f64 > 0.75 {"));
        // Per-output encoders are recorded, and `hardware` is only true when every output is.
        assert!(source.contains("metadata[\"hardware\"] = summary[\"allHardware\"].clone();"));
    }
    #[test]
    fn source_arrival_rate_is_its_own_figure_not_the_output_rate() {
        let t0 = Instant::now();
        let mut stats = SourceStats::default();
        for i in 0..31u64 {
            stats.accept(t0 + Duration::from_millis(i * 100)); // a 10 fps source
        }
        let json = stats.json();
        assert!((json["arrivalFps"].as_f64().unwrap() - 10.0).abs() < 1e-6, "{json}");
        assert!((json["maxGapMs"].as_f64().unwrap() - 100.0).abs() < 1e-6);
        assert!(SourceStats::default().json()["arrivalFps"].is_null(), "unknown stays unknown");
    }
}
