#[cfg(feature = "desktop-ui")]
pub type AudioHandle = Arc<crate::audio::Shared>;
#[cfg(not(feature = "desktop-ui"))]
pub type AudioHandle = ();
/// `shape`: 0 circle, 1 rounded square, 2 legacy plain square, 3 wide 16:9.
/// `mirror` is the only mirroring the composite applies; a preview-only
/// mirror must be kept out of this struct (set it to `false`).
#[derive(Clone, Copy)]
pub struct Layout {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub shape: u8,
    pub mirror: bool,
    /// False draws no bubble at all (the camera may still be running and its original is untouched).
    pub visible: bool,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            x: 0.04,
            y: 0.7,
            size: 0.2,
            shape: 1,
            mirror: true,
            visible: true,
        }
    }
}
/// Camera bubble size for a base edge, in the caller's unit. Circle, rounded
/// and legacy square are `base` x `base`; wide is a true 16:9 whose width is
/// `base` * 4/3 and whose height is `base` * 3/4; portrait is its 9:16 turn,
/// `base` * 3/4 wide and `base` * 4/3 tall.
pub fn camera_dimensions(base: f32, shape: u8) -> (f32, f32) {
    match shape {
        3 => (base * 4. / 3., base * 3. / 4.),
        4 => (base * 3. / 4., base * 4. / 3.),
        _ => (base, base),
    }
}
// Whether the bubble pixel (x, y) of a `bw` x `bh` bubble is inside its mask.
// Circle fills the inscribed circle, rounded uses 22% of the short side,
// wide and portrait use 20% of the short side, and legacy square is unmasked.
fn bubble_contains(shape: u8, bw: usize, bh: usize, x: usize, y: usize) -> bool {
    let (hw, hh) = (bw as f32 / 2., bh as f32 / 2.);
    let dx = (x as f32 + 0.5 - hw).abs();
    let dy = (y as f32 + 0.5 - hh).abs();
    let radius = match shape {
        2 => return true,
        0 => {
            let r = hw.min(hh);
            return dx * dx + dy * dy <= r * r;
        }
        3 | 4 => bw.min(bh) as f32 * 0.2,
        _ => bw.min(bh) as f32 * 0.22,
    };
    let qx = (dx - (hw - radius)).max(0.);
    let qy = (dy - (hh - radius)).max(0.);
    qx * qx + qy * qy <= radius * radius
}
/// Where the bubble lands in the screen and how camera pixels map onto it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bubble {
    pub x0: usize,
    pub y0: usize,
    pub bw: usize,
    pub bh: usize,
    scale: f32,
    ox: f32,
    oy: f32,
}
pub(crate) fn bubble_geometry(
    (width, height): (usize, usize),
    (cw, ch): (usize, usize),
    layout: &Layout,
) -> Bubble {
    let base = (width as f32 * layout.size.clamp(0.01, 0.5))
        .min(width as f32)
        .min(height as f32)
        .max(2.);
    let (fw, fh) = camera_dimensions(base, layout.shape);
    let fit = (width as f32 / fw).min(height as f32 / fh).min(1.);
    let bw = ((fw * fit).round() as usize).clamp(2, width);
    let bh = ((fh * fit).round() as usize).clamp(2, height);
    let x0 = ((layout.x.clamp(0., 1.) * width as f32) as usize).min(width - bw);
    let y0 = ((layout.y.clamp(0., 1.) * height as f32) as usize).min(height - bh);
    let scale = (bw as f32 / cw as f32).max(bh as f32 / ch as f32);
    let ox = (cw as f32 - bw as f32 / scale) / 2.;
    let oy = (ch as f32 - bh as f32 / scale) / 2.;
    Bubble { x0, y0, bw, bh, scale, ox, oy }
}
/// Draws the bubble into `rows`, a buffer of `stride` pixels per row that starts at screen row `first_row`. The
/// camera image is scaled uniformly to cover the bubble and cropped around its centre.
pub(crate) fn draw_bubble(rows: &mut [u8], first_row: usize, stride: usize, b: &Bubble, pixels: &[u8], (cw, ch): (usize, usize), layout: &Layout) {
    for y in 0..b.bh {
        let sy = ((b.oy + (y as f32 + 0.5) / b.scale) as usize).min(ch - 1);
        for x in 0..b.bw {
            if !bubble_contains(layout.shape, b.bw, b.bh, x, y) {
                continue;
            }
            let mx = if layout.mirror { b.bw - 1 - x } else { x };
            let sx = ((b.ox + (mx as f32 + 0.5) / b.scale) as usize).min(cw - 1);
            let i = ((b.y0 + y - first_row) * stride + b.x0 + x) * 4;
            let j = (sy * cw + sx) * 4;
            rows[i..i + 4].copy_from_slice(&pixels[j..j + 4]);
        }
    }
}
// Draws the camera bubble into a frame of `stride` pixels per row whose
// screen content is the top-left `width` x `height`.
fn composite_camera(
    frame: &mut [u8],
    stride: usize,
    screen: (usize, usize),
    pixels: &[u8],
    camera: (usize, usize),
    layout: &Layout,
) {
    if !layout.visible {
        return;
    }
    let bubble = bubble_geometry(screen, camera, layout);
    draw_bubble(frame, 0, stride, &bubble, pixels, camera, layout);
}
// Packed layout: screen band, unmirrored full camera band, then the combined
// recording band. Only the combined band receives the bubble.
fn place_camera(
    frame: &mut [u8],
    stride: usize,
    screen: (usize, usize),
    pixels: &[u8],
    camera: (usize, usize),
    original_tracks: bool,
    layout: &Layout,
) {
    let (width, height) = screen;
    let (cw, ch) = camera;
    let combined = if original_tracks {
        for y in 0..ch {
            let start = (height + y) * stride * 4;
            frame[start..start + cw * 4].copy_from_slice(&pixels[y * cw * 4..(y + 1) * cw * 4]);
        }
        let combined_start = (height + ch) * stride * 4;
        let (left, combined) = frame.split_at_mut(combined_start);
        combined.copy_from_slice(&left[..stride * height * 4]);
        combined
    } else {
        frame
    };
    composite_camera(combined, stride, (width, height), pixels, camera, layout);
}
/// How many megapixels per second the take asks of its video encoders (a diagnostic recorded in the timing sidecar;
/// it never changes which encoder writes a file).
pub(crate) fn hardware_megapixels_per_second(w: usize, h: usize, cw: usize, ch: usize, fps: u32, originals_with_camera: bool) -> f64 {
    let screens = if originals_with_camera { 2.0 } else { 1.0 };
    let camera = if originals_with_camera { (cw * ch) as f64 } else { 0.0 };
    (screens * (w * h) as f64 + camera) * f64::from(fps) / 1e6
}
// Bounded encoder stage timings. Everything is fixed-size so `write` neither
// allocates nor touches the disk; the sidecar is written once at the end.
const EARLY_WRITES: usize = 8;
const WRITE_BUCKETS_MS: [u128; 5] = [10, 33, 100, 250, 750];
struct Timing {
    created: Instant,
    spawn: Duration,
    writes: u64,
    write_failures: u64,
    first_write_at: Option<Duration>,
    early: [Duration; EARLY_WRITES],
    max_write: Duration,
    max_write_index: u64,
    total_write: Duration,
    buckets: [u64; WRITE_BUCKETS_MS.len() + 1],
    audio_finish: Duration,
    encoder_wait: Duration,
    remux: Duration,
    finish_total: Duration,
    finish_called: bool,
    written: bool,
    // Sampler side: building a frame, and waiting to hand it to the writer (true backpressure).
    frames: u64,
    prep_total: Duration,
    prep_max: Duration,
    handoff_total: Duration,
    handoff_max: Duration,
    handoff_blocked: u64,
}
/// A hand-off that waited longer than this counts as the encoder pushing back.
const HANDOFF_BLOCKED: Duration = Duration::from_millis(2);
impl Timing {
    fn record_sampler(&mut self, prep: Duration, handoff: Duration) {
        self.frames += 1;
        self.prep_total += prep;
        self.prep_max = self.prep_max.max(prep);
        self.handoff_total += handoff;
        self.handoff_max = self.handoff_max.max(handoff);
        if handoff > HANDOFF_BLOCKED {
            self.handoff_blocked += 1;
        }
    }
    fn new(created: Instant) -> Self {
        Self {
            created,
            spawn: Duration::ZERO,
            writes: 0,
            write_failures: 0,
            first_write_at: None,
            early: [Duration::ZERO; EARLY_WRITES],
            max_write: Duration::ZERO,
            max_write_index: 0,
            total_write: Duration::ZERO,
            buckets: [0; WRITE_BUCKETS_MS.len() + 1],
            audio_finish: Duration::ZERO,
            encoder_wait: Duration::ZERO,
            remux: Duration::ZERO,
            finish_total: Duration::ZERO,
            finish_called: false,
            written: false,
            frames: 0,
            prep_total: Duration::ZERO,
            prep_max: Duration::ZERO,
            handoff_total: Duration::ZERO,
            handoff_max: Duration::ZERO,
            handoff_blocked: 0,
        }
    }
    fn record_write(&mut self, began: Instant, took: Duration, ok: bool) {
        if self.first_write_at.is_none() {
            self.first_write_at = Some(began.saturating_duration_since(self.created));
        }
        if let Some(slot) = self.early.get_mut(self.writes as usize) {
            *slot = took;
        }
        if took > self.max_write {
            self.max_write = took;
            self.max_write_index = self.writes;
        }
        let millis = took.as_millis();
        let bucket = WRITE_BUCKETS_MS
            .iter()
            .position(|limit| millis <= *limit)
            .unwrap_or(WRITE_BUCKETS_MS.len());
        self.buckets[bucket] += 1;
        self.total_write += took;
        self.writes += 1;
        if !ok {
            self.write_failures += 1;
        }
    }
    fn to_json(
        &self,
        codec: &str,
        shape: serde_json::Value,
        outcome: &str,
        audio_failed: bool,
        video_failed: bool,
    ) -> serde_json::Value {
        let ms = |d: Duration| (d.as_secs_f64() * 1e6).round() / 1e3;
        let recorded = (self.writes as usize).min(EARLY_WRITES);
        serde_json::json!({
            "schema":1,"outcome":outcome,"finishCalled":self.finish_called,
            "codec":codec,"encoder":shape,
            "startup":{"spawnMs":ms(self.spawn)},
            "writes":{
                "count":self.writes,"failed":self.write_failures,
                "firstWriteAtMs":self.first_write_at.map(ms),
                "firstWritesMs":self.early[..recorded].iter().map(|d|ms(*d)).collect::<Vec<_>>(),
                "maxMs":ms(self.max_write),"maxIndex":self.max_write_index,
                "totalMs":ms(self.total_write),
                "bucketUpperMs":WRITE_BUCKETS_MS,"bucketCounts":self.buckets
            },
            "finalization":{
                "audioMs":ms(self.audio_finish),"encoderWaitMs":ms(self.encoder_wait),
                "remuxMs":ms(self.remux),"totalMs":ms(self.finish_total),
                "audioFailed":audio_failed,"videoFailed":video_failed
            },
            "sampler":{
                "frames":self.frames,
                "prepMsMean":ms(self.prep_total / self.frames.max(1) as u32),"prepMsMax":ms(self.prep_max),
                "handoffWaitMsMean":ms(self.handoff_total / self.frames.max(1) as u32),"handoffWaitMsMax":ms(self.handoff_max),
                "handoffBlockedFrames":self.handoff_blocked
            },
            "sinceStartMs":ms(self.created.elapsed())
        })
    }
}
use std::{
    io,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    ops::Range,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
/// What `Pipeline` writes to the encoder, in order. The helper reads one packed raw frame per picture:
/// screen band, camera band, combined band. Bands are never assembled into one buffer: the screen band is
/// the capture's own buffer, and the combined band is the same buffer with only the bubble's rows replaced.
enum Part {
    /// A range of a buffer someone else owns (the captured screen). Written as is, never copied.
    Shared(Arc<Vec<u8>>, Range<usize>),
    /// A buffer this frame owns; it goes back to the pool once written.
    Owned(Vec<u8>, Kind),
}
#[derive(Clone, Copy)]
enum Kind {
    /// The camera original band: full rows, zero padding beyond the camera's width (never rewritten).
    Camera = 0,
    /// The bubble's rows of the combined band.
    Patch = 1,
    /// A screen-sized band with padded rows (only when the camera is wider than the screen).
    Padded = 2,
}
impl Part {
    fn bytes(&self) -> &[u8] {
        match self {
            Part::Shared(buffer, range) => &buffer[range.clone()],
            Part::Owned(buffer, _) => buffer,
        }
    }
}
struct Job {
    parts: Vec<Part>,
}
/// A built picture and how long building it took, waiting to be handed to the writer.
struct Prepared {
    job: Job,
    built: Duration,
}
/// A few recycled buffers per kind, so a steady state allocates nothing.
#[derive(Default)]
struct Pool([Vec<Vec<u8>>; 3]);
type SharedPool = Arc<Mutex<Pool>>;
const POOL_KEEP: usize = 3;
fn take_buffer(pool: &SharedPool, kind: Kind, len: usize) -> Vec<u8> {
    let mut held = pool.lock().unwrap_or_else(|e| e.into_inner());
    match kind {
        // Fixed-size buffers keep their zero padding for life, so only an exact size is reused.
        Kind::Camera | Kind::Padded => match held.0[kind as usize].iter().position(|b| b.len() == len) {
            Some(i) => held.0[kind as usize].swap_remove(i),
            None => vec![0; len],
        },
        // The patch is cleared and refilled each frame; its capacity is what is reused.
        Kind::Patch => held.0[kind as usize].pop().map(|mut b| {
            b.clear();
            b
        }).unwrap_or_default(),
    }
}
fn give_buffer(pool: &SharedPool, kind: Kind, buffer: Vec<u8>) {
    let mut held = pool.lock().unwrap_or_else(|e| e.into_inner());
    if held.0[kind as usize].len() < POOL_KEEP {
        held.0[kind as usize].push(buffer);
    }
}
/// The thread that owns the encoder's input. The sampler hands it a finished frame and goes on building the next
/// one while this one is written, so a frame's preparation and its (blocking) write overlap.
struct Writer {
    done: Receiver<()>,
    join: Option<thread::JoinHandle<()>>,
}
impl Writer {
    fn spawn(
        mut input: ChildStdin,
        jobs: Receiver<Job>,
        timing: Arc<Mutex<Timing>>,
        heartbeat: Arc<Mutex<Instant>>,
        failure: Arc<Mutex<Option<String>>>,
        pool: SharedPool,
    ) -> io::Result<Self> {
        let (finished, done) = mpsc::channel();
        let join = thread::Builder::new().name("encoder-writer".into()).spawn(move || {
            use io::Write;
            while let Ok(job) = jobs.recv() {
                let began = Instant::now();
                let result = job.parts.iter().try_for_each(|part| input.write_all(part.bytes()));
                timing.lock().unwrap_or_else(|e| e.into_inner()).record_write(began, began.elapsed(), result.is_ok());
                for part in job.parts {
                    if let Part::Owned(buffer, kind) = part {
                        give_buffer(&pool, kind, buffer);
                    }
                }
                match result {
                    Ok(()) => {
                        if let Ok(mut time) = heartbeat.lock() {
                            *time = Instant::now();
                        }
                    }
                    Err(error) => {
                        *failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.to_string());
                        break;
                    }
                }
            }
            // Dropping `jobs` makes a sender that is still waiting fail instead of hang; dropping `input` ends the stream.
            drop(jobs);
            drop(input);
            let _ = finished.send(());
        })?;
        Ok(Self { done, join: Some(join) })
    }
    /// Waits for every queued frame to be written and the input closed. False if it did not happen in time.
    fn wait(&mut self, limit: Duration) -> bool {
        let drained = self.done.recv_timeout(limit).is_ok();
        if drained {
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
        drained
    }
}
pub struct Pipeline {
    pub layout: Layout,
    #[cfg(feature = "desktop-ui")]
    audio: Option<crate::audio::Session>,
    pub child: Arc<Mutex<Child>>,
    alive: Arc<AtomicBool>,
    heartbeat: Arc<Mutex<Instant>>,
    jobs: Option<SyncSender<Job>>,
    writer: Option<Writer>,
    /// Why the writer stopped, when it did (a closed pipe, a full disk behind the encoder).
    failure: Arc<Mutex<Option<String>>>,
    pool: SharedPool,
    /// The last picture and camera frame given through the copying `set_screen` / `set_camera` API.
    held_screen: Option<Arc<Vec<u8>>>,
    held_camera: Option<Vec<u8>>,
    pub width: usize,
    pub height: usize,
    pub camera_width: usize,
    pub camera_height: usize,
    original_tracks: bool,
    packed_width: usize,
    outputs: Vec<std::path::PathBuf>,
    executable: String,
    final_audio_diagnostics: Option<serde_json::Value>,
    codec: &'static str,
    fps: u32,
    has_audio: bool,
    timing: Arc<Mutex<Timing>>,
    timing_path: PathBuf,
}
impl Pipeline {
    pub fn start(
        exe: &str,
        dir: &Path,
        w: usize,
        h: usize,
        cw: usize,
        ch: usize,
    ) -> io::Result<Self> {
        Self::start_with_audio(exe, dir, w, h, cw, ch, None)
    }
    pub fn start_with_audio(
        exe: &str,
        dir: &Path,
        w: usize,
        h: usize,
        cw: usize,
        ch: usize,
        audio_source: Option<AudioHandle>,
    ) -> io::Result<Self> {
        Self::start_config(exe, dir, w, h, cw, ch, audio_source, true, 30)
    }
    pub fn start_config(
        exe: &str,
        dir: &Path,
        w: usize,
        h: usize,
        cw: usize,
        ch: usize,
        audio_source: Option<AudioHandle>,
        original_tracks: bool,
        fps: u32,
    ) -> io::Result<Self> {
        let created = Instant::now();
        if w < 2 || h < 2 || w % 2 != 0 || h % 2 != 0 || w > 3840 || h > 3840 || w * h > 3840 * 2400
        {
            return Err(io::Error::other("Invalid encoder dimensions"));
        }
        let codec = super::codec();
        #[cfg(feature = "desktop-ui")]
        let audio = audio_source.map(crate::audio::Session::new).transpose()?;
        #[cfg(feature = "desktop-ui")]
        let has_audio = audio.is_some();
        #[cfg(not(feature = "desktop-ui"))]
        let has_audio = false;

        let has_camera = cw > 0 && ch > 0;
        let packed_camera = has_camera && original_tracks;
        let packed_width = if packed_camera { w.max(cw) } else { w };
        let mut cmd = Command::new(exe);
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-filter_complex_threads",
            "1",
        ]);
        if codec == "h264_vaapi" {
            cmd.args([
                "-vaapi_device",
                &std::env::var("OPENRECORDER_GPU_DEVICE")
                    .unwrap_or_else(|_| "/dev/dri/renderD128".into()),
            ]);
        }
        cmd.args([
            "-thread_queue_size",
            "1",
            "-probesize",
            "32",
            "-analyzeduration",
            "1",
            "-fpsprobesize",
            "0",
            "-f",
            "rawvideo",
            "-pixel_format",
            "bgra",
            "-video_size",
            &format!(
                "{packed_width}x{}",
                if packed_camera { 2 * h + ch } else { h }
            ),
            "-framerate",
            &fps.to_string(),
            "-i",
            "pipe:0",
        ]);
        #[cfg(feature = "desktop-ui")]
        if let Some(session) = &audio {
            cmd.args([
                "-thread_queue_size",
                "1",
                "-probesize",
                "32",
                "-analyzeduration",
                "1",
                "-fpsprobesize",
                "0",
                "-f",
                "f32le",
                "-ar",
                &session.rate.to_string(),
                "-ac",
                "1",
                "-i",
                &format!("tcp://{}", session.address),
            ]);
        }
        let mut graph = if packed_camera {
            format!("[0:v]split=3[s0][c0][r0];[s0]crop={w}:{h}:0:0[screen];[c0]crop={cw}:{ch}:0:{h}[camera];[r0]crop={w}:{h}:0:{}[recording]",h+ch)
        } else {
            "[0:v]null[recording]".into()
        };
        let names: Vec<&str> = if packed_camera {
            vec!["screen", "camera", "recording"]
        } else {
            vec!["recording"]
        };
        for name in &names {
            if codec == "h264_vaapi" {
                graph.push_str(&format!(";[{name}]format=nv12,hwupload[{name}_out]"));
            } else {
                graph.push_str(&format!(";[{name}]format=yuv420p[{name}_out]"));
            }
        }
        if has_audio {
            if packed_camera {
                graph.push_str(";[1:a]asplit=2[screen_audio][recording_audio]");
            } else {
                graph.push_str(";[1:a]anull[recording_audio]");
            }
        }
        cmd.args(["-filter_complex", &graph]);
        for name in &names {
            cmd.args(["-map", &format!("[{name}_out]"), "-c:v", codec]);
            if has_audio && *name != "camera" {
                cmd.args([
                    "-map",
                    &format!("[{name}_audio]"),
                    "-c:a",
                    "aac",
                    "-b:a",
                    "128000",
                ]);
            } else {
                cmd.arg("-an");
            }
            if codec == "h264_vaapi" {
                cmd.args(["-async_depth", "1", "-bf", "0"]);
            }
            if codec == "h264_mf" {
                cmd.args(["-hw_encoding", "1"]);
            }
            if codec == "libx264" {
                cmd.args(["-preset", "ultrafast", "-threads", "1"]);
            } else {
                cmd.args(["-b:v", "6000000"]);
                if codec == "h264_videotoolbox" {
                    cmd.args(["-allow_sw", "0"]);
                }
            }
            cmd.args([
                "-g",
                &(fps * 2).to_string(),
                "-max_muxing_queue_size",
                "64",
                "-movflags",
                "+frag_keyframe+empty_moov+default_base_moof",
            ])
            .arg(dir.join(format!("{name}.mp4")));
        }
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("No encoder input"))?;
        let mut timing = Timing::new(created);
        timing.spawn = created.elapsed();
        let timing = Arc::new(Mutex::new(timing));
        let child = Arc::new(Mutex::new(child));
        let alive = Arc::new(AtomicBool::new(true));
        let heartbeat = Arc::new(Mutex::new(Instant::now()));
        let process = child.clone();
        let active = alive.clone();
        let last = heartbeat.clone();
        thread::spawn(move || {
            while active.load(Ordering::Relaxed) {
                if last
                    .lock()
                    .is_ok_and(|time| time.elapsed() > Duration::from_secs(5))
                {
                    if let Ok(mut child) = process.lock() {
                        let _ = child.kill();
                    }
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        let failure = Arc::new(Mutex::new(None));
        let pool: SharedPool = Arc::default();
        // Rendezvous hand-off: one frame is being written while the next is built, and the sampler waits for the
        // writer rather than queueing more (memory stays at two frames of owned buffers, whatever the encoder does).
        let (jobs, receiver) = mpsc::sync_channel(0);
        let writer = Writer::spawn(input, receiver, timing.clone(), heartbeat.clone(), failure.clone(), pool.clone())?;
        Ok(Self {
            layout: Layout::default(),
            #[cfg(feature = "desktop-ui")]
            audio,
            child,
            alive,
            heartbeat,
            jobs: Some(jobs),
            writer: Some(writer),
            failure,
            pool,
            held_screen: None,
            held_camera: None,
            packed_width,
            original_tracks: packed_camera,
            outputs: names
                .iter()
                .map(|name| dir.join(format!("{name}.mp4")))
                .collect(),
            executable: exe.into(),
            final_audio_diagnostics: None,
            codec,
            fps,
            has_audio,
            timing,
            timing_path: dir.join("encoder-timing.json"),
            width: w,
            height: h,
            camera_width: cw,
            camera_height: ch,
        })
    }
    /// Copying convenience for callers that own a plain buffer (synthetic modes, tests): the picture is kept
    /// until the next call, as the old packed frame kept it. The recorder's own loop uses `submit`.
    pub fn set_screen(&mut self, pixels: &[u8]) -> io::Result<()> {
        if pixels.len() != self.width * self.height * 4 {
            return Err(io::Error::other("Screen dimensions changed"));
        }
        self.held_screen = Some(Arc::new(pixels.to_vec()));
        Ok(())
    }
    pub fn set_camera(&mut self, pixels: &[u8]) -> io::Result<()> {
        if pixels.len() != self.camera_width * self.camera_height * 4 {
            return Err(io::Error::other("Camera dimensions changed"));
        }
        self.held_camera = Some(pixels.to_vec());
        Ok(())
    }
    pub fn write(&mut self) -> io::Result<()> {
        let screen = self.held_screen.clone().unwrap_or_else(|| Arc::new(vec![0; self.width * self.height * 4]));
        let camera = self.held_camera.take();
        let result = self.submit(&screen, camera.as_deref());
        self.held_camera = camera;
        result
    }
    /// One picture for the encoder. Builds only what differs from the captured screen (the camera band and the
    /// bubble's rows), then hands the frame to the writer thread. Blocks only while the writer is still busy with
    /// the previous frame, and records how long (that wait is the encoder's backpressure).
    pub fn submit(&mut self, screen: &Arc<Vec<u8>>, camera: Option<&[u8]>) -> io::Result<()> {
        let prepared = self.prepare(screen, camera)?;
        self.send(prepared)
    }
    /// The recorder's per-picture call when the camera frame is shared with its producer and the preview. The camera's
    /// lock is held only while the camera band and the bubble's rows are copied out of it; the wait for the writer (the
    /// encoder's backpressure) happens after it is released, so a stalled encoder can never hold the camera or its
    /// preview behind it. `check` runs under the lock on the frame being used (the liveness test).
    pub(crate) fn submit_shared_camera(
        &mut self,
        screen: &Arc<Vec<u8>>,
        camera: &Mutex<crate::camera_frame::Frame>,
        check: impl FnOnce(&crate::camera_frame::Frame) -> Result<(), Box<dyn std::error::Error>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let prepared = {
            let frame = camera.lock().map_err(|_| "Camera source poisoned")?;
            check(&frame)?;
            if (frame.width, frame.height) != (self.camera_width, self.camera_height) {
                return Err("Camera dimensions changed".into());
            }
            self.prepare(screen, Some(&frame.bgra))?
        };
        self.send(prepared)?;
        Ok(())
    }
    /// Validates and builds one picture. Takes no time that depends on the encoder.
    fn prepare(&self, screen: &Arc<Vec<u8>>, camera: Option<&[u8]>) -> io::Result<Prepared> {
        #[cfg(feature = "desktop-ui")]
        if let Some(audio) = &self.audio {
            audio.check()?;
        }
        if screen.len() != self.width * self.height * 4 {
            return Err(io::Error::other("Screen dimensions changed"));
        }
        if camera.is_some_and(|c| c.len() != self.camera_width * self.camera_height * 4) {
            return Err(io::Error::other("Camera dimensions changed"));
        }
        self.check_writer()?;
        let began = Instant::now();
        let job = self.build_job(screen, camera);
        Ok(Prepared { job, built: began.elapsed() })
    }
    /// Hands a built picture to the writer. Blocks only while the writer is still busy with the previous one.
    fn send(&mut self, Prepared { job, built: prepared }: Prepared) -> io::Result<()> {
        let jobs = self.jobs.as_ref().ok_or_else(|| io::Error::other("Encoder stopped"))?;
        let handoff = Instant::now();
        let sent = jobs.send(job);
        let waited = handoff.elapsed();
        self.timing.lock().unwrap_or_else(|e| e.into_inner()).record_sampler(prepared, waited);
        if sent.is_err() {
            self.check_writer()?;
            return Err(io::Error::other("Encoder stopped"));
        }
        Ok(())
    }
    fn check_writer(&self) -> io::Result<()> {
        match &*self.failure.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(message) => Err(io::Error::other(message.clone())),
            None => Ok(()),
        }
    }
    fn build_job(&self, screen: &Arc<Vec<u8>>, camera: Option<&[u8]>) -> Job {
        let (w, h, cw, ch) = (self.width, self.height, self.camera_width, self.camera_height);
        let row = self.packed_width * 4;
        // The screen's rows are exactly the packed rows unless the camera is wider than the screen.
        let direct = self.packed_width == w;
        let mut parts = Vec::with_capacity(5);
        if self.original_tracks {
            parts.push(if direct { Part::Shared(screen.clone(), 0..screen.len()) } else { self.padded_screen(screen) });
            let mut band = take_buffer(&self.pool, Kind::Camera, ch * row);
            if let Some(pixels) = camera {
                for y in 0..ch {
                    band[y * row..y * row + cw * 4].copy_from_slice(&pixels[y * cw * 4..(y + 1) * cw * 4]);
                }
            }
            parts.push(Part::Owned(band, Kind::Camera));
        }
        // No bubble (Show camera off): the recording is the screen as it is, exactly as when there is no camera. The
        // camera band above is untouched, so the camera original is still the raw camera.
        let bubble_pixels = camera.filter(|_| self.layout.visible);
        match bubble_pixels {
            None if direct => parts.push(Part::Shared(screen.clone(), 0..screen.len())),
            None => parts.push(self.padded_screen(screen)),
            Some(pixels) if direct => {
                let bubble = bubble_geometry((w, h), (cw, ch), &self.layout);
                let (top, bottom) = (bubble.y0 * row, (bubble.y0 + bubble.bh) * row);
                let mut patch = take_buffer(&self.pool, Kind::Patch, 0);
                patch.extend_from_slice(&screen[top..bottom]);
                draw_bubble(&mut patch, bubble.y0, self.packed_width, &bubble, pixels, (cw, ch), &self.layout);
                if top > 0 {
                    parts.push(Part::Shared(screen.clone(), 0..top));
                }
                parts.push(Part::Owned(patch, Kind::Patch));
                if bottom < screen.len() {
                    parts.push(Part::Shared(screen.clone(), bottom..screen.len()));
                }
            }
            Some(pixels) => {
                let Part::Owned(mut band, kind) = self.padded_screen(screen) else { unreachable!() };
                composite_camera(&mut band, self.packed_width, (w, h), pixels, (cw, ch), &self.layout);
                parts.push(Part::Owned(band, kind));
            }
        }
        Job { parts }
    }
    /// The screen with each row padded out to the packed width (only needed for a camera wider than the screen).
    fn padded_screen(&self, screen: &[u8]) -> Part {
        let row = self.packed_width * 4;
        let mut band = take_buffer(&self.pool, Kind::Padded, self.height * row);
        for y in 0..self.height {
            band[y * row..y * row + self.width * 4].copy_from_slice(&screen[y * self.width * 4..(y + 1) * self.width * 4]);
        }
        Part::Owned(band, Kind::Padded)
    }
    /// Which encoder writes which file, in words a take's metadata can carry. Every output uses the platform's one
    /// encoder, so `allHardware` is true exactly when that encoder is a hardware one.
    pub fn encoder_summary(&self) -> serde_json::Value {
        let codec = self.codec;
        let hardware = codec != "libx264";
        serde_json::json!({
            "encoders": if self.original_tracks {
                serde_json::json!({"screen":codec,"camera":codec,"recording":codec})
            } else {
                serde_json::json!({"recording":codec})
            },
            "allHardware": hardware,
            "anyHardware": hardware,
        })
    }
    /// Closes the hand-off and waits for the writer to finish what it holds. If it cannot (the encoder stopped
    /// reading), the encoder is killed so the blocked write fails; media already written stays as it is.
    fn drain_writer(&mut self, limit: Duration) -> io::Result<()> {
        self.jobs.take();
        let Some(mut writer) = self.writer.take() else { return Ok(()) };
        if writer.wait(limit) {
            return Ok(());
        }
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
        if writer.wait(Duration::from_secs(3)) {
            Err(io::Error::other("The encoder stopped accepting video; what was already written is kept"))
        } else {
            Err(io::Error::other("The encoder's input did not close"))
        }
    }
    fn remux(&self) -> io::Result<()> {
        for file in &self.outputs {
            let temp = file.with_extension("finalizing.mp4");
            let mut process = Command::new(&self.executable)
                .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                .arg(file)
                .args(["-map", "0", "-c", "copy", "-movflags", "+faststart"])
                .arg(&temp)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(30);
            let status = loop {
                if let Some(status) = process.try_wait()? {
                    break status;
                }
                if Instant::now() > deadline {
                    let _ = process.kill();
                    let _ = process.wait();
                    return Err(io::Error::other(
                        "Video finalization timed out; fragments preserved",
                    ));
                }
                thread::sleep(Duration::from_millis(20));
            };
            if !status.success() {
                return Err(io::Error::other(
                    "Final video could not be saved; recoverable fragments are preserved",
                ));
            }
            std::fs::rename(temp, file)?;
        }
        Ok(())
    }
    pub fn audio_diagnostics(&self) -> serde_json::Value {
        if let Some(value) = &self.final_audio_diagnostics {
            return value.clone();
        }
        #[cfg(feature = "desktop-ui")]
        if let Some(audio) = &self.audio {
            return serde_json::json!({"sampleRate":audio.rate,"droppedSamples":audio.dropped.load(Ordering::Relaxed)});
        }
        serde_json::Value::Null
    }
    // Best effort: a sidecar failure (disk full, read-only directory) must
    // never replace or mask the take's own error.
    fn write_timing(&mut self, audio_failed: bool, video_failed: bool) {
        let mut timing = self.timing.lock().unwrap_or_else(|e| e.into_inner());
        if timing.written {
            return;
        }
        timing.written = true;
        let outcome = if !timing.finish_called {
            "not_finished"
        } else if audio_failed || video_failed {
            "failed"
        } else {
            "completed"
        };
        let shape = serde_json::json!({"width":self.width,"height":self.height,
            "cameraWidth":self.camera_width,"cameraHeight":self.camera_height,
            "fps":self.fps,"originalTracks":self.original_tracks,"audio":self.has_audio,
            // Which encoder wrote the files, and what the encoder was asked for (megapixels per second).
            "encoders":{"screen":self.codec,"camera":self.codec,"recording":self.codec},
            "hardwareMegapixelsPerSecond":hardware_megapixels_per_second(self.width,self.height,self.camera_width,self.camera_height,self.fps,self.original_tracks)});
        let json = timing.to_json(self.codec, shape, outcome, audio_failed, video_failed);
        drop(timing);
        if let Ok(bytes) = serde_json::to_vec_pretty(&json) {
            let _ = std::fs::write(&self.timing_path, bytes);
        }
    }
    pub fn finish(&mut self) -> io::Result<()> {
        let finish_began = Instant::now();
        self.timing.lock().unwrap_or_else(|e| e.into_inner()).finish_called = true;
        // Every frame already handed over is written, then the input is closed so the encoder sees the end. The
        // watchdog stays up until then: a stuck encoder is killed here rather than leaving the writer blocked.
        let drained = self.drain_writer(Duration::from_secs(10));
        self.alive.store(false, Ordering::Relaxed);
        self.final_audio_diagnostics = Some(self.audio_diagnostics());
        let audio_began = Instant::now();
        let audio_result: io::Result<()> = {
            #[cfg(feature = "desktop-ui")]
            {
                if let Some(mut audio) = self.audio.take() {
                    let (tx, rx) = std::sync::mpsc::sync_channel(1);
                    thread::spawn(move || {
                        let result = audio.finish();
                        let diagnostics = serde_json::json!({"sampleRate":audio.rate,
                            "droppedSamples":audio.dropped.load(Ordering::Relaxed)});
                        let _ = tx.send((result, diagnostics));
                    });
                    match rx.recv_timeout(Duration::from_secs(3)) {
                        Ok((result, diagnostics)) => {
                            self.final_audio_diagnostics = Some(diagnostics);
                            result
                        }
                        Err(_) => Err(io::Error::other("Microphone finalization did not complete")),
                    }
                } else {
                    Ok(())
                }
            }
            #[cfg(not(feature = "desktop-ui"))]
            {
                Ok(())
            }
        };
        self.timing.lock().unwrap_or_else(|e| e.into_inner()).audio_finish = audio_began.elapsed();
        let (mut encoder_wait, mut remux_time) = (Duration::ZERO, Duration::ZERO);
        // An audio failure must not bypass closing and remuxing the video.
        let wait_began = Instant::now();
        let video_result = (|| -> io::Result<()> {
            let mut child = self
                .child
                .lock()
                .map_err(|_| io::Error::other("Encoder process lock poisoned"))?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait()? {
                    return if status.success() {
                        drop(child);
                        encoder_wait = wait_began.elapsed();
                        let remux_began = Instant::now();
                        let result = self.remux();
                        remux_time = remux_began.elapsed();
                        result
                    } else {
                        encoder_wait = wait_began.elapsed();
                        Err(io::Error::other(format!("Encoder failed: {status}")))
                    };
                }
                if std::time::Instant::now() > deadline {
                    encoder_wait = wait_began.elapsed();
                    child.kill()?;
                    child.wait()?;
                    return Err(io::Error::other("Encoder finalization timed out"));
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        })();
        {
            let mut timing = self.timing.lock().unwrap_or_else(|e| e.into_inner());
            timing.encoder_wait = encoder_wait;
            timing.remux = remux_time;
            timing.finish_total = finish_began.elapsed();
        }
        // The writer's own reason leads: a killed encoder's exit status is only its consequence.
        let drained = match (drained, self.check_writer()) {
            (Ok(()), Err(written)) => Err(io::Error::other(format!("Video could not be written to the encoder: {written}"))),
            (other, _) => other,
        };
        let video_result = match (drained, video_result) {
            (Err(drain), Err(video)) => Err(io::Error::other(format!("{drain}; {video}"))),
            (Err(error), Ok(())) => Err(error),
            (_, other) => other,
        };
        self.write_timing(audio_result.is_err(), video_result.is_err());
        match (audio_result, video_result) {
            (Err(audio), Err(video)) => Err(io::Error::other(format!("{audio}; {video}"))),
            (Err(error), _) | (_, Err(error)) => Err(error),
            _ => Ok(()),
        }
    }
}
impl Drop for Pipeline {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        self.jobs.take();
        if let Ok(mut child) = self.child.lock() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        // With the encoder gone the writer's pending write fails and it exits.
        if let Some(mut writer) = self.writer.take() {
            writer.wait(Duration::from_secs(3));
        }
        self.write_timing(false, false);
    }
}

#[cfg(test)]
mod camera_pipeline_tests {
    #[test]
    #[ignore = "requires explicitly supplied bundled helper and development ffprobe"]
    fn portrait_camera_original_survives_real_encoder() {
        use super::*;
        let helper = std::env::var("OPENRECORDER_FFMPEG").expect("Set bundled fixture helper");
        let probe = std::env::var("OPENRECORDER_TEST_FFPROBE").expect("Set development ffprobe");
        let dir =
            std::env::temp_dir().join(format!("recorder-portrait-codec-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let mut encoder =
            Pipeline::start_config(&helper, &dir, 640, 360, 144, 240, None, true, 30).unwrap();
        for n in 0..12 {
            let screen = [40u8, 50, 60, 255].repeat(640 * 360);
            let camera = [60u8 + n, 80, 100, 255].repeat(144 * 240);
            encoder.set_screen(&screen).unwrap();
            encoder.set_camera(&camera).unwrap();
            encoder.write().unwrap();
        }
        encoder.finish().unwrap();
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
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod camera_geometry_tests {
    use super::*;

    // Every camera pixel encodes its own column and row, so the composite
    // reveals exactly which source pixel each bubble pixel was taken from.
    fn gradient(cw: usize, ch: usize) -> Vec<u8> {
        (0..cw * ch)
            .flat_map(|n| [(n % cw) as u8, (n / cw) as u8, 7, 255])
            .collect()
    }
    fn layout(shape: u8, mirror: bool) -> Layout {
        Layout {
            x: 0.25,
            y: 0.25,
            size: 0.2,
            shape,
            mirror,
            visible: true,
        }
    }
    const SENTINEL: [u8; 4] = [1, 2, 3, 255];
    // Composite onto a SENTINEL screen and return the bubble's bounding box
    // plus the buffer. 500 x 300 at size 0.2 gives a 100 px base.
    fn render(shape: u8, mirror: bool, cw: usize, ch: usize) -> (Vec<u8>, [usize; 4]) {
        let (w, h) = (500, 300);
        let mut frame = SENTINEL.repeat(w * h);
        composite_camera(
            &mut frame,
            w,
            (w, h),
            &gradient(cw, ch),
            (cw, ch),
            &layout(shape, mirror),
        );
        let touched = |x: usize, y: usize| frame[(y * w + x) * 4..(y * w + x) * 4 + 4] != SENTINEL;
        let xs: Vec<usize> = (0..w).filter(|x| (0..h).any(|y| touched(*x, y))).collect();
        let ys: Vec<usize> = (0..h).filter(|y| (0..w).any(|x| touched(x, *y))).collect();
        let bbox = [xs[0], ys[0], xs[xs.len() - 1] + 1, ys[ys.len() - 1] + 1];
        (frame, bbox)
    }
    fn source_at(frame: &[u8], x: usize, y: usize) -> (i32, i32) {
        let i = (y * 500 + x) * 4;
        (frame[i] as i32, frame[i + 1] as i32)
    }

    #[test]
    fn a_small_size_fraction_is_not_lifted_to_a_floor() {
        // 120 pt on a 2560 pt display is 120 / 2560 of the frame. The old 0.08 floor
        // made it 205 px at 2560 wide. Here the frame is 1280 wide, so the base is 60 px.
        let (w, h) = (1280, 720);
        let mut frame = SENTINEL.repeat(w * h);
        let l = Layout { x: 0.1, y: 0.1, size: 120. / 2560., shape: 0, mirror: false, visible: true };
        composite_camera(&mut frame, w, (w, h), &gradient(200, 200), (200, 200), &l);
        let touched = |x: usize, y: usize| frame[(y * w + x) * 4..(y * w + x) * 4 + 4] != SENTINEL;
        let width = (0..w).filter(|x| (0..h).any(|y| touched(*x, y))).count();
        assert!((59..=61).contains(&width), "bubble is {width} px wide, expected 60");
        // The sanity bounds are still enforced for absurd input.
        let mut frame = SENTINEL.repeat(w * h);
        let tiny = Layout { size: 0.0, ..l };
        composite_camera(&mut frame, w, (w, h), &gradient(200, 200), (200, 200), &tiny);
        assert!((0..w).filter(|x| (0..h).any(|y| frame[(y * w + x) * 4..(y * w + x) * 4 + 4] != SENTINEL)).count() >= 2);
    }

    #[test]
    fn dimensions_follow_the_design_table() {
        for shape in 0..3 {
            assert_eq!(camera_dimensions(168., shape), (168., 168.));
        }
        for (base, w, h) in [(120., 160., 90.), (168., 224., 126.), (224., 298.67, 168.)] {
            let (cw, ch) = camera_dimensions(base, 3);
            assert!((cw - w).abs() < 0.01 && (ch - h).abs() < 0.01, "{base}");
            assert!((cw / ch - 16. / 9.).abs() < 1e-4);
        }
    }

    #[test]
    fn bubble_sizes_match_the_shape() {
        for cam in [(120, 240), (240, 120)] {
            for shape in 0..3 {
                let (_, b) = render(shape, false, cam.0, cam.1);
                assert_eq!((b[2] - b[0], b[3] - b[1]), (100, 100), "shape {shape}");
            }
            let (_, b) = render(3, false, cam.0, cam.1);
            assert_eq!((b[2] - b[0], b[3] - b[1]), (133, 75));
            assert_eq!((b[0], b[1]), (125, 75));
        }
    }

    #[test]
    fn cover_crop_is_uniform_centred_and_never_padded() {
        // (camera size, shape): portrait and landscape into square and wide.
        for (cam, shape) in [
            ((120, 240), 1),
            ((240, 120), 1),
            ((120, 240), 3),
            ((240, 120), 3),
            ((240, 120), 0),
        ] {
            let (frame, b) = render(shape, false, cam.0, cam.1);
            let (bw, bh) = ((b[2] - b[0]) as i32, (b[3] - b[1]) as i32);
            let (mid_x, mid_y) = ((b[0] + b[2]) / 2, (b[1] + b[3]) / 2);
            let left = source_at(&frame, b[0], mid_y);
            let right = source_at(&frame, b[2] - 1, mid_y);
            let top = source_at(&frame, mid_x, b[1]);
            let bottom = source_at(&frame, mid_x, b[3] - 1);
            // One scale for both axes: source span per bubble pixel agrees.
            let per_x = (right.0 - left.0) as f32 / (bw - 1) as f32;
            let per_y = (bottom.1 - top.1) as f32 / (bh - 1) as f32;
            assert!(
                (per_x - per_y).abs() < 0.03,
                "{cam:?} shape {shape}: stretched {per_x} vs {per_y}"
            );
            // Cover: the tighter camera axis is used edge to edge.
            if (cam.0 as f32 / bw as f32) < (cam.1 as f32 / bh as f32) {
                assert!(
                    left.0 <= 1 && right.0 >= cam.0 as i32 - 2,
                    "{cam:?} {shape}"
                );
            } else {
                assert!(
                    top.1 <= 1 && bottom.1 >= cam.1 as i32 - 2,
                    "{cam:?} {shape}"
                );
            }
            // Centred: the bubble centre samples the camera centre.
            let centre = source_at(&frame, mid_x, mid_y);
            assert!((centre.0 - cam.0 as i32 / 2).abs() <= 2, "{cam:?} {shape}");
            assert!((centre.1 - cam.1 as i32 / 2).abs() <= 2, "{cam:?} {shape}");
        }
    }

    #[test]
    fn mirror_flips_only_when_the_layout_asks() {
        let (plain, b) = render(2, false, 240, 120);
        let (mirrored, _) = render(2, true, 240, 120);
        let y = (b[1] + b[3]) / 2;
        let (a, m) = (source_at(&plain, b[0], y), source_at(&mirrored, b[0], y));
        // A 240 x 120 camera in a square bubble shows source columns 60..180.
        assert!(
            (a.0 - 60).abs() <= 2 && (m.0 - 179).abs() <= 2,
            "{a:?} {m:?}"
        );
        assert_eq!(a.1, m.1);
        assert!(source_at(&plain, b[2] - 1, y).0 > source_at(&plain, b[0], y).0);
        assert!(source_at(&mirrored, b[2] - 1, y).0 < source_at(&mirrored, b[0], y).0);
    }

    #[test]
    fn portrait_is_the_nine_by_sixteen_turn_of_wide_and_masks_like_it() {
        let (w, h) = camera_dimensions(120., 4);
        assert!((w - 90.).abs() < 1e-4 && (h - 160.).abs() < 1e-4, "{w}x{h}");
        let (ww, wh) = camera_dimensions(120., 3);
        assert!((ww - h).abs() < 1e-4 && (wh - w).abs() < 1e-4, "wide and portrait are transposes");
        assert_eq!(camera_dimensions(120., 0), (120., 120.));
        assert!(!bubble_contains(4, 90, 160, 0, 0), "corner is cut");
        assert!(bubble_contains(4, 90, 160, 45, 80) && bubble_contains(4, 90, 160, 45, 2));
        assert!(!bubble_contains(4, 90, 160, 89, 159));
    }
    #[test]
    fn masks_cut_the_right_corners() {
        let inside = |frame: &[u8], x: usize, y: usize| {
            frame[(y * 500 + x) * 4..(y * 500 + x) * 4 + 4] != SENTINEL
        };
        // Bubble top-left is (125, 75).
        let (circle, b) = render(0, false, 120, 240);
        assert!(!inside(&circle, b[0] + 14, b[1] + 14));
        assert!(inside(&circle, b[0] + 15, b[1] + 15));
        assert!(inside(&circle, b[0], b[1] + 50) && inside(&circle, b[0] + 50, b[1]));
        // 22% of 100 px short side.
        let (rounded, b) = render(1, false, 120, 240);
        assert!(!inside(&rounded, b[0] + 3, b[1] + 3));
        assert!(inside(&rounded, b[0] + 7, b[1] + 7));
        assert!(inside(&rounded, b[0] + 50, b[1]) && inside(&rounded, b[0], b[1] + 50));
        // Legacy square has no mask at all.
        let (square, b) = render(2, false, 120, 240);
        assert!(inside(&square, b[0], b[1]) && inside(&square, b[2] - 1, b[3] - 1));
        // Wide: 20% of its 75 px height is a 15 px corner radius.
        let (wide, b) = render(3, false, 240, 120);
        assert!(!inside(&wide, b[0], b[1]) && !inside(&wide, b[0] + 2, b[1] + 2));
        assert!(inside(&wide, b[0] + 5, b[1] + 5));
        assert!(inside(&wide, b[0], b[1] + 37) && inside(&wide, b[0] + 66, b[1]));
    }

    #[test]
    fn packed_originals_are_untouched_and_only_recording_gets_the_bubble() {
        // Narrow and wider-than-screen cameras both exercise the stride.
        for (cw, ch) in [(30, 50), (80, 20)] {
            let (w, h) = (64, 40);
            let stride = w.max(cw);
            let screen: Vec<u8> = (0..w * h)
                .flat_map(|n| [(n % 251) as u8, (n / 7 % 251) as u8, 9, 255])
                .collect();
            let camera = gradient(cw, ch);
            let mut frame = vec![0u8; stride * (2 * h + ch) * 4];
            for y in 0..h {
                frame[y * stride * 4..(y * stride + w) * 4]
                    .copy_from_slice(&screen[y * w * 4..(y + 1) * w * 4]);
            }
            let layout = Layout {
                x: 0.5,
                y: 0.5,
                size: 0.4,
                shape: 3,
                mirror: true,
                visible: true,
            };
            place_camera(&mut frame, stride, (w, h), &camera, (cw, ch), true, &layout);
            for y in 0..h {
                let row =
                    |band: usize| &frame[((band + y) * stride) * 4..((band + y) * stride + w) * 4];
                assert_eq!(
                    row(0),
                    &screen[y * w * 4..(y + 1) * w * 4],
                    "screen row {y}"
                );
            }
            for y in 0..ch {
                let start = (h + y) * stride * 4;
                assert_eq!(
                    &frame[start..start + cw * 4],
                    &camera[y * cw * 4..(y + 1) * cw * 4]
                );
            }
            let (mut changed, mut min, mut max) = (0, (w, h), (0, 0));
            for y in 0..h {
                for x in 0..w {
                    let i = ((h + ch + y) * stride + x) * 4;
                    if frame[i..i + 4] != screen[(y * w + x) * 4..(y * w + x) * 4 + 4] {
                        changed += 1;
                        min = (min.0.min(x), min.1.min(y));
                        max = (max.0.max(x + 1), max.1.max(y + 1));
                    }
                }
            }
            // 0.4 of 64 px is a 25.6 px base: a 34 x 19 wide bubble clamped to (30, 20)
            // with rounded corners, and nothing changed outside it.
            assert_eq!((min, max), ((30, 20), (64, 39)), "{cw}x{ch}");
            assert!((600..34 * 19).contains(&changed), "{cw}x{ch}: {changed}");
        }
    }

    #[test]
    fn every_shape_stays_inside_small_and_extreme_frames() {
        for (w, h) in [(2, 2), (4, 4), (8, 4), (4, 8), (100, 20), (20, 100)] {
            for shape in 0..=4u8 {
                for (x, y) in [(0., 0.), (1., 1.), (0.5, 0.5)] {
                    for size in [0., 0.2, 1.] {
                        let layout = Layout {
                            visible: true,
                            x,
                            y,
                            size,
                            shape,
                            mirror: shape % 2 == 0,
                        };
                        for (cw, ch) in [(2, 2), (3, 7), (9, 3)] {
                            let mut frame = vec![0u8; w * h * 4];
                            composite_camera(
                                &mut frame,
                                w,
                                (w, h),
                                &gradient(cw, ch),
                                (cw, ch),
                                &layout,
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn timing_buckets_are_bounded_and_failed_writes_counted() {
        let start = Instant::now();
        let mut timing = Timing::new(start);
        for n in 0..1000u64 {
            timing.record_write(start, Duration::from_millis(n % 900), n != 3);
        }
        assert_eq!(timing.writes, 1000);
        assert_eq!(timing.write_failures, 1);
        assert_eq!(timing.buckets.iter().sum::<u64>(), 1000);
        assert_eq!(timing.early[7], Duration::from_millis(7));
        assert_eq!(timing.max_write, Duration::from_millis(899));
        let json = timing.to_json("codec", serde_json::json!({}), "failed", false, true);
        assert_eq!(
            json["writes"]["firstWritesMs"].as_array().unwrap().len(),
            EARLY_WRITES
        );
        assert_eq!(json["finalization"]["videoFailed"], true);
        assert!(serde_json::to_vec(&json).unwrap().len() < 2048);
    }
}

/// The encoder hand-off, the frame bands the helper reads, and which encoder writes which file. The real helper is
/// replaced by a tiny script that reads (or refuses to read) the pipe, so these run anywhere with a POSIX shell and
/// need no media tools. The real-helper workload is `throughput_bench` (ignored).
#[cfg(all(test, unix))]
mod throughput_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in for the helper: `body` runs for the encode; a re-mux call (no stdin) just creates its output.
    fn fake_helper(name: &str, body: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("openrecorder-pipeline-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("helper.sh");
        std::fs::write(&exe, format!("#!/bin/sh\nfor a; do last=\"$a\"; done\n: > \"$last\"\n{body}\n")).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        (exe, dir)
    }
    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed).wrapping_add((i >> 8) as u8)).collect()
    }
    /// What the encoder was fed before the hand-off existed: the packed frame, assembled whole.
    fn reference_frame(screen: &[u8], camera: Option<&[u8]>, (w, h): (usize, usize), (cw, ch): (usize, usize), originals: bool, layout: &Layout) -> Vec<u8> {
        let has_camera = cw > 0 && ch > 0;
        let packed = has_camera && originals;
        let pw = if packed { w.max(cw) } else { w };
        let mut frame = vec![0u8; pw * if packed { 2 * h + ch } else { h } * 4];
        for y in 0..h {
            frame[y * pw * 4..y * pw * 4 + w * 4].copy_from_slice(&screen[y * w * 4..(y + 1) * w * 4]);
        }
        if let Some(camera) = camera {
            place_camera(&mut frame, pw, (w, h), camera, (cw, ch), packed, layout);
        }
        frame
    }
    fn job_bytes(job: &Job) -> Vec<u8> {
        job.parts.iter().flat_map(|p| p.bytes().iter().copied()).collect()
    }

    #[test]
    fn the_bands_written_to_the_encoder_are_byte_for_byte_the_old_packed_frame() {
        let (exe, dir) = fake_helper("golden", "cat >/dev/null");
        let layouts = [
            Layout::default(),
            Layout { x: 0., y: 0., size: 0.5, shape: 0, mirror: false, visible: true },   // top-left corner, circle, unmirrored
            Layout { x: 1., y: 1., size: 0.2, shape: 3, mirror: true, visible: true },    // bottom-right edge, wide
            Layout { x: 0.4, y: 0.9, size: 0.3, shape: 4, mirror: false, visible: true }, // portrait near the bottom
            Layout { x: 0.5, y: 0.5, size: 0.01, shape: 2, mirror: true, visible: true }, // smallest, square
            Layout { x: 0.25, y: 0.25, size: 0.2, shape: 0, mirror: false, visible: false }, // Show camera off: no bubble anywhere
        ];
        // (screen, camera, originals): camera inside the screen's width, camera wider than the screen, no originals.
        for ((w, h), (cw, ch), originals) in [((64, 48), (20, 16), true), ((32, 24), (48, 16), true), ((64, 48), (20, 16), false), ((64, 48), (0, 0), true), ((62, 46), (22, 14), true)] {
            let mut pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, w, h, cw, ch, None, originals, 30).unwrap();
            for (n, layout) in layouts.iter().enumerate() {
                let screen = Arc::new(pattern(w * h * 4, n as u8));
                let camera = (cw > 0).then(|| pattern(cw * ch * 4, 100 + n as u8));
                pipeline.layout = *layout;
                // Twice, so buffers coming back from the pool are known not to leak the previous frame.
                for round in 0..2 {
                    let job = pipeline.build_job(&screen, camera.as_deref());
                    let expected = reference_frame(&screen, camera.as_deref(), (w, h), (cw, ch), originals, layout);
                    let got = job_bytes(&job);
                    assert_eq!(got.len(), expected.len(), "{w}x{h} cam {cw}x{ch} originals {originals} layout {n}");
                    assert!(got == expected, "{w}x{h} cam {cw}x{ch} originals {originals} layout {n} round {round}: the bytes differ");
                    for part in job.parts {
                        if let Part::Owned(buffer, kind) = part {
                            give_buffer(&pipeline.pool, kind, buffer);
                        }
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn show_camera_off_draws_no_bubble_in_the_recording_and_leaves_the_camera_original_raw_on_the_real_job_path() {
        let (exe, dir) = fake_helper("hidden", "cat >/dev/null");
        // Direct (camera narrower than the screen) and padded (camera wider) layouts, with and without originals.
        for ((w, h), (cw, ch), originals) in [((64, 48), (20, 16), true), ((32, 24), (48, 16), true), ((64, 48), (20, 16), false)] {
            let mut pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, w, h, cw, ch, None, originals, 30).unwrap();
            let screen = Arc::new(pattern(w * h * 4, 21));
            let camera = pattern(cw * ch * 4, 22);
            pipeline.layout = Layout { x: 0.25, y: 0.25, size: 0.3, shape: 0, mirror: false, visible: false };
            let hidden = job_bytes(&pipeline.build_job(&screen, Some(&camera)));
            let pw = pipeline.packed_width;
            // Hidden is, byte for byte, what a take with no camera produces for the recording...
            pipeline.layout.visible = true;
            let shown = job_bytes(&pipeline.build_job(&screen, Some(&camera)));
            assert_ne!(hidden, shown, "{w}x{h}: the bubble really is the difference");
            let row = pw * 4;
            let band = |frame: &[u8], from: usize, rows: usize| frame[from * row..(from + rows) * row].to_vec();
            let padded_screen = {
                let mut v = vec![0u8; h * row];
                for y in 0..h {
                    v[y * row..y * row + w * 4].copy_from_slice(&screen[y * w * 4..(y + 1) * w * 4]);
                }
                v
            };
            if originals {
                assert_eq!(band(&hidden, 0, h), padded_screen, "screen original");
                let camera_band = band(&hidden, h, ch);
                for y in 0..ch {
                    assert_eq!(&camera_band[y * row..y * row + cw * 4], &camera[y * cw * 4..(y + 1) * cw * 4], "camera original row {y} is the raw camera");
                }
                assert_eq!(band(&hidden, h + ch, h), padded_screen, "the recording has no bubble");
                assert_eq!(band(&shown, 0, h + ch), band(&hidden, 0, h + ch), "showing the bubble changes only the recording");
            } else {
                assert_eq!(hidden, padded_screen, "no originals: the recording is the screen");
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn a_stalled_encoder_never_holds_the_camera_producer_or_preview_behind_it() {
        // The helper reads nothing for a while, so the writer blocks and the sampler waits for it at the hand-off.
        let (exe, dir) = fake_helper("camera-lock", "sleep 2.5; cat >/dev/null");
        let (w, h, cw, ch) = (640, 480, 160, 120);
        let mut pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, w, h, cw, ch, None, true, 30).unwrap();
        let camera = Arc::new(Mutex::new(crate::camera_frame::Frame { width: cw, height: ch, bgra: pattern(cw * ch * 4, 5), sequence: 1 }));
        let screen = Arc::new(pattern(w * h * 4, 6));
        let started = Arc::new(AtomicBool::new(false));
        let sampler = {
            let (camera, started) = (camera.clone(), started.clone());
            thread::spawn(move || {
                let began = Instant::now();
                for _ in 0..3 {
                    started.store(true, Ordering::SeqCst);
                    pipeline.submit_shared_camera(&screen, &camera, |_| Ok(())).unwrap();
                }
                (began.elapsed(), pipeline)
            })
        };
        while !started.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(5));
        }
        // While the sampler is waiting for the blocked writer, the camera producer (and the preview) must be able to
        // take the lock, publish a new frame and read the old one, every time, without waiting.
        let window = Instant::now();
        let (mut grabbed, mut longest) = (0, Duration::ZERO);
        thread::sleep(Duration::from_millis(250));
        while window.elapsed() < Duration::from_millis(900) {
            let asked = Instant::now();
            let mut frame = camera.lock().unwrap();
            longest = longest.max(asked.elapsed());
            frame.sequence += 1;
            drop(frame);
            grabbed += 1;
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!sampler.is_finished(), "the sampler really was blocked behind the encoder during the check");
        assert!(grabbed > 20 && longest < Duration::from_millis(100), "the camera lock was held behind the encoder: {grabbed} grabs, longest wait {longest:?}");
        let (elapsed, mut pipeline) = sampler.join().unwrap();
        assert!(elapsed > Duration::from_millis(900), "and the hand-off did wait for the writer: {elapsed:?}");
        let _ = pipeline.finish();
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn the_screen_band_is_the_captures_own_buffer_and_only_the_bubbles_rows_are_copied() {
        let (exe, dir) = fake_helper("zero-copy", "cat >/dev/null");
        let (w, h, cw, ch) = (64, 48, 20, 16);
        let mut pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, w, h, cw, ch, None, true, 30).unwrap();
        pipeline.layout = Layout { x: 0.25, y: 0.5, size: 0.25, shape: 0, mirror: false, visible: true };
        let screen = Arc::new(pattern(w * h * 4, 7));
        let camera = pattern(cw * ch * 4, 9);
        let job = pipeline.build_job(&screen, Some(&camera));
        let owned: usize = job.parts.iter().map(|p| if let Part::Owned(b, _) = p { b.len() } else { 0 }).sum();
        let bubble = bubble_geometry((w, h), (cw, ch), &pipeline.layout);
        assert!(matches!(&job.parts[0], Part::Shared(buffer, range) if Arc::ptr_eq(buffer, &screen) && *range == (0..screen.len())), "the screen band is not copied");
        assert_eq!(owned, ch * w * 4 + bubble.bh * w * 4, "owned bytes are the camera band and the bubble's rows, nothing else");
        assert!(owned < screen.len(), "far less than a screen");
        // With no camera at all, nothing is built: the capture's buffer is the whole frame.
        let (exe2, dir2) = fake_helper("zero-copy-screen-only", "cat >/dev/null");
        let plain = Pipeline::start_config(exe2.to_str().unwrap(), &dir2, w, h, 0, 0, None, true, 30).unwrap();
        let job = plain.build_job(&screen, None);
        assert!(job.parts.len() == 1 && matches!(&job.parts[0], Part::Shared(b, r) if Arc::ptr_eq(b, &screen) && *r == (0..screen.len())));
        let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(dir2));
    }

    #[test]
    fn frames_go_out_in_order_and_finish_drains_everything_already_handed_over() {
        let (exe, dir) = fake_helper("drain", "cat >/dev/null");
        let mut pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, 64, 48, 20, 16, None, true, 30).unwrap();
        let screen = Arc::new(pattern(64 * 48 * 4, 1));
        let camera = pattern(20 * 16 * 4, 2);
        for _ in 0..25 {
            pipeline.submit(&screen, Some(&camera)).unwrap();
        }
        pipeline.finish().unwrap();
        let timing: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("encoder-timing.json")).unwrap()).unwrap();
        assert_eq!((timing["writes"]["count"].as_u64(), timing["writes"]["failed"].as_u64()), (Some(25), Some(0)), "every submitted frame was written before the input closed");
        assert_eq!(timing["sampler"]["frames"], 25);
        assert_eq!(timing["outcome"], "completed");
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn a_slow_encoder_makes_the_sampler_wait_one_frame_deep_and_the_wait_is_reported() {
        // The helper does not read for 700 ms. Frames are far larger than a pipe buffer, so the writer blocks.
        let (exe, dir) = fake_helper("backpressure", "sleep 0.7; cat >/dev/null");
        let (w, h) = (640, 480);
        let mut pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, w, h, 0, 0, None, true, 30).unwrap();
        let screen = Arc::new(pattern(w * h * 4, 3));
        let started = Instant::now();
        pipeline.submit(&screen, None).unwrap(); // taken by the writer at once: it now blocks writing
        let first = started.elapsed();
        pipeline.submit(&screen, None).unwrap(); // must wait until the writer is free again
        let second = started.elapsed();
        assert!(first < Duration::from_millis(300), "the first hand-off does not wait: {first:?}");
        assert!(second > Duration::from_millis(400), "the second waits for the blocked writer (bounded, not queued): {second:?}");
        assert!(second < Duration::from_secs(5));
        pipeline.finish().unwrap();
        let timing: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("encoder-timing.json")).unwrap()).unwrap();
        assert!(timing["sampler"]["handoffBlockedFrames"].as_u64().unwrap() >= 1);
        assert!(timing["sampler"]["handoffWaitMsMax"].as_f64().unwrap() > 300.0, "{timing}");
        assert_eq!(timing["writes"]["count"], 2);
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn an_encoder_that_stops_reading_fails_the_next_submit_with_its_reason_and_finish_stays_bounded() {
        let (exe, dir) = fake_helper("dies", "exit 0");
        let (w, h) = (640, 480);
        let mut pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, w, h, 0, 0, None, true, 30).unwrap();
        let screen = Arc::new(pattern(w * h * 4, 4));
        let began = Instant::now();
        let error = (0..200).find_map(|_| pipeline.submit(&screen, None).err()).expect("a closed pipe is reported to the sampler");
        assert!(began.elapsed() < Duration::from_secs(10));
        assert!(!error.to_string().is_empty(), "{error}");
        // It keeps failing, quickly, and never blocks.
        let again = Instant::now();
        assert!(pipeline.submit(&screen, None).is_err());
        assert!(again.elapsed() < Duration::from_secs(1));
        let finish = Instant::now();
        let finished = pipeline.finish();
        assert!(finish.elapsed() < Duration::from_secs(15), "finish is bounded");
        let timing: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("encoder-timing.json")).unwrap()).unwrap();
        assert!(timing["writes"]["failed"].as_u64().unwrap() >= 1, "the failed write is counted: {timing}");
        let _ = (finished, std::fs::remove_dir_all(dir));
    }
    #[test]
    fn dropping_a_pipeline_whose_encoder_is_stuck_does_not_hang() {
        // The helper never reads. A cancel (the pipeline dropped mid-take) must still return promptly.
        let (exe, dir) = fake_helper("stuck", "exec sleep 30");
        let (w, h) = (640, 480);
        let mut pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, w, h, 0, 0, None, true, 30).unwrap();
        let screen = Arc::new(pattern(w * h * 4, 5));
        pipeline.submit(&screen, None).unwrap(); // the writer is now blocked in write
        let began = Instant::now();
        drop(pipeline);
        assert!(began.elapsed() < Duration::from_secs(8), "drop killed the encoder and the writer ended: {:?}", began.elapsed());
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn finishing_with_a_stuck_encoder_kills_it_and_reports_instead_of_waiting_forever() {
        let (exe, dir) = fake_helper("stuck-finish", "exec sleep 30");
        let (w, h) = (640, 480);
        let mut pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, w, h, 0, 0, None, true, 30).unwrap();
        let screen = Arc::new(pattern(w * h * 4, 6));
        pipeline.submit(&screen, None).unwrap();
        let began = Instant::now();
        let result = pipeline.finish();
        assert!(began.elapsed() < Duration::from_secs(20), "bounded: {:?}", began.elapsed());
        let error = result.expect_err("a take whose encoder stopped accepting video does not finish cleanly").to_string();
        // Either the watchdog (no frame written for 5 s) or the drain's own kill ended it; both say why.
        assert!(error.contains("stopped accepting video") || error.contains("could not be written to the encoder"), "{error}");
        assert!(!error.starts_with("Encoder failed"), "the cause leads, the exit status follows: {error}");
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn a_picture_of_the_wrong_size_is_refused_before_anything_is_built_or_sent() {
        let (exe, dir) = fake_helper("sizes", "cat >/dev/null");
        let mut pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, 64, 48, 20, 16, None, true, 30).unwrap();
        let wrong = Arc::new(vec![0u8; 64 * 48 * 4 - 4]);
        assert_eq!(pipeline.submit(&wrong, None).unwrap_err().to_string(), "Screen dimensions changed");
        let screen = Arc::new(pattern(64 * 48 * 4, 1));
        assert_eq!(pipeline.submit(&screen, Some(&[0u8; 8])).unwrap_err().to_string(), "Camera dimensions changed");
        let timing = pipeline.timing.lock().unwrap();
        assert_eq!(timing.frames, 0, "nothing reached the hand-off");
        drop(timing);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_output_uses_the_platforms_one_encoder_and_the_summary_says_so() {
        let (exe, dir) = fake_helper("summary", "cat >/dev/null");
        let pipeline = Pipeline::start_config(exe.to_str().unwrap(), &dir, 64, 48, 20, 16, None, true, 30).unwrap();
        let summary = pipeline.encoder_summary();
        let encoders = summary["encoders"].as_object().unwrap();
        assert_eq!(encoders.len(), 3, "one entry per output file");
        assert!(encoders.values().all(|v| v == pipeline.codec), "no output is moved to another encoder");
        assert_eq!(summary["allHardware"].as_bool().unwrap(), pipeline.codec != "libx264");
        assert_eq!(summary["anyHardware"], summary["allHardware"]);
        assert!(summary.get("offload").is_none());
        let single = Pipeline::start_config(exe.to_str().unwrap(), &dir, 64, 48, 0, 0, None, true, 30).unwrap();
        assert_eq!(single.encoder_summary()["encoders"].as_object().unwrap().keys().collect::<Vec<_>>(), ["recording"]);
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn a_heavy_take_is_never_moved_off_the_hardware_encoder_by_a_budget_rule() {
        // The owner's 2560x1440 / 60 fps take with a camera and originals: its demand is a diagnostic, not a switch.
        let rate = hardware_megapixels_per_second(2560, 1440, 900, 720, 60, true);
        assert!((rate - 481.0).abs() < 1.0, "{rate}");
        // The argument builder gives every output the platform's encoder with the bitrate and options as before.
        let source = include_str!("pipeline.rs").split("#[cfg(all(test, unix))]").next().unwrap();
        for needle in [
            "cmd.args([\"-map\", &format!(\"[{name}_out]\"), \"-c:v\", codec]);",
            "[0:v]split=3[s0][c0][r0];[s0]crop={w}:{h}:0:0[screen];[c0]crop={cw}:{ch}:0:{h}[camera];[r0]crop={w}:{h}:0:{}[recording]",
            "\"-movflags\",\n                \"+frag_keyframe+empty_moov+default_base_moof\",",
        ] {
            assert!(source.contains(needle), "{needle}");
        }
        for gone in ["libx264\" } else { offload", "offload_threads", "HARDWARE_BUDGET", "P9_HW_BUDGET", "plan_encoders", "P9_NO_REALTIME"] {
            assert!(!source.contains(gone), "{gone} must not ship");
        }
    }
}
