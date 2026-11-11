//! The pure rules between the menu, Home and the capture machine.
//!
//! Nothing here touches a device, a window or the network. The desktop module owns the machine and
//! feeds these functions its facts; they decide which relayed command may run, how a failure reads to
//! a person, and exactly what is published to Home.
use super::connection::{HumanAction, HumanCommand, HUMAN_TTL_MS};
use serde_json::{json, Value};

/// The stages of a take as Home and the tray both name them. Terminal outcomes (saved, failed,
/// cancelled, interrupted) are not stages: they are `Idle` plus a notice or a Recent entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Countdown(u32),
    /// Inputs are being checked and opened, or the encoder is starting. Nothing is captured yet.
    Starting,
    Recording,
    /// Capture has ended and the encoder is finishing the files.
    Stopping,
    /// Capture is done; the library has not confirmed the take yet.
    Saving,
}
impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Countdown(_) => "countdown",
            Self::Starting => "starting",
            Self::Recording => "recording",
            Self::Stopping => "stopping",
            Self::Saving => "saving",
        }
    }
    pub fn busy(self) -> bool {
        self != Self::Idle
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub kind: &'static str,
    pub text: String,
    pub details: Option<String>,
    /// The take left files: Home's one next step is to open that recording.
    pub kept: bool,
    pub generation: u64,
}

fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The long edge picked in Settings read as the 16:9 size it produces ("2560 × 1440").
pub fn size_words(quality: u32) -> String {
    match quality {
        1280 => "1280 × 720".into(),
        2560 => "2560 × 1440".into(),
        _ => "1920 × 1080".into(),
    }
}

/// What a failed take says to a person. The raw backend text only ever goes to `details`.
pub fn failure_notice(error: &str, fps: u32, quality: u32, seconds: u64, frames: u64, generation: u64) -> Notice {
    let lower = error.to_lowercase();
    let text = if error.starts_with("Recording cannot keep up") {
        format!("Couldn't keep up at {fps} fps, {}. Originals are kept.", size_words(quality))
    } else if error.starts_with("Unexpected native pixel dimensions or stride") {
        "The screen image had an unexpected size or layout. Originals are kept.".to_owned()
    } else if frames == 0 {
        "Couldn't start recording. Nothing was recorded.".to_owned()
    } else if lower.contains("camera") {
        format!("Camera disconnected. Saved up to {}.", clock(seconds))
    } else if lower.contains("microphone") || lower.contains("audio") {
        format!("Microphone stopped. Saved up to {}.", clock(seconds))
    } else {
        "Recording stopped. Originals are kept.".to_owned()
    };
    // Whatever the cause, Home says only "Recording stopped early. Available files were kept." and offers the
    // recording; the cause stays in `details` (Technical details). A take with no frames left nothing to open.
    Notice { kind: "failed", text, details: Some(error.chars().take(1200).collect()), kept: frames > 0, generation }
}

/// Whether a relayed command may run now. Anything not `Run` is dropped silently: a late, stale, replayed or
/// out-of-order command must never do something the person did not ask for at this moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Run,
    /// Made for another launch or take, issued before this process started, expired, or already used.
    Stale,
    /// Not allowed in the current stage.
    Busy,
}

/// What the app knows about itself when a command is about to run.
pub struct Session<'a> {
    pub launch: &'a str,
    pub generation: u64,
    pub launched_at: u64,
    /// Read when the command is about to run, not when it was received.
    pub now: u64,
}

/// Request numbers already seen, so the same control cannot run twice. Bounded.
#[derive(Default)]
pub struct Consumed(std::collections::VecDeque<String>);
impl Consumed {
    /// Marks a number used. False if it already was.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn take(&mut self, nonce: &str) -> bool {
        if self.0.iter().any(|n| n == nonce) {
            return false;
        }
        if self.0.len() >= 128 {
            self.0.pop_front();
        }
        self.0.push_back(nonce.to_owned());
        true
    }
}

/// Every check a control must pass at the moment of execution: this launch, this take, not expired (the
/// expiry is read again here, however long the command waited), not issued before the app started, single use.
/// Only then does the stage decide.
pub fn gate(cmd: &HumanCommand, stage: Stage, s: &Session, seen: &mut Consumed) -> Gate {
    if cmd.launch != s.launch || cmd.issued_at < s.launched_at || s.now > cmd.expires_at || cmd.expires_at > cmd.issued_at + HUMAN_TTL_MS + 1000 {
        return Gate::Stale;
    }
    // Used up on the first look: a control that was busy a moment ago is not held for later.
    if !seen.take(&cmd.nonce) {
        return Gate::Stale;
    }
    let for_this_take = cmd.generation == Some(s.generation);
    match &cmd.action {
        // A start request exists only for an idle app; it never queues behind a take, and names the take it saw.
        HumanAction::Record => match stage {
            Stage::Idle if for_this_take => Gate::Run,
            Stage::Idle => Gate::Stale,
            _ => Gate::Busy,
        },
        HumanAction::Stop => match stage {
            Stage::Recording if for_this_take => Gate::Run,
            Stage::Recording => Gate::Stale,
            _ => Gate::Busy,
        },
        HumanAction::Cancel => match stage {
            Stage::Countdown(_) | Stage::Starting if for_this_take => Gate::Run,
            Stage::Countdown(_) | Stage::Starting => Gate::Stale,
            _ => Gate::Busy,
        },
        // Shape, size and mirror are live while recording; everything about inputs, sources and rates is not.
        HumanAction::Shape(_) | HumanAction::Size(_) | HumanAction::Mirror(_) | HumanAction::ShowCamera(_) => {
            if matches!(stage, Stage::Idle | Stage::Recording) { Gate::Run } else { Gate::Busy }
        }
        // Opening a System Settings page asks for nothing and changes nothing, so it is allowed at any stage.
        HumanAction::DismissNotice | HumanAction::OpenPrivacy(_) => Gate::Run,
        HumanAction::Input { .. }
        | HumanAction::Source { .. }
        | HumanAction::Device { .. }
        | HumanAction::Fps(_)
        | HumanAction::ChooseWindow
        | HumanAction::CapacityCheck => if stage == Stage::Idle { Gate::Run } else { Gate::Busy },
    }
}

/// How often the app tells the host it is alive and what state it is in, and exactly when it next needs to wake.
/// The host refuses controls for an app it has not heard from in 12 s, so an idle app must publish on its own
/// schedule, not only when something else happens to repaint it. `due` says whether to publish now and, either
/// way, when the next wakeup must be requested; the wakeup is never sooner than 250 ms (no busy loop).
#[derive(Default)]
pub struct Heartbeat {
    last: Option<(String, u64)>,
}
pub const BUSY_PERIOD_MS: u64 = 1_000;
pub const IDLE_PERIOD_MS: u64 = 4_000;
pub const MIN_WAKE_MS: u64 = 250;
impl Heartbeat {
    /// `(publish now, milliseconds until the next wakeup)`. A changed `key` publishes at once.
    pub fn due(&mut self, key: &str, now: u64, period_ms: u64) -> (bool, u64) {
        let publish = match &self.last {
            Some((last, at)) => last != key || now.saturating_sub(*at) >= period_ms,
            None => true,
        };
        if publish {
            self.last = Some((key.to_owned(), now));
        }
        let waited = self.last.as_ref().map_or(0, |(_, at)| now.saturating_sub(*at));
        (publish, period_ms.saturating_sub(waited).max(MIN_WAKE_MS))
    }
    #[cfg(test)]
    pub fn last_for_test(&self) -> Option<u64> {
        self.last.as_ref().map(|(_, at)| *at)
    }
    /// Forget what was sent, so the next call publishes (a new connection, a changed capability).
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// When to open Home because there is no way to reach the controls otherwise. One episode is one stretch of the
/// tray being unavailable; within it Home is opened at most once, after a short grace, and only if Home is not
/// already open. Episodes are also spaced out, so a tray host that flaps never produces a stream of browser tabs.
#[derive(Default)]
pub struct HomeFallback {
    episode: Option<Episode>,
    last_open: Option<u64>,
}
struct Episode {
    since: u64,
    handled: bool,
}
pub const FALLBACK_GRACE_MS: u64 = 2_000;
pub const FALLBACK_COOLDOWN_MS: u64 = 60_000;
/// Home polls while it is open, at least every 4 s even when hidden in a background tab at the slowest.
pub const HOME_OPEN_WITHIN_MS: u64 = 20_000;
impl HomeFallback {
    /// Feed the tray's availability each pass. Becoming unavailable starts an episode; becoming available ends it.
    pub fn observe(&mut self, available: bool, now: u64) {
        match (available, self.episode.is_some()) {
            (false, false) => self.episode = Some(Episode { since: now, handled: false }),
            (true, true) => self.episode = None,
            _ => {}
        }
    }
    pub fn pending(&self) -> bool {
        self.episode.as_ref().is_some_and(|e| !e.handled)
    }
    /// True when Home should be opened now. `home_age_ms` is asked only when the decision is due: how long ago
    /// Home last read the state, or None if it never has. During the cooldown after an earlier open the loss is
    /// **kept pending** (not used up): if the tray comes back first the loss is cancelled by `observe`, and if it
    /// is still lost when the cooldown ends Home is opened then, once.
    pub fn due(&mut self, now: u64, home_age_ms: impl FnOnce() -> Option<u64>) -> bool {
        let cooling = self.last_open.is_some_and(|t| now.saturating_sub(t) < FALLBACK_COOLDOWN_MS);
        let Some(e) = self.episode.as_mut().filter(|e| !e.handled && now.saturating_sub(e.since) >= FALLBACK_GRACE_MS) else { return false };
        if cooling {
            return false;
        }
        e.handled = true;
        if home_age_ms().is_some_and(|age| age <= HOME_OPEN_WITHIN_MS) {
            return false;
        }
        self.last_open = Some(now);
        true
    }
    /// How long until `due` could next say yes, for scheduling one wakeup (None: nothing is waiting). Never zero,
    /// so a caller that sleeps for it cannot spin.
    pub fn next_check(&self, now: u64) -> Option<u64> {
        let e = self.episode.as_ref().filter(|e| !e.handled)?;
        let grace_ends = e.since + FALLBACK_GRACE_MS;
        let cooldown_ends = self.last_open.map_or(0, |t| t + FALLBACK_COOLDOWN_MS);
        Some(grace_ends.max(cooldown_ends).saturating_sub(now).max(250))
    }
}

/// Everything the app publishes about capture, gathered by the desktop module from its one machine.
pub struct Snapshot {
    pub stage: Stage,
    pub generation: u64,
    /// Names this app launch; Home puts it on every control so a control cannot outlive the launch it was made for.
    pub launch: String,
    /// now - started_at, only while recording.
    pub elapsed_ms: Option<u64>,
    pub fps_set: u32,
    pub output_frames: u64,
    /// Measured output cadence for this take (frames written / loop seconds).
    pub fps_delivered: Option<f64>,
    pub source_frames: u64,
    pub source_fps: Option<f64>,
    pub tray_available: bool,
    pub tray_note: Option<String>,
    pub screen_allowed: bool,
    /// (mark, device name) for camera and microphone.
    pub camera: (&'static str, Option<String>),
    pub microphone: (&'static str, Option<String>),
    /// The camera's look as the app applies it right now: (shape name, size, mirror, show camera). During a take a change is
    /// applied live and saved once the take is over, so this is what Home must show, not the saved preference.
    pub appearance: (String, u32, bool, bool),
    /// The listed (camera, microphone) devices as (id, name, selected). Never a made-up name; empty until listed.
    pub devices: (Vec<(String, String, bool)>, Vec<(String, String, bool)>),
    pub source: (&'static str, String),
    pub notice: Option<Notice>,
    pub fps60: Option<Value>,
    /// (id, kind, name, selected) for displays; at most 16 are published.
    pub sources: Vec<(u32, &'static str, String, bool)>,
    /// "allowed", "not-asked" or "denied".
    pub screen_state: &'static str,
}

/// The `native-progress` body. A stage without a running take carries no elapsed time at all, so a
/// stale timer cannot be reconstructed from it.
pub fn progress_json(s: &Snapshot) -> Value {
    let mut v = json!({"phase":s.stage.name(),"generation":s.generation,"launch":s.launch,"fpsSet":s.fps_set,
        "trayAvailable":s.tray_available,"screenAllowed":s.screen_allowed,
        "inputs":{"camera":input(&s.camera),"microphone":input(&s.microphone)},
        "source":{"kind":s.source.0,"name":s.source.1}});
    if let Stage::Countdown(n) = s.stage {
        v["countdown"] = json!(n);
    }
    if s.stage == Stage::Recording {
        v["elapsedMs"] = json!(s.elapsed_ms.unwrap_or(0));
    }
    if matches!(s.stage, Stage::Recording | Stage::Stopping) {
        v["outputFrames"] = json!(s.output_frames);
        v["sourceFrames"] = json!(s.source_frames);
        if let Some(f) = s.fps_delivered.filter(|f| f.is_finite() && *f >= 0.) {
            v["fpsDelivered"] = json!(f);
        }
        if let Some(f) = s.source_fps.filter(|f| f.is_finite() && *f >= 0.) {
            v["sourceFps"] = json!(f);
        }
    }
    if let Some(note) = &s.tray_note {
        v["trayNote"] = json!(note);
    }
    if let Some(n) = &s.notice {
        v["notice"] = json!({"kind":n.kind,"text":n.text,"details":n.details,"generation":n.generation,"kept":n.kept});
    }
    let list = |l: &[(String, String, bool)]| Value::Array(l.iter().take(16).map(|(uid, name, selected)| json!({"uid":uid,"name":name,"selected":selected})).collect());
    v["devices"] = json!({"camera":list(&s.devices.0),"microphone":list(&s.devices.1)});
    v["appearance"] = json!({"shape":s.appearance.0,"size":s.appearance.1,"mirror":s.appearance.2,"showCamera":s.appearance.3});
    v["screenState"] = json!(s.screen_state);
    v["sources"] = Value::Array(s.sources.iter().take(16).map(|(id, kind, name, selected)| json!({"id":id,"kind":kind,"name":name,"selected":selected})).collect());
    if let Some(f) = &s.fps60 {
        v["capabilities"] = json!({"fps60":f});
    }
    v
}
fn input(i: &(&'static str, Option<String>)) -> Value {
    let mut v = json!({"mark":i.0});
    if let Some(name) = &i.1 {
        v["name"] = json!(name);
    }
    v
}

/// The three most recent finished takes from a host `state` reply, as (id, "title · m:ss").
pub fn recent_from_state(state: &Value) -> Vec<(String, String)> {
    let mut takes: Vec<&Value> = state["recordings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["status"] == "ready" && r["archived"] != true)
        .collect();
    takes.sort_by_key(|r| std::cmp::Reverse(r["createdAt"].as_u64().unwrap_or(0)));
    takes
        .into_iter()
        .take(3)
        .filter_map(|r| {
            let id = r["id"].as_str()?;
            let title: String = r["title"].as_str().unwrap_or("Recording").chars().take(40).collect();
            Some((id.to_owned(), format!("{title} · {}", clock(r["duration"].as_f64().unwrap_or(0.).round() as u64))))
        })
        .collect()
}

/// What each kind of source can do, in one line (design §5). The camera bubble is part of a display
/// recording only; a window recording keeps it out.
pub fn capability_line(wayland: bool, window_selected: bool) -> &'static str {
    if wayland {
        "Your desktop asks what to record each time"
    } else if window_selected {
        "Recording a window keeps the camera bubble out of it."
    } else {
        "The camera bubble can appear in a display recording. Recording a window keeps it out."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 10_000;
    const LAUNCH: &str = "launch-aaaa";
    fn cmd(action: HumanAction, generation: Option<u64>, nonce: &str) -> HumanCommand {
        HumanCommand { id: 1, issued_at: NOW - 100, expires_at: NOW + 4_000, launch: LAUNCH.into(), nonce: nonce.into(), generation, action }
    }
    fn session(generation: u64, now: u64) -> Session<'static> {
        Session { launch: LAUNCH, generation, launched_at: 1_000, now }
    }
    /// Runs one fresh command through the gate at the moment it was received.
    fn run(action: HumanAction, stage: Stage, generation: u64) -> Gate {
        gate(&cmd(action, Some(generation), "nonce-run0"), stage, &session(generation, NOW), &mut Consumed::default())
    }

    #[test]
    fn record_only_starts_from_idle_and_never_queues_behind_a_take() {
        assert_eq!(run(HumanAction::Record, Stage::Idle, 4), Gate::Run);
        for busy in [Stage::Countdown(2), Stage::Starting, Stage::Recording, Stage::Stopping, Stage::Saving] {
            assert_eq!(run(HumanAction::Record, busy, 4), Gate::Busy, "{busy:?}");
        }
    }
    #[test]
    fn a_command_waits_no_longer_than_its_expiry_however_it_got_here() {
        // Accepted by the polling thread with plenty of time left, then the app stalls and resumes idle.
        let c = cmd(HumanAction::Record, Some(4), "nonce-late");
        assert_eq!(gate(&c, Stage::Idle, &session(4, NOW + 3_999), &mut Consumed::default()), Gate::Run);
        assert_eq!(gate(&c, Stage::Idle, &session(4, NOW + 4_000), &mut Consumed::default()), Gate::Run, "valid up to the expiry instant");
        assert_eq!(gate(&c, Stage::Idle, &session(4, NOW + 4_001), &mut Consumed::default()), Gate::Stale, "one millisecond later it is dead");
        assert_eq!(gate(&c, Stage::Idle, &session(4, NOW + 600_000), &mut Consumed::default()), Gate::Stale);
        // An expiry far beyond what the host would ever grant is not honoured either.
        let mut long = cmd(HumanAction::Record, Some(4), "nonce-long");
        long.expires_at = long.issued_at + HUMAN_TTL_MS + 5_000;
        assert_eq!(gate(&long, Stage::Idle, &session(4, NOW), &mut Consumed::default()), Gate::Stale);
    }
    #[test]
    fn a_command_from_before_this_launch_or_for_another_launch_can_never_start_anything() {
        for action in [HumanAction::Record, HumanAction::Stop, HumanAction::Mirror(true), HumanAction::CapacityCheck] {
            let mut early = cmd(action.clone(), Some(0), "nonce-pre0");
            early.issued_at = 999;
            assert_eq!(gate(&early, Stage::Idle, &session(0, NOW), &mut Consumed::default()), Gate::Stale);
            let mut other = cmd(action, Some(0), "nonce-oth0");
            other.launch = "launch-bbbb".into();
            assert_eq!(gate(&other, Stage::Idle, &session(0, NOW), &mut Consumed::default()), Gate::Stale, "made for a previous launch");
        }
        let mut edge = cmd(HumanAction::Record, Some(0), "nonce-edge");
        edge.issued_at = 1_000;
        edge.expires_at = 5_000;
        assert_eq!(gate(&edge, Stage::Idle, &session(0, 2_000), &mut Consumed::default()), Gate::Run);
    }
    #[test]
    fn a_control_runs_once_and_a_replay_or_a_busy_look_uses_it_up() {
        let mut seen = Consumed::default();
        let c = cmd(HumanAction::Record, Some(4), "nonce-once");
        assert_eq!(gate(&c, Stage::Idle, &session(4, NOW), &mut seen), Gate::Run);
        assert_eq!(gate(&c, Stage::Idle, &session(4, NOW), &mut seen), Gate::Stale, "the same request number again");
        // Refused while recording, it is not kept for the moment the take ends.
        let held = cmd(HumanAction::Record, Some(4), "nonce-held");
        assert_eq!(gate(&held, Stage::Recording, &session(4, NOW), &mut seen), Gate::Busy);
        assert_eq!(gate(&held, Stage::Idle, &session(4, NOW), &mut seen), Gate::Stale);
        // Bounded.
        for i in 0..300 {
            seen.take(&format!("n-{i}"));
        }
        assert!(seen.0.len() <= 128);
    }
    #[test]
    fn record_stop_and_cancel_must_name_the_current_take() {
        let mut fresh = || Consumed::default();
        let with = |action: HumanAction, g: Option<u64>| cmd(action, g, "nonce-gen0");
        assert_eq!(gate(&with(HumanAction::Record, Some(4)), Stage::Idle, &session(4, NOW), &mut fresh()), Gate::Run);
        assert_eq!(gate(&with(HumanAction::Record, Some(3)), Stage::Idle, &session(4, NOW), &mut fresh()), Gate::Stale, "Home saw an older take");
        assert_eq!(gate(&with(HumanAction::Record, None), Stage::Idle, &session(4, NOW), &mut fresh()), Gate::Stale, "no take named at all");
        assert_eq!(gate(&with(HumanAction::Stop, Some(4)), Stage::Recording, &session(4, NOW), &mut fresh()), Gate::Run);
        assert_eq!(gate(&with(HumanAction::Stop, None), Stage::Recording, &session(4, NOW), &mut fresh()), Gate::Stale);
        assert_eq!(gate(&with(HumanAction::Stop, Some(3)), Stage::Recording, &session(4, NOW), &mut fresh()), Gate::Stale, "an older take's Stop must not stop this one");
        assert_eq!(gate(&with(HumanAction::Stop, Some(4)), Stage::Idle, &session(4, NOW), &mut fresh()), Gate::Busy);
        assert_eq!(gate(&with(HumanAction::Stop, Some(4)), Stage::Stopping, &session(4, NOW), &mut fresh()), Gate::Busy, "already stopping");
        assert_eq!(gate(&with(HumanAction::Cancel, Some(4)), Stage::Countdown(3), &session(4, NOW), &mut fresh()), Gate::Run);
        assert_eq!(gate(&with(HumanAction::Cancel, Some(9)), Stage::Countdown(3), &session(4, NOW), &mut fresh()), Gate::Stale);
        assert_eq!(gate(&with(HumanAction::Cancel, Some(4)), Stage::Starting, &session(4, NOW), &mut fresh()), Gate::Run);
        assert_eq!(gate(&with(HumanAction::Cancel, Some(4)), Stage::Recording, &session(4, NOW), &mut fresh()), Gate::Busy, "recording ends with Stop, not Cancel");
    }
    #[test]
    fn inputs_sources_and_rates_are_locked_during_a_take_but_bubble_looks_stay_live() {
        for action in [
            HumanAction::Input { camera: true, on: true },
            HumanAction::Source { kind: "default".into(), id: None },
            HumanAction::Device { camera: false, uid: None },
            HumanAction::Fps(60),
            HumanAction::CapacityCheck,
        ] {
            assert_eq!(run(action.clone(), Stage::Idle, 1), Gate::Run);
            for busy in [Stage::Countdown(1), Stage::Starting, Stage::Recording, Stage::Stopping, Stage::Saving] {
                assert_eq!(run(action.clone(), busy, 1), Gate::Busy, "{action:?} during {busy:?}");
            }
        }
        for action in [HumanAction::Shape("portrait".into()), HumanAction::Size(168), HumanAction::Mirror(false), HumanAction::ShowCamera(false)] {
            assert_eq!(run(action.clone(), Stage::Recording, 1), Gate::Run);
            assert_eq!(run(action.clone(), Stage::Idle, 1), Gate::Run);
            assert_eq!(run(action.clone(), Stage::Stopping, 1), Gate::Busy);
        }
        assert_eq!(run(HumanAction::DismissNotice, Stage::Recording, 1), Gate::Run);
        // Opening a System Settings page requests nothing and changes nothing, so it is allowed at every stage.
        for stage in [Stage::Idle, Stage::Countdown(1), Stage::Starting, Stage::Recording, Stage::Stopping, Stage::Saving] {
            assert_eq!(run(HumanAction::OpenPrivacy("screen".into()), stage, 1), Gate::Run, "{stage:?}");
        }
    }

    #[test]
    fn home_is_opened_once_per_loss_only_when_closed_and_never_in_a_stream() {
        let mut f = HomeFallback::default();
        let never = || None;
        // Available: nothing to do, and asking costs nothing.
        f.observe(true, 0);
        assert!(!f.pending() && !f.due(5_000, || panic!("not asked while available")) && f.next_check(5_000).is_none());
        // The tray is lost after startup: wait out the grace, then open if Home is closed.
        f.observe(false, 10_000);
        assert!(f.pending());
        assert!(!f.due(11_999, || panic!("not asked before the grace ends")));
        assert_eq!(f.next_check(11_000), Some(1_000));
        assert!(f.due(12_000, never), "Home was never open");
        assert!(!f.pending() && !f.due(70_000, never) && f.next_check(70_000).is_none(), "one open per loss, however long it lasts");
        f.observe(false, 20_000);
        assert!(!f.due(90_000, never), "still the same loss");
        // Home already open (it read the state a moment ago): no popup, ever, for that loss.
        let mut open = HomeFallback::default();
        open.observe(false, 0);
        assert!(!open.due(3_000, || Some(3_000)));
        assert!(!open.due(300_000, never), "decided once");
        // A page that stopped polling long ago counts as closed.
        let mut stale = HomeFallback::default();
        stale.observe(false, 0);
        assert!(stale.due(3_000, || Some(HOME_OPEN_WITHIN_MS + 1)));
    }
    #[test]
    fn a_loss_inside_the_cooldown_waits_for_it_instead_of_being_used_up() {
        let never = || None;
        // Lose the tray and open Home; the tray returns, Home is closed, and it is lost again 20 s later.
        let mut f = HomeFallback::default();
        f.observe(false, 0);
        assert!(f.due(2_000, never));
        f.observe(true, 10_000);
        f.observe(false, 22_000);
        assert!(!f.due(24_000, never), "inside the cooldown: no tab yet");
        assert!(f.pending(), "but the loss is still waiting, not consumed");
        assert!(!f.due(61_999, never), "still cooling (opened at 2 000)");
        assert_eq!(f.next_check(30_000), Some(32_000), "one wakeup, at the end of the cooldown");
        // The loss continues: Home opens once the cooldown ends, then never again for it.
        assert!(f.due(62_000, never));
        assert!(!f.pending() && !f.due(70_000, never) && !f.due(500_000, never));
        // A loss that recovers while it waits is cancelled and opens nothing, ever.
        let mut g = HomeFallback::default();
        g.observe(false, 0);
        assert!(g.due(2_000, never));
        g.observe(true, 5_000);
        g.observe(false, 6_000);
        assert!(!g.due(9_000, never) && g.pending());
        g.observe(true, 30_000);
        assert!(!g.pending() && g.next_check(30_000).is_none());
        assert!(!g.due(62_000, never) && !g.due(120_000, never), "recovered during the cooldown: nothing opens");
        // Flapping inside one cooldown still produces at most one tab, after it.
        let mut h = HomeFallback::default();
        h.observe(false, 0);
        assert!(h.due(2_000, never));
        let mut opened = 0;
        for t in (3_000..61_000).step_by(1_000) {
            h.observe(t % 2_000 == 1_000, t);
            opened += h.due(t, never) as u32;
        }
        assert_eq!(opened, 0, "no stream during the cooldown");
        // The last flap (a loss from 60 000) is still there when the cooldown ends at 62 000: one tab, then none.
        assert!(h.due(62_000, never));
        let mut after = 0;
        for t in (63_000..200_000).step_by(1_000) {
            after += h.due(t, never) as u32;
        }
        assert_eq!(after, 0, "and nothing more for the same continuing loss");
    }
    #[test]
    fn an_idle_app_publishes_on_its_own_schedule_and_asks_for_exactly_the_wakeups_it_needs() {
        // A fake clock and a fake wakeup queue: nothing else ever repaints the app.
        let mut beat = Heartbeat::default();
        let key = "idle|gen0";
        let mut now = 0u64;
        let mut published = vec![];
        let mut wakes = 0;
        while now <= 120_000 {
            let (publish, wake) = beat.due(key, now, IDLE_PERIOD_MS);
            if publish {
                published.push(now);
            }
            assert!((MIN_WAKE_MS..=IDLE_PERIOD_MS).contains(&wake), "a bounded, non-zero wakeup: {wake}");
            wakes += 1;
            now += wake; // the only event is the wakeup it asked for
        }
        assert_eq!(published[0], 0);
        for pair in published.windows(2) {
            assert!(pair[1] - pair[0] <= IDLE_PERIOD_MS, "never silent for more than the period: {pair:?}");
            assert!(pair[1] - pair[0] < 12_000, "well inside the host's 12 s freshness");
        }
        assert!(wakes <= 40, "about one wakeup per period, not a busy loop: {wakes}");
        // A change publishes at once without waiting for the period, and the next wakeup counts from it.
        let (publish, wake) = beat.due("idle|gen1", now + 10, IDLE_PERIOD_MS);
        assert!(publish && wake == IDLE_PERIOD_MS);
        // An incidental extra call in between neither publishes nor shortens the wait below the floor.
        let (publish, wake) = beat.due("idle|gen1", now + 10 + 3_900, IDLE_PERIOD_MS);
        assert!(!publish && wake == MIN_WAKE_MS);
        // Busy states beat faster; reset forces the next one out.
        let (publish, wake) = beat.due("rec", now + 5_000, BUSY_PERIOD_MS);
        assert!(publish && wake == BUSY_PERIOD_MS);
        beat.reset();
        assert!(beat.due("rec", now + 5_100, BUSY_PERIOD_MS).0);
    }
    #[test]
    fn failures_read_in_plain_words_with_the_raw_message_only_in_details() {
        let gate_error = "Recording cannot keep up. Choose a lower resolution or frame rate.";
        let n = failure_notice(gate_error, 60, 2560, 3, 120, 7);
        assert_eq!(n.text, "Couldn't keep up at 60 fps, 2560 × 1440. Originals are kept.");
        assert!(n.kept && n.kind == "failed" && n.generation == 7);
        assert_eq!(n.details.as_deref(), Some(gate_error));
        assert!(!n.text.contains("Choose a lower"), "raw text stays in details");
        assert!(!failure_notice("Unexpected failure", 30, 1920, 0, 0, 1).kept, "a take with no frames left nothing to open");
        assert!(failure_notice(gate_error, 30, 1920, 3, 90, 1).kept);
        let stride = failure_notice("Unexpected native pixel dimensions or stride: 5120x2880 is outside the supported size", 60, 2560, 0, 1, 1);
        assert_eq!(stride.text, "The screen image had an unexpected size or layout. Originals are kept.");
        assert!(stride.details.unwrap().contains("5120x2880"), "the evidence reaches Details");
        assert_eq!(failure_notice("Camera stopped sending frames", 30, 1920, 198, 5940, 1).text, "Camera disconnected. Saved up to 3:18.");
        assert_eq!(failure_notice("anything", 30, 1920, 0, 0, 1).text, "Couldn't start recording. Nothing was recorded.");
        assert_eq!(failure_notice("disk failed", 30, 1920, 12, 360, 1).text, "Recording stopped. Originals are kept.");
        let long = failure_notice(&"x".repeat(5000), 30, 1920, 1, 1, 1);
        assert_eq!(long.details.unwrap().len(), 1200);
        for notice in [n, failure_notice("x", 30, 1920, 5, 150, 1)] {
            assert!(!notice.text.contains("Recording Stopped"), "no frozen status line: {}", notice.text);
        }
    }

    fn snapshot(stage: Stage) -> Snapshot {
        Snapshot {
            stage,
            generation: 3,
            launch: LAUNCH.into(),
            elapsed_ms: Some(46_000),
            fps_set: 60,
            output_frames: 2760,
            fps_delivered: Some(59.8),
            source_frames: 1380,
            source_fps: Some(30.0),
            tray_available: true,
            tray_note: None,
            screen_allowed: true,
            camera: ("live", Some("FaceTime HD Camera".into())),
            microphone: ("off", None),
            devices: (vec![("CAM-A".into(), "FaceTime HD Camera".into(), true)], Vec::new()),
            appearance: ("rounded".into(), 168, true, true),
            source: ("display", "Built-in Display".into()),
            notice: None,
            fps60: None,
            sources: vec![(1, "display", "Built-in Display".into(), true)],
            screen_state: "allowed",
        }
    }
    #[test]
    fn only_a_recording_publishes_a_timer_and_the_two_cadences_stay_separate() {
        let v = progress_json(&snapshot(Stage::Recording));
        assert_eq!(v["appearance"], json!({"shape":"rounded","size":168,"mirror":true,"showCamera":true}), "the applied look is part of every report");
        let mut s = snapshot(Stage::Recording);
        s.appearance = ("wide".into(), 224, false, false);
        assert_eq!(progress_json(&s)["appearance"], json!({"shape":"wide","size":224,"mirror":false,"showCamera":false}));
        let rec = progress_json(&snapshot(Stage::Recording));
        assert_eq!((rec["phase"].as_str(), rec["elapsedMs"].as_u64(), rec["generation"].as_u64()), (Some("recording"), Some(46_000), Some(3)));
        assert_eq!((rec["fpsSet"].as_u64(), rec["fpsDelivered"].as_f64(), rec["sourceFps"].as_f64()), (Some(60), Some(59.8), Some(30.0)));
        // Every other stage carries no elapsed time, so Home cannot show a frozen "0:46".
        for stage in [Stage::Idle, Stage::Countdown(2), Stage::Starting, Stage::Stopping, Stage::Saving] {
            let v = progress_json(&snapshot(stage));
            assert!(v.get("elapsedMs").is_none(), "{stage:?}");
            assert!(!v.to_string().contains("46000"), "{stage:?}");
            assert_eq!(v["phase"], stage.name());
        }
        let idle = progress_json(&snapshot(Stage::Idle));
        assert!(idle.get("fpsDelivered").is_none() && idle.get("sourceFps").is_none() && idle.get("outputFrames").is_none());
        assert_eq!(progress_json(&snapshot(Stage::Countdown(2)))["countdown"], 2);
    }
    #[test]
    fn a_nan_or_negative_cadence_is_never_published() {
        let mut s = snapshot(Stage::Recording);
        s.fps_delivered = Some(f64::NAN);
        s.source_fps = Some(-1.0);
        let v = progress_json(&s);
        assert!(v.get("fpsDelivered").is_none() && v.get("sourceFps").is_none());
        s.fps_delivered = None;
        assert!(progress_json(&s).get("fpsDelivered").is_none(), "unknown stays unknown");
    }
    #[test]
    fn the_notice_and_capability_travel_with_the_state_and_clear_with_it() {
        let mut s = snapshot(Stage::Idle);
        s.notice = Some(failure_notice("Recording cannot keep up. Choose a lower resolution or frame rate.", 60, 2560, 3, 100, 3));
        s.fps60 = Some(json!({"status":"failed-last","basis":"The last 60 fps recording could not keep up"}));
        let v = progress_json(&s);
        assert_eq!((v["notice"]["kind"].as_str(), v["notice"]["kept"].as_bool()), (Some("failed"), Some(true)));
        assert!(v["notice"].get("offer30").is_none(), "no frame-rate advice travels to Home");
        assert_eq!(v["capabilities"]["fps60"]["status"], "failed-last");
        s.notice = None;
        s.fps60 = None;
        let v = progress_json(&s);
        assert!(v.get("notice").is_none() && v.get("capabilities").is_none());
    }

    #[test]
    fn recent_lists_three_ready_takes_newest_first_and_skips_the_rest() {
        let state = json!({"recordings":[
            {"id":"a","title":"First","status":"ready","createdAt":1,"duration":12.4},
            {"id":"b","title":"Second","status":"ready","createdAt":3,"duration":65},
            {"id":"c","title":"Gone","status":"ready","createdAt":9,"duration":1,"archived":true},
            {"id":"d","title":"Interrupted","status":"interrupted","createdAt":8,"duration":0},
            {"id":"e","title":"Third","status":"ready","createdAt":2,"duration":3725},
            {"id":"f","title":"Old","status":"ready","createdAt":0,"duration":1},
        ]});
        let recent = recent_from_state(&state);
        assert_eq!(recent, vec![("b".into(), "Second · 1:05".into()), ("e".into(), "Third · 1:02:05".into()), ("a".into(), "First · 0:12".into())]);
        assert!(recent_from_state(&json!({})).is_empty());
    }
    #[test]
    fn the_capability_line_is_honest_about_what_each_source_can_do() {
        assert!(capability_line(false, false).contains("can appear in a display recording"));
        assert!(capability_line(false, true).contains("keeps the camera bubble out"));
        assert_eq!(capability_line(true, false), "Your desktop asks what to record each time");
    }
}
