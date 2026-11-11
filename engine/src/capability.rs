//! What this machine has been shown to sustain, kept apart from what a display or a setting merely claims.
//!
//! Sixty frames per second is a request until something measured says otherwise. Four states are
//! distinguished and never merged:
//!   unknown      nothing measured yet for *this* workload; the default, never a reason to refuse or hide 60
//!   qualified    a completed take of exactly this workload delivered the rate
//!   failed-last  the latest take of exactly this workload missed it; 60 stays selectable, with a plain hint
//!   unavailable  (encoder check only) the encoder itself refused those settings
//!
//! **Evidence applies only to the workload that produced it.** A workload is the rate, the size bucket, the
//! codec, the source (a particular display, or a window), the screen's captured pixel size, the camera and
//! its frame size, whether a microphone is encoded and whether originals are written. A take qualifies only
//! that exact combination. The synthetic encoder check is a different kind of evidence: it exercises the
//! encoder alone (no screen, camera, audio or originals) and is kept in its own record that is *shown* but
//! never used to promote a default. Nothing is guessed: if the geometry or camera size of the planned take
//! has not been observed before, the workload cannot match any evidence and the answer is `unknown`.
//!
//! Choosing a rate is never blocked by any of this, so there is no loop in which 60 is refused until a 60
//! take qualifies. The synthetic check is the only code here that costs CPU, runs only when a person asks for
//! it, and opens no screen, camera or microphone.
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path, time::Duration};

/// A take counts as delivering its rate when frames written over loop time reach this share of it.
const DELIVERED_SHARE: f64 = 0.97;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Unknown,
    Qualified,
    FailedLast,
    Unavailable,
}
impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Qualified => "qualified",
            Self::FailedLast => "failed-last",
            Self::Unavailable => "unavailable",
        }
    }
    fn parse(name: &str) -> Option<Self> {
        [Self::Unknown, Self::Qualified, Self::FailedLast, Self::Unavailable].into_iter().find(|s| s.name() == name)
    }
}

/// Output sizes the recorder offers, as the long edge of a 16:9 frame.
pub fn check_dimensions(quality: u32) -> Option<(usize, usize)> {
    match quality {
        1280 => Some((1280, 720)),
        1920 => Some((1920, 1080)),
        2560 => Some((2560, 1440)),
        _ => None,
    }
}

/// What the camera contributes to a workload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CameraPlan {
    Off,
    /// On, with the frame size it is known to deliver.
    Size(u32, u32),
    /// On, but its frame size has not been observed.
    Unobserved,
}

/// Everything that decides how heavy a take is for this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workload {
    pub fps: u32,
    pub quality: u32,
    pub codec: String,
    /// "display:<id>" (the system's id for it, including the main display when the default is used), "window", or
    /// "unresolved" when the actual source is not known (a portal that chooses per take, an unreported main display).
    pub source: String,
    /// The screen's captured pixel size, when it has been observed for this source.
    pub geometry: Option<(u32, u32)>,
    pub camera: CameraPlan,
    pub audio: bool,
    pub originals: bool,
}
impl Workload {
    /// Whether every part is known. A window changes size under the recorder, and an unresolved source could be
    /// any display or window, so neither is ever exact: their evidence is never reused.
    pub fn is_exact(&self) -> bool {
        Self::stable_source(&self.source) && self.geometry.is_some() && self.camera != CameraPlan::Unobserved
    }
    /// Only a real display id names something that stays the same between takes.
    fn stable_source(source: &str) -> bool {
        source.strip_prefix("display:").is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
    }
    fn key(&self) -> Option<String> {
        if !self.is_exact() {
            return None;
        }
        let (w, h) = self.geometry?;
        let camera = match &self.camera {
            CameraPlan::Off => "off".to_owned(),
            CameraPlan::Size(cw, ch) => format!("{cw}x{ch}"),
            CameraPlan::Unobserved => return None,
        };
        Some(format!(
            "{}@{}|{}|{}|{w}x{h}|cam:{camera}|audio:{}|originals:{}",
            self.fps, self.quality, self.codec, self.source, self.audio as u8, self.originals as u8
        ))
    }
    /// The workload a finished take actually ran, read from its capture.json. Takes written before the
    /// workload was recorded, windows, and takes without a measured size say nothing and return None.
    pub fn from_take(capture: &Value, source: &str, codec: &str) -> Option<Self> {
        let fps = capture["fps"].as_u64().filter(|n| [24, 30, 60].contains(n))? as u32;
        let info = capture["workload"].as_object()?;
        // The capture says which display it was told to record; a result is attributed to that one only.
        if info.get("source")?.as_str()? != source {
            return None;
        }
        let quality = info.get("quality")?.as_u64().filter(|n| [1280, 1920, 2560].contains(n))? as u32;
        let (w, h) = (capture["width"].as_u64()? as u32, capture["height"].as_u64()? as u32);
        if w < 2 || h < 2 {
            return None;
        }
        let camera = if capture["camera"].as_bool()? {
            let (cw, ch) = (capture["cameraWidth"].as_u64()? as u32, capture["cameraHeight"].as_u64()? as u32);
            if cw < 2 || ch < 2 {
                return None;
            }
            CameraPlan::Size(cw, ch)
        } else {
            CameraPlan::Off
        };
        Some(Self {
            fps,
            quality,
            codec: codec.to_owned(),
            source: source.to_owned(),
            geometry: Some((w, h)),
            camera,
            audio: info.get("audio")?.as_bool()?,
            originals: info.get("originals")?.as_bool()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub status: Status,
    /// "take" or "check": what produced the evidence.
    pub basis: String,
    pub codec: String,
    pub at: u64,
    pub measured_fps: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Capabilities {
    /// Evidence from real takes, by exact workload.
    entries: BTreeMap<String, Entry>,
    /// Evidence from the synthetic encoder check, by rate, size bucket and codec. Shown, never used to promote.
    encoder: BTreeMap<String, Entry>,
    /// Sizes seen before: "screen:<source>" and "camera:<device>" to (width, height).
    observed: BTreeMap<String, (u32, u32)>,
}

/// One synthetic check's result.
#[derive(Clone, Debug, PartialEq)]
pub enum CheckOutcome {
    Passed { output_fps: f64 },
    /// Ran, but the encoder fell behind the fixed clock by more than the recording gate allows.
    Slow { output_fps: f64 },
    /// The encoder would not start with these settings.
    EncoderRefused(String),
    Cancelled,
    Failed(String),
}

fn encoder_key(fps: u32, quality: u32, codec: &str) -> String {
    format!("{fps}@{quality}|{codec}")
}
const SCHEMA: u64 = 2;

impl Capabilities {
    pub fn load(path: &Path) -> Self {
        let mut out = Self::default();
        let Ok(text) = std::fs::read_to_string(path) else { return out };
        let Ok(value) = serde_json::from_str::<Value>(&text) else { return out };
        // An older file recorded evidence without its workload; it is not trusted for anything.
        if value["schema"] != SCHEMA {
            return out;
        }
        let read = |v: &Value| -> BTreeMap<String, Entry> {
            v.as_object()
                .into_iter()
                .flatten()
                .filter_map(|(name, e)| {
                    Some((
                        name.clone(),
                        Entry {
                            status: e["status"].as_str().and_then(Status::parse)?,
                            basis: e["basis"].as_str().unwrap_or("take").chars().take(16).collect(),
                            codec: e["codec"].as_str().unwrap_or_default().chars().take(40).collect(),
                            at: e["at"].as_u64().unwrap_or(0),
                            measured_fps: e["measuredFps"].as_f64().filter(|n| n.is_finite()),
                        },
                    ))
                })
                .collect()
        };
        out.entries = read(&value["entries"]);
        out.encoder = read(&value["encoder"]);
        for (name, size) in value["observed"].as_object().into_iter().flatten() {
            if let (Some(w), Some(h)) = (size[0].as_u64(), size[1].as_u64()) {
                if (2..=16384).contains(&w) && (2..=16384).contains(&h) && out.observed.len() < 256 {
                    out.observed.insert(name.chars().take(80).collect(), (w as u32, h as u32));
                }
            }
        }
        out
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let write = |map: &BTreeMap<String, Entry>| -> serde_json::Map<String, Value> {
            map.iter()
                .map(|(k, e)| (k.clone(), json!({"status":e.status.name(),"basis":e.basis,"codec":e.codec,"at":e.at,"measuredFps":e.measured_fps})))
                .collect()
        };
        let observed: serde_json::Map<String, Value> = self.observed.iter().map(|(k, (w, h))| (k.clone(), json!([w, h]))).collect();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&json!({"schema":SCHEMA,"entries":write(&self.entries),"encoder":write(&self.encoder),"observed":observed}))?)?;
        std::fs::rename(tmp, path)
    }

    /// The workload a take would be, from what is known now. Anything not yet observed stays unknown.
    pub fn plan(&self, fps: u32, quality: u32, codec: &str, source: &str, camera: Option<&str>, audio: bool, originals: bool) -> Workload {
        Workload {
            fps,
            quality,
            codec: codec.to_owned(),
            source: source.to_owned(),
            geometry: self.observed.get(&format!("screen:{source}")).copied(),
            camera: match camera {
                None => CameraPlan::Off,
                Some(device) => self.observed.get(&format!("camera:{device}")).map_or(CameraPlan::Unobserved, |(w, h)| CameraPlan::Size(*w, *h)),
            },
            audio,
            // Originals only exist beside a camera: a screen-only take never duplicates the screen.
            originals: originals && camera.is_some(),
        }
    }
    /// Evidence for exactly this workload; unknown for anything else.
    pub fn status(&self, w: &Workload) -> Entry {
        match w.key().and_then(|k| self.entries.get(&k)) {
            Some(e) => e.clone(),
            None => Entry { status: Status::Unknown, basis: "none".into(), codec: w.codec.clone(), at: 0, measured_fps: None },
        }
    }
    /// What the synthetic encoder check last found for a rate, size and codec. Display only.
    pub fn encoder_status(&self, fps: u32, quality: u32, codec: &str) -> Option<Entry> {
        self.encoder.get(&encoder_key(fps, quality, codec)).cloned()
    }
    /// Remember the sizes a take actually saw, so the next plan can name its workload.
    pub fn observe(&mut self, source: &str, camera: Option<&str>, capture: &Value) -> bool {
        let mut changed = false;
        let mut put = |key: String, w: Option<u64>, h: Option<u64>| {
            if let (Some(w), Some(h)) = (w.filter(|n| (2..=16384).contains(n)), h.filter(|n| (2..=16384).contains(n))) {
                if self.observed.len() < 256 || self.observed.contains_key(&key) {
                    changed |= self.observed.insert(key, (w as u32, h as u32)) != Some((w as u32, h as u32));
                }
            }
        };
        if Workload::stable_source(source) {
            put(format!("screen:{source}"), capture["width"].as_u64(), capture["height"].as_u64());
        }
        if let (Some(device), true) = (camera, capture["camera"].as_bool() == Some(true)) {
            put(format!("camera:{device}"), capture["cameraWidth"].as_u64(), capture["cameraHeight"].as_u64());
        }
        changed
    }
    /// Fold one finished take's capture.json in. Returns whether anything changed.
    /// Only a completed take that delivered its rate qualifies *its own workload*; only the recording-speed gate
    /// failing marks it failed. Cancels, camera failures and every other error say nothing about capacity.
    pub fn record_take(&mut self, capture: &Value, source: &str, camera: Option<&str>, codec: &str, at: u64) -> bool {
        let seen = self.observe(source, camera, capture);
        let Some(workload) = Workload::from_take(capture, source, codec) else { return seen };
        if workload.fps != 60 {
            return seen;
        }
        let Some(key) = workload.key() else { return seen };
        let measured = capture["cadence"]["outputFps"].as_f64().filter(|n| n.is_finite());
        let status = match capture["status"].as_str() {
            Some("completed") if measured.is_some_and(|m| m >= f64::from(workload.fps) * DELIVERED_SHARE) => Status::Qualified,
            Some("failed") if capture["error"].as_str().is_some_and(|e| e.starts_with("Recording cannot keep up")) => Status::FailedLast,
            _ => return seen,
        };
        let entry = Entry { status, basis: "take".into(), codec: codec.to_owned(), at, measured_fps: measured };
        let changed = self.entries.insert(key, entry.clone()).as_ref() != Some(&entry);
        changed || seen
    }
    /// Record a synthetic check. It describes the encoder alone and never touches take evidence.
    pub fn record_check(&mut self, fps: u32, quality: u32, codec: &str, outcome: &CheckOutcome, at: u64) -> bool {
        let (status, measured) = match outcome {
            CheckOutcome::Passed { output_fps } => (Status::Qualified, Some(*output_fps)),
            CheckOutcome::Slow { output_fps } => (Status::FailedLast, Some(*output_fps)),
            CheckOutcome::EncoderRefused(_) => (Status::Unavailable, None),
            CheckOutcome::Cancelled | CheckOutcome::Failed(_) => return false,
        };
        let entry = Entry { status, basis: "check".into(), codec: codec.to_owned(), at, measured_fps: measured };
        self.encoder.insert(encoder_key(fps, quality, codec), entry.clone()).as_ref() != Some(&entry)
    }
    /// The `capabilities.fps60` object the app publishes to Home: the status for this workload, and the
    /// encoder check beside it, labelled as encoder capacity only.
    pub fn publish_fps60(&self, w: &Workload) -> Value {
        let e = self.status(w);
        let basis = match (e.status, w.is_exact()) {
            (Status::Unknown, false) => "Not measured for this setup yet",
            (Status::Unknown, true) => "Not measured for this setup yet",
            (Status::Qualified, _) => "A recording of this setup on this computer kept up",
            (Status::FailedLast, _) => "The last 60 fps recording of this setup could not keep up",
            (Status::Unavailable, _) => "Not measured for this setup yet",
        };
        let mut out = json!({"status":match e.status { Status::Unavailable => "unknown", s => s.name() },"basis":basis,"measuredFps":e.measured_fps});
        if let Some(c) = self.encoder_status(w.fps, w.quality, &w.codec) {
            out["encoder"] = json!({"status":c.status.name(),"measuredFps":c.measured_fps});
        }
        out
    }
}

/// The rate to use when the person has not chosen one. Only the factory default (30, never chosen) can become
/// 60, and only where 60 is qualified for this very workload; every other stored value, 24 and an explicit 30
/// or 60 included, is returned unchanged. Nothing is ever lowered.
pub fn effective_fps(stored: u32, chosen: bool, status: Status) -> u32 {
    if !chosen && stored == 30 && status == Status::Qualified {
        60
    } else {
        stored
    }
}

/// The few things the synthetic check needs from an encoder. The real one is the recorder's pipeline; tests
/// substitute one that fails where a real encoder could.
pub trait CheckSink {
    fn push(&mut self, picture: &[u8]) -> std::io::Result<()>;
    /// Flush and close. A check only counts if this succeeds.
    fn finish(&mut self) -> std::io::Result<()>;
}
struct PipelineSink(crate::pipeline::Pipeline);
impl CheckSink for PipelineSink {
    fn push(&mut self, picture: &[u8]) -> std::io::Result<()> {
        self.0.set_screen(picture)?;
        self.0.write()
    }
    fn finish(&mut self) -> std::io::Result<()> {
        self.0.finish()
    }
}

/// The paced loop of the check, over any sink. The same 0.75 s lag limit a recording applies (capture.rs) is
/// reproduced rather than relaxed. A pass needs the delivered cadence **and** a successful finish: an encoder
/// that accepts frames and then fails to flush has not shown anything.
pub fn run_check(sink: &mut dyn CheckSink, w: usize, h: usize, fps: u32, seconds: f64, cancel: &std::sync::atomic::AtomicBool) -> CheckOutcome {
    use std::{sync::atomic::Ordering, time::Instant};
    // Moving content, so the encoder does real work rather than coding a flat frame.
    let mut picture = vec![255u8; w * h * 4];
    let period = Duration::from_secs_f64(1. / f64::from(fps));
    let epoch = Instant::now();
    let mut count = 0u32;
    let mut slow = false;
    while epoch.elapsed().as_secs_f64() < seconds {
        if cancel.load(Ordering::Relaxed) {
            let _ = sink.finish();
            return CheckOutcome::Cancelled;
        }
        let shade = (count * 7 % 256) as u8;
        for (i, px) in picture.chunks_exact_mut(4).enumerate() {
            px[0] = shade.wrapping_add((i % w / 8) as u8);
            px[1] = shade.wrapping_add((i / w / 8) as u8);
            px[2] = shade;
        }
        if let Err(e) = sink.push(&picture) {
            let _ = sink.finish();
            return CheckOutcome::Failed(e.to_string());
        }
        count += 1;
        if epoch.elapsed().as_secs_f64() - f64::from(count) / f64::from(fps) > 0.75 {
            slow = true;
            break;
        }
        std::thread::sleep((epoch + period.mul_f64(f64::from(count))).saturating_duration_since(Instant::now()));
    }
    let elapsed = epoch.elapsed().as_secs_f64().max(1e-3);
    let output_fps = f64::from(count) / elapsed;
    if let Err(e) = sink.finish() {
        return CheckOutcome::Failed(format!("The encoder did not finish cleanly: {e}"));
    }
    if slow || output_fps < f64::from(fps) * DELIVERED_SHARE {
        CheckOutcome::Slow { output_fps }
    } else {
        CheckOutcome::Passed { output_fps }
    }
}

/// An explicit, bounded, input-free check: encodes synthetic pictures at the requested size and rate through
/// the real encoder path on the fixed clock, writing to a throwaway folder that is removed on every path. It
/// opens no screen, camera or microphone, needs no permission, and is only ever started by a person's request.
pub fn capacity_check(ffmpeg: &str, fps: u32, quality: u32, seconds: f64, cancel: &std::sync::atomic::AtomicBool) -> CheckOutcome {
    let Some((w, h)) = check_dimensions(quality) else { return CheckOutcome::Failed("Unsupported size".into()) };
    if ![24, 30, 60].contains(&fps) {
        return CheckOutcome::Failed("Unsupported frame rate".into());
    }
    let dir = std::env::temp_dir().join(format!("openrecorder-capacity-{}-{}", std::process::id(), crate::connection::now_ms()));
    if std::fs::create_dir_all(&dir).is_err() {
        return CheckOutcome::Failed("No scratch space for the test".into());
    }
    let outcome = match crate::pipeline::Pipeline::start_config(ffmpeg, &dir, w, h, 0, 0, None, false, fps) {
        Ok(pipeline) => run_check(&mut PipelineSink(pipeline), w, h, fps, seconds, cancel),
        Err(e) => CheckOutcome::EncoderRefused(e.to_string()),
    };
    // Cleanup runs on every path. A scratch folder that cannot be removed is reported, not hidden.
    match (std::fs::remove_dir_all(&dir), outcome) {
        (Err(e), CheckOutcome::Passed { .. }) => CheckOutcome::Failed(format!("The test's scratch files could not be removed: {e}")),
        (_, outcome) => outcome,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    const CODEC: &str = "h264_videotoolbox";
    fn take(status: &str, error: Option<&str>, out: Option<f64>, camera: Option<(u64, u64)>, audio: bool, originals: bool, geometry: (u64, u64)) -> Value {
        json!({"status":status,"error":error,"fps":60,"width":geometry.0,"height":geometry.1,"camera":camera.is_some(),
               "cameraWidth":camera.map(|c| c.0),"cameraHeight":camera.map(|c| c.1),"cadence":{"outputFps":out},
               "workload":{"quality":2560,"audio":audio,"originals":originals,"source":"display:1"}})
    }
    /// The heavy workload of the failed ac1552 take: display, camera 900x720, microphone, originals.
    fn heavy(c: &Capabilities) -> Workload {
        c.plan(60, 2560, CODEC, "display:1", Some("cam-A"), true, true)
    }
    fn light(c: &Capabilities) -> Workload {
        c.plan(60, 2560, CODEC, "display:1", None, false, false)
    }
    fn good(camera: Option<(u64, u64)>, audio: bool, originals: bool) -> Value {
        take("completed", None, Some(59.9), camera, audio, originals, (2560, 1440))
    }

    #[test]
    fn nothing_is_qualified_or_refused_without_a_measurement() {
        let c = Capabilities::default();
        assert_eq!(c.status(&heavy(&c)).status, Status::Unknown);
        assert_eq!(c.publish_fps60(&heavy(&c))["status"], "unknown");
        assert_eq!(effective_fps(30, false, Status::Unknown), 30);
        assert_eq!(effective_fps(60, true, Status::Unknown), 60, "an explicit 60 is never lowered while unknown");
        assert_eq!(effective_fps(60, false, Status::FailedLast), 60, "no silent downgrade");
    }

    #[test]
    fn a_take_qualifies_only_its_own_workload_never_a_heavier_or_different_one() {
        let mut c = Capabilities::default();
        // A light screen-only take completes at 60 on display 1.
        assert!(c.record_take(&good(None, false, false), "display:1", None, CODEC, 1));
        assert_eq!(c.status(&light(&c)).status, Status::Qualified);
        // The camera + microphone + originals workload (the one that failed in ac1552) is a different workload:
        // its camera size has not been observed, and even once it is, nothing qualified it.
        assert_eq!(c.status(&heavy(&c)).status, Status::Unknown);
        c.record_take(&take("failed", Some("Camera stopped"), None, Some((900, 720)), true, true, (2560, 1440)), "display:1", Some("cam-A"), CODEC, 2);
        assert_eq!(c.status(&heavy(&c)).status, Status::Unknown, "now planned exactly, still never measured");
        // Each single difference keeps the evidence apart.
        for other in [
            c.plan(60, 2560, CODEC, "display:1", None, true, false),           // audio
            c.plan(60, 1920, CODEC, "display:1", None, false, false),          // size bucket
            c.plan(60, 2560, "libx264", "display:1", None, false, false),      // codec
            c.plan(60, 2560, CODEC, "display:2", None, false, false),          // another display
            c.plan(60, 2560, CODEC, "window", None, false, false),             // a window
        ] {
            assert_eq!(c.status(&other).status, Status::Unknown, "{other:?}");
        }
        // Qualifying the heavy workload does not leak back or sideways.
        assert!(c.record_take(&good(Some((900, 720)), true, true), "display:1", Some("cam-A"), CODEC, 3));
        assert_eq!(c.status(&heavy(&c)).status, Status::Qualified);
        let mut d = c.clone();
        d.observed.insert("camera:cam-A".into(), (1280, 720));
        assert_eq!(d.status(&heavy(&d)).status, Status::Unknown, "a different camera frame size is a different workload");
        assert_eq!(c.status(&c.plan(60, 2560, CODEC, "display:1", Some("cam-A"), false, true)).status, Status::Unknown, "no microphone");
        assert_eq!(c.status(&c.plan(60, 2560, CODEC, "display:1", Some("cam-A"), true, false)).status, Status::Unknown, "no originals");
    }

    #[test]
    fn a_source_that_is_not_a_real_display_id_never_matches_and_never_records_evidence() {
        let mut c = Capabilities::default();
        // Takes on an unresolved source or a window qualify nothing and teach no geometry.
        for source in ["unresolved", "window", "default", "display:", "display:x1", "portal"] {
            c.record_take(&good(None, false, false), source, None, CODEC, 1);
            assert!(c.entries.is_empty() && c.observed.is_empty(), "{source} left evidence behind: {:?}", c.observed);
            let w = c.plan(60, 2560, CODEC, source, None, false, false);
            assert!(!w.is_exact() && c.status(&w).status == Status::Unknown, "{source}");
        }
        // A real display qualifies itself, and none of the others can borrow it, whatever geometry they report.
        assert!(c.record_take(&good(None, false, false), "display:1", None, CODEC, 2));
        assert_eq!(c.status(&light(&c)).status, Status::Qualified);
        for source in ["unresolved", "window", "display:2"] {
            let mut w = light(&c);
            w.source = source.into();
            w.geometry = Some((2560, 1440));
            assert_eq!(c.status(&w).status, Status::Unknown, "{source}");
        }
        let mut published = c.plan(60, 2560, CODEC, "unresolved", None, false, false);
        published.geometry = Some((2560, 1440));
        assert_eq!(c.publish_fps60(&published)["status"], "unknown", "and Home is told unknown, not qualified");
    }
    #[test]
    fn the_synthetic_check_is_encoder_evidence_only_and_cannot_promote_an_actual_configuration() {
        let mut c = Capabilities::default();
        assert!(c.record_check(60, 2560, CODEC, &CheckOutcome::Passed { output_fps: 60.0 }, 5));
        for w in [heavy(&c), light(&c)] {
            assert_eq!(c.status(&w).status, Status::Unknown);
            assert_eq!(effective_fps(30, false, c.status(&w).status), 30, "a passing check never raises a default");
        }
        let published = c.publish_fps60(&light(&c));
        assert_eq!(published["status"], "unknown");
        assert_eq!(published["encoder"]["status"], "qualified", "but it is shown, beside the workload's own answer");
        // A failing or refusing check does not mark any workload failed or unavailable either.
        assert!(c.record_check(60, 2560, CODEC, &CheckOutcome::EncoderRefused("no".into()), 6));
        assert_eq!(c.status(&light(&c)).status, Status::Unknown);
        assert_eq!(c.publish_fps60(&light(&c))["status"], "unknown", "the workload is never reported unavailable");
        assert_eq!(c.publish_fps60(&light(&c))["encoder"]["status"], "unavailable");
        assert!(!c.record_check(60, 2560, CODEC, &CheckOutcome::Cancelled, 7) && !c.record_check(60, 2560, CODEC, &CheckOutcome::Failed("x".into()), 7));
    }

    #[test]
    fn only_the_speed_gate_marks_a_workload_failed_and_the_latest_measurement_wins() {
        let mut c = Capabilities::default();
        c.record_take(&good(None, false, false), "display:1", None, CODEC, 1);
        let gate = "Recording cannot keep up. Choose a lower resolution or frame rate.";
        assert!(c.record_take(&take("failed", Some(gate), None, None, false, false, (2560, 1440)), "display:1", None, CODEC, 2));
        assert_eq!(c.status(&light(&c)).status, Status::FailedLast);
        for (status, error) in [("failed", "Camera stopped"), ("failed", "Unexpected native pixel dimensions or stride: 1x1"), ("cancelled", ""), ("failed", "Encoder failed: exit status: 234")] {
            assert!(!c.record_take(&take(status, Some(error), None, None, false, false, (2560, 1440)), "display:1", None, CODEC, 3), "{error}");
        }
        assert!(!c.record_take(&take("completed", None, Some(40.0), None, false, false, (2560, 1440)), "display:1", None, CODEC, 4));
        assert_eq!(c.status(&light(&c)).status, Status::FailedLast);
        assert!(c.record_take(&good(None, false, false), "display:1", None, CODEC, 5));
        assert_eq!(c.status(&light(&c)).status, Status::Qualified);
        // Takes of other rates, windows, and takes written without a workload record nothing.
        let mut thirty = good(None, false, false);
        thirty["fps"] = json!(30);
        assert!(!c.record_take(&thirty, "display:1", None, CODEC, 6));
        let mut legacy = good(None, false, false);
        legacy.as_object_mut().unwrap().remove("workload");
        let before = c.entries.clone();
        c.record_take(&legacy, "display:9", None, CODEC, 7);
        assert_eq!(c.entries, before, "a take without its workload qualifies nothing");
        c.record_take(&good(None, false, false), "window", None, CODEC, 8);
        assert_eq!(c.status(&c.plan(60, 2560, CODEC, "window", None, false, false)).status, Status::Unknown);
    }

    #[test]
    fn promotion_to_sixty_touches_only_the_untouched_factory_default() {
        assert_eq!(effective_fps(30, false, Status::Qualified), 60);
        assert_eq!(effective_fps(30, true, Status::Qualified), 30, "a chosen 30 stays 30");
        assert_eq!(effective_fps(24, false, Status::Qualified), 24, "a stored 24 is the owner's and stays");
        assert_eq!(effective_fps(24, true, Status::Qualified), 24);
        assert_eq!(effective_fps(60, false, Status::Qualified), 60);
        assert_eq!(effective_fps(30, false, Status::FailedLast), 30);
        assert_eq!(effective_fps(30, false, Status::Unavailable), 30);
    }

    #[test]
    fn evidence_and_observed_sizes_survive_a_restart_and_a_damaged_or_older_file_is_ignored() {
        let dir = std::env::temp_dir().join(format!("openrecorder-capabilities-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("capabilities.json");
        let mut c = Capabilities::default();
        c.record_take(&good(Some((900, 720)), true, true), "display:1", Some("cam-A"), CODEC, 9);
        c.record_check(60, 1920, "libx264", &CheckOutcome::Passed { output_fps: 60.0 }, 10);
        c.save(&path).unwrap();
        let loaded = Capabilities::load(&path);
        assert_eq!(loaded, c);
        assert_eq!(loaded.status(&heavy(&loaded)).status, Status::Qualified);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Capabilities::load(&path), Capabilities::default());
        // The previous schema keyed evidence by rate and size only; it must not be trusted for any workload.
        std::fs::write(&path, r#"{"schema":1,"entries":{"60@2560":{"status":"qualified","codec":"h264_videotoolbox"}}}"#).unwrap();
        assert_eq!(Capabilities::load(&path), Capabilities::default());
        std::fs::write(&path, r#"{"schema":2,"entries":{"x":{"status":"bogus"}},"observed":{"screen:display:1":[1,99999],"camera:a":[640,480]}}"#).unwrap();
        let odd = Capabilities::load(&path);
        assert!(odd.entries.is_empty() && odd.observed.len() == 1 && odd.observed["camera:a"] == (640, 480));
        let _ = std::fs::remove_dir_all(dir);
    }

    struct FakeSink {
        pushes: u32,
        finish: Result<(), &'static str>,
        finished: u32,
        push_error: bool,
    }
    impl CheckSink for FakeSink {
        fn push(&mut self, _: &[u8]) -> std::io::Result<()> {
            self.pushes += 1;
            if self.push_error {
                Err(std::io::Error::other("pipe closed"))
            } else {
                Ok(())
            }
        }
        fn finish(&mut self) -> std::io::Result<()> {
            self.finished += 1;
            self.finish.map_err(std::io::Error::other)
        }
    }
    #[test]
    fn a_check_passes_only_if_the_encoder_finishes_cleanly_whatever_the_cadence() {
        let cancel = AtomicBool::new(false);
        let mut ok = FakeSink { pushes: 0, finish: Ok(()), finished: 0, push_error: false };
        let passed = run_check(&mut ok, 16, 16, 60, 0.4, &cancel);
        assert!(matches!(passed, CheckOutcome::Passed { output_fps } if output_fps > 55.0), "{passed:?}");
        assert_eq!((ok.finished, ok.pushes > 15), (1, true));
        // Frames were accepted at full cadence, then the flush failed: nothing was shown.
        let mut flush = FakeSink { pushes: 0, finish: Err("remux failed"), finished: 0, push_error: false };
        let outcome = run_check(&mut flush, 16, 16, 60, 0.4, &cancel);
        assert!(matches!(&outcome, CheckOutcome::Failed(m) if m.contains("did not finish cleanly") && m.contains("remux failed")), "{outcome:?}");
        assert!(flush.pushes > 15, "the cadence alone was fine");
        let mut c = Capabilities::default();
        assert!(!c.record_check(60, 2560, CODEC, &outcome, 1), "and so it records nothing");
        assert!(c.encoder_status(60, 2560, CODEC).is_none());
        // A write error still closes the encoder and is reported, not recorded.
        let mut broken = FakeSink { pushes: 0, finish: Ok(()), finished: 0, push_error: true };
        assert!(matches!(run_check(&mut broken, 16, 16, 60, 0.4, &cancel), CheckOutcome::Failed(_)));
        assert_eq!(broken.finished, 1);
        // Cancelling closes it too.
        let stop = AtomicBool::new(true);
        let mut cancelled = FakeSink { pushes: 0, finish: Ok(()), finished: 0, push_error: false };
        assert_eq!(run_check(&mut cancelled, 16, 16, 60, 0.4, &stop), CheckOutcome::Cancelled);
        assert_eq!(cancelled.finished, 1);
    }

    #[test]
    fn the_check_is_input_free_cleans_up_on_every_path_and_keeps_the_recording_gate_unchanged() {
        let production = include_str!("capability.rs").split("#[cfg(test)]").next().unwrap();
        for forbidden in ["scap::", "camera_frame", "AudioHandle", "permissions::", "request_"] {
            assert!(!production.contains(forbidden), "the capacity check must not touch {forbidden}");
        }
        let capture = include_str!("capture.rs").split("#[cfg(test)]").next().unwrap();
        assert!(capture.contains("- count as f64 / options.fps as f64 > 0.75"));
        assert!(production.contains("/ f64::from(fps) > 0.75"));
        assert!(!capture.contains("excluded_startup") && !capture.contains("warmup"));
        // Every take records the workload that evidence is keyed on.
        assert!(capture.contains(r#""workload":{"quality":options.quality,"audio":options.microphone.is_some(),"originals":options.originals && camera.is_some(),"#) && capture.contains(r#""source":target_name(options.selection)}"#));
        // One removal, after the outcome is known, on every path (start refused, finished or failed).
        assert_eq!(production.matches("remove_dir_all(&dir)").count(), 1);
        assert!(production.contains("sink.finish()") && production.contains("did not finish cleanly"));
        assert_eq!(check_dimensions(2560), Some((2560, 1440)));
        assert_eq!(check_dimensions(1000), None);
        let cancel = AtomicBool::new(false);
        assert!(matches!(capacity_check("ffmpeg", 61, 1920, 1.0, &cancel), CheckOutcome::Failed(_)));
        assert!(matches!(capacity_check("ffmpeg", 60, 1000, 1.0, &cancel), CheckOutcome::Failed(_)));
        // With no encoder to start, the scratch folder is still removed.
        let before = std::fs::read_dir(std::env::temp_dir()).unwrap().filter(|e| e.as_ref().is_ok_and(|e| e.file_name().to_string_lossy().starts_with(&format!("openrecorder-capacity-{}-", std::process::id())))).count();
        assert!(matches!(capacity_check("/nonexistent/ffmpeg", 60, 1920, 1.0, &cancel), CheckOutcome::EncoderRefused(_)));
        let after = std::fs::read_dir(std::env::temp_dir()).unwrap().filter(|e| e.as_ref().is_ok_and(|e| e.file_name().to_string_lossy().starts_with(&format!("openrecorder-capacity-{}-", std::process::id())))).count();
        assert_eq!(before, after, "no scratch folder is left behind");
    }
}
