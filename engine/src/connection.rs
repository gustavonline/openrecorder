use serde::Deserialize;
use serde_json::{json, Value};
#[derive(Clone, Deserialize)]
pub struct Connection {
    pub origin: String,
    #[serde(default)]
    pub setup_complete: bool,
    key: String,
}
impl Connection {
    pub fn load() -> Result<Option<Self>, String> {
        let file = super::library_host::gui_config()?;
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if let Some(ffmpeg) = raw["ffmpeg"].as_str() {
            if std::path::Path::new(ffmpeg).is_absolute() {
                std::env::set_var("OPENRECORDER_FFMPEG", ffmpeg);
            }
        }
        let value: Self = serde_json::from_slice(&std::fs::read(file).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let port = value
            .origin
            .strip_prefix("http://127.0.0.1:")
            .ok_or("Library must run on loopback")?;
        if port.parse::<u16>().is_err() || value.key.len() < 32 {
            return Err("Invalid local library connection".into());
        }
        Ok(Some(value))
    }
    pub fn action(&self, op: &str, data: Value) -> Result<Value, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(1))
            .timeout_read(std::time::Duration::from_secs(if op == "native-command" {
                30
            } else {
                4
            }))
            .timeout_write(std::time::Duration::from_secs(4))
            .build();
        agent
            .post(&(self.origin.clone() + "/api/action"))
            .set("Authorization", &format!("Bearer {}", self.key))
            .send_json(json!({"op":op,"data":data}))
            .map_err(|error| match error {
                // The host answered and refused: keep its short reason (it is what
                // reconciliation has to tell "already finished" from "unreachable").
                ureq::Error::Status(_, response) => response
                    .into_string()
                    .ok()
                    .and_then(|body| host_refusal(&body))
                    .unwrap_or_else(|| UNAVAILABLE.to_string()),
                ureq::Error::Transport(_) => UNAVAILABLE.to_string(),
            })?
            .into_json()
            // The host answered with a body that could not be read: whether it acted is unknown.
            .map_err(|_| UNAVAILABLE.to_string())
    }
}
impl Connection {
    /// A connection to a fake host in tests.
    #[cfg(test)]
    pub fn for_test(origin: String) -> Self {
        Self { origin, setup_complete: true, key: "k".repeat(64) }
    }
}
pub const UNAVAILABLE: &str = "The library is unavailable. Your local recordings are safe.";
/// The `error` text of a host refusal body, bounded and single-line.
pub fn host_refusal(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body.get(..4096).unwrap_or(body)).ok()?;
    let text = value["error"].as_str()?.trim();
    (!text.is_empty() && text.len() <= 300 && !text.chars().any(char::is_control)).then(|| text.to_owned())
}
/// The only remote commands the toolbar accepts. Nothing here can start a take
/// or open an input: unknown names, including "record" and "prepare", are ignored.
pub fn command_code(command: Option<&str>) -> u8 {
    match command {
        Some("show") => 1,
        Some("quit") => 4,
        Some("preferences") => 7,
        Some("human") => 8,
        _ => 0,
    }
}
pub fn shape_name(shape: u8) -> &'static str {
    match shape {
        0 => "circle",
        1 => "rounded",
        3 => "wide",
        4 => "portrait",
        _ => "square",
    }
}
impl Connection {
    /// Free bytes at the recordings folder as the library reports them. None when unknown.
    pub fn free_bytes(&self) -> Option<u64> {
        self.action("state", json!({})).ok()?["freeBytes"].as_u64()
    }
}
// ---------------------------------------------------------------------------
// Take completion. The host keeps a take `active` until one native-finish or
// native-abort is stored, and deliberately restores `active` when that write
// fails. A finished capture whose completion did not land is therefore a
// pending operation with an identity, retried a bounded number of times and
// kept until the host confirms it. Nothing here deletes or rewrites media.
// ---------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub enum CompletionKind {
    Finish,
    Abort(String),
    /// A native-begin whose reply never arrived. Carries the attempt identity chosen before dispatch.
    Begin(String),
}
#[derive(Clone, Debug)]
pub struct PendingCompletion {
    pub id: Value,
    pub directory: std::path::PathBuf,
    kind: CompletionKind,
    pub attempts: u32,
    metadata_failures: u32,
    pub last_error: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completed {
    /// The host stored the finished take with its capture metadata.
    Saved,
    /// The host stored the take as interrupted (aborted, or details unreadable); media kept.
    Partial,
    /// The recording never started (the host confirmed it, or the unstarted take was closed). Nothing to save.
    NotStarted,
}
/// Reads of an unreadable capture.json before the take is stored as partial.
pub const MAX_METADATA_READS: u32 = 3;
pub type PendingSlot = std::sync::Arc<std::sync::Mutex<Option<PendingCompletion>>>;
/// The few facts about a take's capture the library page shows under Technical details. Numbers and short words only.
pub fn capture_summary(meta: &Value) -> Value {
    let number = |v: &Value| v.as_f64().filter(|n| n.is_finite() && *n >= 0.).map(|n| json!(n));
    let mut out = serde_json::Map::new();
    for (key, source) in [("fps", &meta["fps"]), ("width", &meta["width"]), ("height", &meta["height"]), ("frames", &meta["frames"]), ("outputFps", &meta["cadence"]["outputFps"]), ("sourceFps", &meta["source"]["arrivalFps"])] {
        if let Some(n) = number(source) {
            out.insert(key.into(), n);
        }
    }
    for (key, source) in [("finalization", &meta["finalization"]["status"]), ("status", &meta["status"])] {
        if let Some(text) = source.as_str().filter(|t| t.len() <= 24 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
            out.insert(key.into(), json!(text));
        }
    }
    if let Some(map) = meta["encoders"].as_object() {
        let clean: serde_json::Map<String, Value> = map.iter().filter(|(k, v)| ["screen", "camera", "recording"].contains(&k.as_str()) && v.as_str().is_some_and(|s| s.len() <= 32)).map(|(k, v)| (k.clone(), v.clone())).collect();
        if !clean.is_empty() {
            out.insert("encoders".into(), Value::Object(clean));
        }
    }
    Value::Object(out)
}
fn read_metadata(directory: &std::path::Path) -> Result<Value, String> {
    let bytes = std::fs::read(directory.join("capture.json")).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
impl PendingCompletion {
    pub fn finish(id: Value, directory: std::path::PathBuf) -> Self {
        Self { id, directory, kind: CompletionKind::Finish, attempts: 0, metadata_failures: 0, last_error: String::new() }
    }
    /// A begin that was sent but not confirmed. The id is unknown until the host says the take exists.
    pub fn begin(attempt: &str) -> Self {
        Self::finish(Value::Null, std::path::PathBuf::new()).with_kind(CompletionKind::Begin(attempt.to_owned()))
    }
    fn with_kind(mut self, kind: CompletionKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn abort(id: Value, directory: std::path::PathBuf, error: String) -> Self {
        Self { kind: CompletionKind::Abort(error), ..Self::finish(id, directory) }
    }
    pub fn kind(&self) -> &CompletionKind {
        &self.kind
    }
    /// One try. Success is returned only when the host confirmed the write, so
    /// "Saved" can never be shown for a take the library still calls active.
    pub fn attempt(&mut self, host: &mut dyn FnMut(&str, Value) -> Result<Value, String>) -> Result<Completed, String> {
        self.attempts += 1;
        if let CompletionKind::Begin(attempt) = self.kind.clone() {
            return self.settle_begin(&attempt, host);
        }
        let (op, data, outcome) = match &self.kind {
            CompletionKind::Finish => match read_metadata(&self.directory) {
                Ok(meta) => (
                    "native-finish",
                    json!({"id":self.id,"duration":meta["duration"],"timing":meta}),
                    Completed::Saved,
                ),
                Err(error) => {
                    self.metadata_failures += 1;
                    self.last_error = format!("Recording details couldn't be read. {error}");
                    if self.metadata_failures < MAX_METADATA_READS {
                        return Err(self.last_error.clone());
                    }
                    // Still unreadable: store what is on disk as an interrupted take.
                    (
                        "native-finish",
                        json!({"id":self.id,"partial":true,"error":"Recording details couldn't be read. Available files have been preserved."}),
                        Completed::Partial,
                    )
                }
            },
            CompletionKind::Abort(error) => {
                let mut data = json!({"id":self.id,"error":error});
                // What the take itself says about how far it got, if it wrote that down. Nothing is invented: a take
                // without readable details is reported as it always was.
                if let Ok(meta) = read_metadata(&self.directory) {
                    data["finalized"] = json!(meta["finalization"]["status"] == "completed");
                    data["capture"] = capture_summary(&meta);
                }
                ("native-abort", data, Completed::Partial)
            }
            CompletionKind::Begin(_) => unreachable!("handled above"),
        };
        match host(op, data) {
            Ok(_) => Ok(outcome),
            Err(error) if error.contains("No matching active recording") && self.settled(host) => Ok(outcome),
            Err(error) => {
                self.last_error = error.clone();
                Err(error)
            }
        }
    }
    /// Settles an unanswered begin without ever sending a new native-begin. The host answers atomically:
    /// cancelled (it never started and a late arrival is refused), completed (it began and ended), or active
    /// (the take exists but no capture is running, so it is closed as interrupted). Anything else, including
    /// an unreachable host, leaves it pending.
    fn settle_begin(&mut self, attempt: &str, host: &mut dyn FnMut(&str, Value) -> Result<Value, String>) -> Result<Completed, String> {
        let reply = host("native-begin-resolve", json!({"attempt":attempt})).map_err(|error| {
            self.last_error = error.clone();
            error
        })?;
        match reply["state"].as_str() {
            Some("cancelled" | "completed") => Ok(Completed::NotStarted),
            Some("active") => {
                // The take exists although capture never started: keep its identity and close it.
                self.id = reply["record"]["id"].clone();
                self.directory = reply["record"]["directory"].as_str().map(std::path::PathBuf::from).unwrap_or_default();
                let error = "The recording never started. Nothing was captured.".to_owned();
                self.kind = CompletionKind::Abort(error.clone());
                match host("native-abort", json!({"id":self.id,"error":error})) {
                    Ok(_) => Ok(Completed::NotStarted),
                    Err(e) if e.contains("No matching active recording") && self.settled(host) => Ok(Completed::NotStarted),
                    Err(e) => {
                        self.last_error = e.clone();
                        Err(e)
                    }
                }
            }
            _ => {
                self.last_error = "The Library gave no answer about the recording start.".into();
                Err(self.last_error.clone())
            }
        }
    }
    /// A reply can be lost after the host stored the take. The retry is then
    /// refused as "no matching active recording"; that is success only when the
    /// library shows this take present and no longer recording.
    fn settled(&self, host: &mut dyn FnMut(&str, Value) -> Result<Value, String>) -> bool {
        let Ok(state) = host("state", json!({})) else { return false };
        state["active"]["id"] != self.id
            && state["recordings"]
                .as_array()
                .is_some_and(|all| all.iter().any(|r| r["id"] == self.id && r["status"] != "recording"))
    }
}
pub fn new_attempt() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
#[derive(Debug)]
pub enum BeginOutcome {
    /// The host has the take (its own reply, or reconciled as active): capture may start.
    Started { id: Value, directory: Option<std::path::PathBuf> },
    /// Definitely not started: the host refused, or confirmed the attempt cancelled/ended. No capture.
    Refused(String),
    /// The reply never arrived and the host could not settle it yet: pending, no capture, no new begin.
    Unconfirmed(PendingCompletion, String),
}
const RESOLVE_DELAYS_MS: [u64; 3] = [0, 300, 900];
/// native-begin with an identity chosen before dispatch. An unanswered request (timeout, dropped or mangled
/// reply) is never repeated as a new take: the attempt is resolved with the host a bounded number of times.
pub fn begin_take(
    host: &mut dyn FnMut(&str, Value) -> Result<Value, String>,
    attempt: &str,
    mut request: Value,
    sleep: &mut dyn FnMut(std::time::Duration),
) -> BeginOutcome {
    request["attempt"] = json!(attempt);
    match host("native-begin", request) {
        Ok(take) => BeginOutcome::Started { id: take["id"].clone(), directory: take["directory"].as_str().map(std::path::PathBuf::from) },
        // The host answered and said no: nothing was started and nothing else is touched.
        Err(error) if error != UNAVAILABLE => BeginOutcome::Refused(error),
        Err(_) => {
            for delay in RESOLVE_DELAYS_MS {
                sleep(std::time::Duration::from_millis(delay));
                let Ok(reply) = host("native-begin-resolve", json!({"attempt":attempt})) else { continue };
                match reply["state"].as_str() {
                    Some("active") => {
                        return BeginOutcome::Started {
                            id: reply["record"]["id"].clone(),
                            directory: reply["record"]["directory"].as_str().map(std::path::PathBuf::from),
                        }
                    }
                    Some("cancelled" | "completed") => return BeginOutcome::Refused("The recording didn't start. Nothing was recorded.".into()),
                    _ => {}
                }
            }
            BeginOutcome::Unconfirmed(PendingCompletion::begin(attempt), "The Library didn't confirm the recording start. Nothing was recorded.".into())
        }
    }
}
/// Runs a take. A capture failure is returned as before. A completion the host
/// did not confirm is left in `pending` (an identity is parked before the begin
/// request is even sent, and replaced by an abort placeholder once the take
/// exists, so even a panicking worker leaves media-preserving recovery behind),
/// and is never reported as saved. Capture starts only once the host has the take.
pub fn record(
    connection: Option<Connection>,
    directory: std::path::PathBuf,
    camera: Option<std::sync::Arc<std::sync::Mutex<crate::camera_frame::Frame>>>,
    layout: std::sync::Arc<std::sync::Mutex<super::pipeline::Layout>>,
    options: super::capture::RecordOptions,
    pending: PendingSlot,
) -> Result<(), String> {
    let park = |value: Option<PendingCompletion>| {
        if let Ok(mut slot) = pending.lock() {
            *slot = value;
        }
    };
    let take = if let Some(conn) = &connection {
        let l = *layout.lock().map_err(|_| "Layout lock poisoned")?;
        let shape = shape_name(l.shape);
        let request = json!({"root":directory.parent().ok_or("Invalid recording directory")?,"preferences":{"camera":camera.is_some(),"microphone":options.microphone.is_some(),"systemAudio":false,"keepOriginals":options.originals,"quality":options.quality,"fps":options.fps,"mirror":l.mirror,"cameraShape":shape},"ownerPid":std::process::id()});
        let attempt = new_attempt();
        park(Some(PendingCompletion::begin(&attempt)));
        match begin_take(&mut |op, data| conn.action(op, data), &attempt, request, &mut |d| std::thread::sleep(d)) {
            BeginOutcome::Started { id, directory: reported } => Some((id, reported.unwrap_or_default())),
            BeginOutcome::Refused(error) => {
                park(None);
                return Err(error);
            }
            BeginOutcome::Unconfirmed(_, error) => return Err(error),
        }
    } else {
        None
    };
    let directory = take.as_ref().map(|(_, d)| d.clone()).filter(|d| !d.as_os_str().is_empty()).unwrap_or(directory);
    if let Some((id, _)) = &take {
        park(Some(PendingCompletion::abort(
            id.clone(),
            directory.clone(),
            "The recorder stopped unexpectedly. Available files have been preserved.".into(),
        )));
    }
    let result = super::capture::record_shared(directory.clone(), camera, layout, options)
        .map_err(|e| e.to_string());
    if let (Some(conn), Some((id, _))) = (&connection, take) {
        let mut completion = match &result {
            Ok(()) => PendingCompletion::finish(id, directory),
            Err(error) => PendingCompletion::abort(id, directory, error.clone()),
        };
        let confirmed = completion.attempt(&mut |op, data| conn.action(op, data)).is_ok();
        park((!confirmed).then_some(completion));
    }
    result
}

/// A person's capture command relayed by Home. Only what the owner-only `capture-command`
/// operation accepts exists here; there is no way to express any other action.
#[derive(Clone, Debug, PartialEq)]
pub enum HumanAction {
    Record,
    Stop,
    Cancel,
    Input { camera: bool, on: bool },
    Source { kind: String, id: Option<u32> },
    Device { camera: bool, uid: Option<String> },
    Shape(String),
    Size(u32),
    Mirror(bool),
    /// Whether the camera bubble is drawn at all, independent of the camera being on.
    ShowCamera(bool),
    Fps(u32),
    CapacityCheck,
    /// Opens the named System Settings privacy page; nothing is requested.
    OpenPrivacy(String),
    DismissNotice,
    ChooseWindow,
}
#[derive(Clone, Debug, PartialEq)]
pub struct HumanCommand {
    pub id: u64,
    /// When the host accepted it.
    pub issued_at: u64,
    /// Not valid after this instant, by this machine's clock.
    pub expires_at: u64,
    /// The app launch the control was made for.
    pub launch: String,
    /// Single use.
    pub nonce: String,
    /// The take number the control was made for (required for Record, Stop and Cancel).
    pub generation: Option<u64>,
    pub action: HumanAction,
}
/// Longest a relayed command may wait before it is ignored. Mirrors the host's time to live.
pub const HUMAN_TTL_MS: u64 = 5000;
/// Parse the `human` object of a `native-command` reply. A command that is malformed, from an
/// unknown vocabulary, or older than the time to live (by this machine's clock) yields None,
/// so a late delivery can never start anything.
pub fn parse_human(human: &Value, now_ms: u64) -> Option<HumanCommand> {
    let issued_at = human["issuedAt"].as_u64()?;
    let p = &human["payload"];
    let expires_at = p["expires"].as_u64()?;
    if now_ms.saturating_sub(issued_at) > HUMAN_TTL_MS || now_ms > expires_at {
        return None;
    }
    let token = |v: &Value| v.as_str().filter(|s| (8..=64).contains(&s.len())).map(str::to_owned);
    let (launch, nonce) = (token(&p["launch"])?, token(&p["nonce"])?);
    let generation = p["generation"].as_u64();
    let camera = |v: &Value| match v.as_str() {
        Some("camera") => Some(true),
        Some("mic") => Some(false),
        _ => None,
    };
    let action = match human["name"].as_str()? {
        "record" => HumanAction::Record,
        "stop" => HumanAction::Stop,
        "cancel" => HumanAction::Cancel,
        "input" => HumanAction::Input { camera: camera(&p["kind"])?, on: p["on"].as_bool()? },
        "device" => HumanAction::Device {
            camera: camera(&p["kind"])?,
            uid: match &p["uid"] {
                Value::Null => None,
                v => Some(v.as_str()?.to_owned()),
            },
        },
        "source" => {
            let kind = p["kind"].as_str().filter(|k| ["display", "window", "default"].contains(k))?;
            let id = if kind == "default" { None } else { Some(u32::try_from(p["id"].as_u64()?).ok()?) };
            HumanAction::Source { kind: kind.to_owned(), id }
        }
        "shape" => HumanAction::Shape(p["value"].as_str().filter(|v| ["circle", "rounded", "square", "wide", "portrait"].contains(v))?.to_owned()),
        "size" => HumanAction::Size(u32::try_from(p["value"].as_u64()?).ok()?),
        "mirror" => HumanAction::Mirror(p["value"].as_bool()?),
        "show-camera" => HumanAction::ShowCamera(p["value"].as_bool()?),
        "open-privacy" => HumanAction::OpenPrivacy(p["kind"].as_str().filter(|k| ["screen", "camera", "mic"].contains(k))?.to_owned()),
        "fps" => HumanAction::Fps(p["value"].as_u64().filter(|n| [24, 30, 60].contains(n))? as u32),
        "capacity-check" => HumanAction::CapacityCheck,
        "dismiss-notice" => HumanAction::DismissNotice,
        "choose-window" => HumanAction::ChooseWindow,
        _ => return None,
    };
    Some(HumanCommand { id: human["id"].as_u64().unwrap_or(0), issued_at, expires_at, launch, nonce, generation, action })
}
pub type HumanInbox = std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<HumanCommand>>>;
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
/// Latest-value publisher of the app's capture state to Home. The caller never blocks: a new
/// value replaces any unsent one, a slow or absent host loses only stale values, and the
/// worker ends when the publisher is dropped.
#[derive(Clone)]
pub struct Publisher {
    slot: std::sync::Arc<(std::sync::Mutex<Option<Value>>, std::sync::Condvar)>,
}
impl Publisher {
    pub fn publish(&self, value: Value) {
        *self.slot.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
        self.slot.1.notify_one();
    }
    #[cfg(test)]
    pub fn take_for_test(&self) -> Option<Value> {
        self.slot.0.lock().unwrap().take()
    }
    #[cfg(test)]
    pub fn detached() -> Self {
        Self { slot: Default::default() }
    }
}
pub fn publisher(connection: Connection) -> Publisher {
    use std::time::{Duration, Instant};
    let slot: std::sync::Arc<(std::sync::Mutex<Option<Value>>, std::sync::Condvar)> = Default::default();
    let worker = slot.clone();
    std::thread::spawn(move || {
        let mut last = Instant::now() - Duration::from_secs(1);
        loop {
            let next = {
                let mut held = worker.0.lock().unwrap_or_else(|e| e.into_inner());
                while held.is_none() {
                    if std::sync::Arc::strong_count(&worker) == 1 {
                        return;
                    }
                    held = worker.1.wait_timeout(held, Duration::from_secs(1)).unwrap_or_else(|e| e.into_inner()).0;
                }
                held.take()
            };
            if let Some(value) = next {
                // At most four posts a second; terminal states are never lost because the latest value is kept.
                if let Some(wait) = Duration::from_millis(250).checked_sub(last.elapsed()) {
                    std::thread::sleep(wait);
                }
                let value = worker.0.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or(value);
                last = Instant::now();
                let _ = connection.action("native-progress", value);
            }
        }
    });
    Publisher { slot }
}
pub fn commands(
    connection: Connection,
    ctx: eframe::egui::Context,
) -> (std::sync::Arc<std::sync::atomic::AtomicU8>, HumanInbox) {
    use std::sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    };
    let command = Arc::new(AtomicU8::new(0));
    let result = command.clone();
    let inbox: HumanInbox = Default::default();
    let queue = inbox.clone();
    std::thread::spawn(move || {
        let mut offline = false;
        loop {
            if Arc::strong_count(&result) == 1 {
                break;
            }
            match connection.action("native-command", json!({"pid":std::process::id()})) {
                Ok(value) => {
                    if offline {
                        offline = false;
                        result.store(6, Ordering::Relaxed);
                        ctx.request_repaint();
                    }
                    let number = command_code(value["command"].as_str());
                    if number == 8 {
                        if let Some(c) = parse_human(&value["human"], now_ms()) {
                            let mut held = queue.lock().unwrap_or_else(|e| e.into_inner());
                            if held.len() < 16 {
                                held.push_back(c);
                            }
                            drop(held);
                            ctx.request_repaint();
                        }
                    } else if number > 0 {
                        result.store(number, Ordering::Relaxed);
                        ctx.request_repaint();
                        if number == 4 {
                            break;
                        }
                    }
                }
                Err(_) => {
                    if !offline {
                        result.store(5, Ordering::Relaxed);
                        ctx.request_repaint();
                        offline = true;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
            }
        }
    });
    (command, inbox)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_commands_can_show_quit_or_refresh_but_never_record_or_prepare() {
        assert_eq!(command_code(Some("show")), 1);
        assert_eq!(command_code(Some("quit")), 4);
        assert_eq!(command_code(Some("preferences")), 7);
        for name in ["record", "prepare", "prepare-recording", "camera", "start", "idle", ""] {
            assert_eq!(command_code(Some(name)), 0, "{name}");
        }
        assert_eq!(command_code(None), 0);
    }
    #[test]
    fn relayed_commands_parse_only_the_owner_vocabulary_carry_their_session_and_expire() {
        let now = 1_000_000;
        let h = |name: &str, payload: Value| {
            let mut p = json!({"launch":"launch-aaaa","nonce":"nonce-0001","expires":now + 4000,"generation":4});
            for (k, v) in payload.as_object().unwrap() {
                p[k] = v.clone();
            }
            json!({"id":1,"name":name,"payload":p,"issuedAt":now - 100})
        };
        let record = parse_human(&h("record", json!({})), now).unwrap();
        assert_eq!((record.action.clone(), record.launch.as_str(), record.nonce.as_str(), record.generation, record.expires_at), (HumanAction::Record, "launch-aaaa", "nonce-0001", Some(4), now + 4000));
        assert_eq!(parse_human(&h("stop", json!({"generation":4})), now).unwrap().action, HumanAction::Stop);
        assert_eq!(parse_human(&h("cancel", json!({})), now).unwrap().action, HumanAction::Cancel);
        assert_eq!(parse_human(&h("show-camera", json!({"value":false})), now).unwrap().action, HumanAction::ShowCamera(false));
        assert_eq!(parse_human(&h("open-privacy", json!({"kind":"screen"})), now).unwrap().action, HumanAction::OpenPrivacy("screen".into()));
        assert!(parse_human(&h("open-privacy", json!({"kind":"files"})), now).is_none(), "only the three privacy pages");
        assert!(parse_human(&h("show-camera", json!({"value":"yes"})), now).is_none());
        assert_eq!(parse_human(&h("input", json!({"kind":"mic","on":true})), now).unwrap().action, HumanAction::Input { camera: false, on: true });
        assert_eq!(parse_human(&h("device", json!({"kind":"camera","uid":null})), now).unwrap().action, HumanAction::Device { camera: true, uid: None });
        assert_eq!(parse_human(&h("source", json!({"kind":"display","id":3})), now).unwrap().action, HumanAction::Source { kind: "display".into(), id: Some(3) });
        assert_eq!(parse_human(&h("source", json!({"kind":"default"})), now).unwrap().action, HumanAction::Source { kind: "default".into(), id: None });
        assert_eq!(parse_human(&h("shape", json!({"value":"portrait"})), now).unwrap().action, HumanAction::Shape("portrait".into()));
        for bad in [h("prepare-recording", json!({})), h("input", json!({"kind":"screen","on":true})), h("source", json!({"kind":"window"})),
                    h("shape", json!({"value":"oval"})), h("fps", json!({"value":120})), h("device", json!({"kind":"mic","uid":7})), json!({"name":"record"}),
                    h("record", json!({"launch":null})), h("record", json!({"nonce":"x"})), h("record", json!({"expires":"soon"}))] {
            assert!(parse_human(&bad, now).is_none(), "{bad}");
        }
        // Older than the host's time to live, or past its own expiry, by this machine's clock: ignored.
        let old = json!({"id":1,"name":"record","payload":{"launch":"launch-aaaa","nonce":"nonce-0001","expires":now + 9000},"issuedAt":now - HUMAN_TTL_MS - 1});
        assert!(parse_human(&old, now).is_none());
        let mut late = h("record", json!({}));
        late["payload"]["expires"] = json!(now - 1);
        assert!(parse_human(&late, now).is_none());
        late["payload"]["expires"] = json!(now);
        assert!(parse_human(&late, now).is_some(), "valid up to and including the expiry instant");
    }
    #[test]
    fn the_publisher_keeps_only_the_latest_value_and_never_blocks_the_caller() {
        let p = Publisher::detached();
        p.publish(json!({"n":1}));
        p.publish(json!({"n":2}));
        assert_eq!(p.take_for_test().unwrap()["n"], 2);
        assert!(p.take_for_test().is_none());
    }
    #[test]
    fn shapes_keep_legacy_square_and_add_wide() {
        assert_eq!(
            [0, 1, 2, 3, 4].map(shape_name),
            ["circle", "rounded", "square", "wide", "portrait"]
        );
    }

    // --- completion reconciliation ----------------------------------------------------------------
    fn take_dir(name: &str, metadata: Option<&str>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("openrecorder-completion-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("recording.mp4"), b"combined media").unwrap();
        std::fs::write(dir.join("screen.mp4"), b"screen media").unwrap();
        if let Some(text) = metadata {
            std::fs::write(dir.join("capture.json"), text).unwrap();
        }
        dir
    }
    /// A host fake: records calls, fails the first `failures` completion writes the way a
    /// full disk does (the host restores `active`), then stores the take.
    struct FakeHost {
        calls: Vec<(String, Value)>,
        failures: usize,
        stored: bool,
        lose_reply: bool,
    }
    impl FakeHost {
        fn new(failures: usize) -> Self {
            Self { calls: vec![], failures, stored: false, lose_reply: false }
        }
        fn call(&mut self, id: &Value, op: &str, data: Value) -> Result<Value, String> {
            self.calls.push((op.to_owned(), data));
            match op {
                "state" => Ok(json!({"active":if self.stored {Value::Null} else {json!({"id":id})},
                    "recordings":[{"id":id,"status":if self.stored {"ready"} else {"recording"}}]})),
                _ if self.stored => Err("No matching active recording".into()),
                _ if self.failures > 0 => {
                    self.failures -= 1;
                    Err("No space left on device".into())
                }
                _ => {
                    self.stored = true;
                    if self.lose_reply {
                        Err(UNAVAILABLE.into())
                    } else {
                        Ok(json!({"status":"ready"}))
                    }
                }
            }
        }
    }
    const METADATA: &str = r#"{"duration":12.5,"frames":375,"status":"completed"}"#;

    #[test]
    fn a_failed_save_stays_pending_with_its_identity_and_is_never_reported_saved() {
        let dir = take_dir("failed-save", Some(METADATA));
        let id = json!("take-1");
        let mut host = FakeHost::new(2);
        let mut pending = PendingCompletion::finish(id.clone(), dir.clone());
        for expected_attempts in 1..=2 {
            let result = pending.attempt(&mut |op, data| host.call(&id, op, data));
            assert_eq!(result.unwrap_err(), "No space left on device");
            assert_eq!(pending.attempts, expected_attempts);
            assert!(!host.stored);
        }
        // The same operation, same identity and same payload, succeeds once the disk has room.
        let result = pending.attempt(&mut |op, data| host.call(&id, op, data));
        assert_eq!(result, Ok(Completed::Saved));
        let finishes: Vec<&Value> = host.calls.iter().filter(|(op, _)| op == "native-finish").map(|(_, d)| d).collect();
        assert_eq!(finishes.len(), 3);
        assert!(finishes.iter().all(|d| d["id"] == id && d["duration"] == 12.5 && d["timing"]["frames"] == 375));
        assert!(host.calls.iter().all(|(op, _)| op == "native-finish"), "nothing else was called: {:?}", host.calls);
        assert!(dir.join("recording.mp4").exists() && dir.join("screen.mp4").exists(), "media untouched");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn unreadable_metadata_is_retried_then_stored_as_partial_without_losing_media() {
        for (name, content) in [("missing", None), ("garbage", Some("{not json")), ("truncated", Some(r#"{"duration":1"#))] {
            let dir = take_dir(name, content);
            let id = json!("take-2");
            let mut host = FakeHost::new(0);
            let mut pending = PendingCompletion::finish(id.clone(), dir.clone());
            for _ in 1..MAX_METADATA_READS {
                let error = pending.attempt(&mut |op, data| host.call(&id, op, data)).unwrap_err();
                assert!(error.starts_with("Recording details couldn't be read."), "{name}: {error}");
                assert!(host.calls.is_empty(), "{name}: the host must not hear about an unreadable take yet");
            }
            // A metadata file that appears later is used, not skipped.
            if name == "garbage" {
                std::fs::write(dir.join("capture.json"), METADATA).unwrap();
                assert_eq!(pending.attempt(&mut |op, data| host.call(&id, op, data)), Ok(Completed::Saved));
                std::fs::remove_dir_all(dir).unwrap();
                continue;
            }
            let result = pending.attempt(&mut |op, data| host.call(&id, op, data));
            assert_eq!(result, Ok(Completed::Partial), "{name}");
            let (op, data) = &host.calls[0];
            assert_eq!(op, "native-finish");
            assert_eq!((data["partial"].as_bool(), &data["id"]), (Some(true), &id));
            assert!(data.get("timing").is_none() && data.get("duration").is_none(), "no invented timing");
            assert!(dir.join("recording.mp4").exists() && dir.join("screen.mp4").exists(), "{name}: media kept");
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
    #[test]
    fn a_failed_abort_is_retried_too_and_never_discarded() {
        let dir = take_dir("abort", None);
        let id = json!("take-3");
        let mut host = FakeHost::new(1);
        let mut pending = PendingCompletion::abort(id.clone(), dir.clone(), "Camera disconnected".into());
        assert!(pending.attempt(&mut |op, data| host.call(&id, op, data)).is_err());
        assert_eq!(pending.kind(), &CompletionKind::Abort("Camera disconnected".into()));
        assert_eq!(pending.attempt(&mut |op, data| host.call(&id, op, data)), Ok(Completed::Partial));
        assert!(host.calls.iter().all(|(op, d)| op == "native-abort" && d["error"] == "Camera disconnected" && d["id"] == id));
        assert!(dir.join("recording.mp4").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_lost_reply_is_settled_only_when_the_library_shows_the_take_stored() {
        let dir = take_dir("lost-reply", Some(METADATA));
        let id = json!("take-4");
        let mut host = FakeHost::new(0);
        host.lose_reply = true;
        let mut pending = PendingCompletion::finish(id.clone(), dir.clone());
        // The host stored the take but the answer never arrived: unreachable, so still pending.
        assert_eq!(pending.attempt(&mut |op, data| host.call(&id, op, data)).unwrap_err(), UNAVAILABLE);
        assert!(host.stored);
        // The retry is refused as "no matching active recording"; the state shows it stored.
        assert_eq!(pending.attempt(&mut |op, data| host.call(&id, op, data)), Ok(Completed::Saved));
        // A different take that merely isn't active is NOT proof: this one is unknown to the library.
        let other = json!("other");
        let mut stranger = PendingCompletion::finish(other.clone(), dir.clone());
        let mut calls = 0;
        let result = stranger.attempt(&mut |op, _| {
            calls += 1;
            match op {
                "state" => Ok(json!({"active":null,"recordings":[]})),
                _ => Err("No matching active recording".into()),
            }
        });
        assert_eq!(result.unwrap_err(), "No matching active recording");
        assert_eq!(calls, 2);
        // And an unreachable library never counts as settled.
        let mut unreachable = PendingCompletion::finish(other, dir.clone());
        let result = unreachable.attempt(&mut |_, _| Err("No matching active recording".to_string()));
        assert!(result.is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn host_refusals_keep_their_short_reason_and_transport_failures_stay_generic() {
        assert_eq!(host_refusal(r#"{"error":"No matching active recording"}"#).as_deref(), Some("No matching active recording"));
        assert_eq!(host_refusal(r#"{"error":"  "}"#), None);
        assert_eq!(host_refusal("<html>"), None);
        assert_eq!(host_refusal(&format!(r#"{{"error":"{}"}}"#, "x".repeat(400))), None);
        assert_eq!(host_refusal("{\"error\":\"bad\\u0007\"}"), None);
    }

    // --- native-begin identity ----------------------------------------------------------------------
    type Calls = Vec<(String, Value)>;
    /// A scripted host: each call is answered from the queue for that op; every call is recorded.
    fn scripted(calls: &mut Calls, answers: &mut std::collections::HashMap<&'static str, Vec<Result<Value, String>>>, op: &str, data: Value) -> Result<Value, String> {
        calls.push((op.to_owned(), data));
        let queue = answers.get_mut(op).unwrap_or_else(|| panic!("unexpected op {op}"));
        if queue.len() > 1 { queue.remove(0) } else { queue[0].clone() }
    }
    fn ops(calls: &Calls) -> Vec<&str> {
        calls.iter().map(|(op, _)| op.as_str()).collect()
    }
    const ATTEMPT: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn a_begin_carries_its_identity_and_a_confirmed_reply_starts_capture() {
        let mut calls = Calls::new();
        let mut answers = std::collections::HashMap::from([("native-begin", vec![Ok(json!({"id":"take-1","directory":"/media/take-1"}))])]);
        let outcome = begin_take(&mut |op, d| scripted(&mut calls, &mut answers, op, d), ATTEMPT, json!({"root":"/media"}), &mut |_| {});
        assert!(matches!(&outcome, BeginOutcome::Started { id, directory } if id == "take-1" && directory.as_deref() == Some(std::path::Path::new("/media/take-1"))), "{outcome:?}");
        assert_eq!(calls[0].1["attempt"], ATTEMPT, "identity exists before dispatch");
        assert_eq!(calls[0].1["root"], "/media");
        assert_eq!(ops(&calls), vec!["native-begin"]);
    }
    #[test]
    fn a_host_refusal_is_final_and_never_resolves_or_touches_another_take() {
        let mut calls = Calls::new();
        let mut answers = std::collections::HashMap::from([("native-begin", vec![Err("A recording is already active".to_string())])]);
        let outcome = begin_take(&mut |op, d| scripted(&mut calls, &mut answers, op, d), ATTEMPT, json!({}), &mut |_| panic!("no waiting"));
        assert!(matches!(outcome, BeginOutcome::Refused(ref e) if e == "A recording is already active"));
        assert_eq!(ops(&calls), vec!["native-begin"], "no resolve, no abort of the other take");
    }
    #[test]
    fn an_unanswered_begin_that_the_host_confirms_cancelled_starts_nothing_and_is_not_repeated() {
        for state in ["cancelled", "completed"] {
            let mut calls = Calls::new();
            let mut answers = std::collections::HashMap::from([
                ("native-begin", vec![Err(UNAVAILABLE.to_string())]),
                ("native-begin-resolve", vec![Ok(json!({"state":state}))]),
            ]);
            let outcome = begin_take(&mut |op, d| scripted(&mut calls, &mut answers, op, d), ATTEMPT, json!({}), &mut |_| {});
            assert!(matches!(outcome, BeginOutcome::Refused(ref e) if e.contains("Nothing was recorded")), "{state}: {outcome:?}");
            assert_eq!(ops(&calls), vec!["native-begin", "native-begin-resolve"], "never a second begin");
            assert_eq!(calls[1].1["attempt"], ATTEMPT);
        }
    }
    #[test]
    fn an_unanswered_begin_the_host_did_commit_is_reconciled_to_that_take() {
        let mut calls = Calls::new();
        let mut waits = Vec::new();
        let mut answers = std::collections::HashMap::from([
            ("native-begin", vec![Err(UNAVAILABLE.to_string())]),
            ("native-begin-resolve", vec![Err(UNAVAILABLE.to_string()), Ok(json!({"state":"active","record":{"id":"take-7","directory":"/media/take-7"}}))]),
        ]);
        let outcome = begin_take(&mut |op, d| scripted(&mut calls, &mut answers, op, d), ATTEMPT, json!({}), &mut |d| waits.push(d.as_millis()));
        assert!(matches!(&outcome, BeginOutcome::Started { id, .. } if id == "take-7"), "{outcome:?}");
        assert_eq!(ops(&calls), vec!["native-begin", "native-begin-resolve", "native-begin-resolve"]);
        assert_eq!(waits, vec![0, 300], "bounded, short waits");
    }
    #[test]
    fn an_unsettled_begin_stays_pending_with_its_identity_and_never_replays_the_begin() {
        let mut calls = Calls::new();
        let mut waits = Vec::new();
        let mut answers = std::collections::HashMap::from([
            ("native-begin", vec![Err(UNAVAILABLE.to_string())]),
            ("native-begin-resolve", vec![Err(UNAVAILABLE.to_string())]),
        ]);
        let outcome = begin_take(&mut |op, d| scripted(&mut calls, &mut answers, op, d), ATTEMPT, json!({}), &mut |d| waits.push(d.as_millis()));
        let BeginOutcome::Unconfirmed(mut pending, message) = outcome else { panic!("expected pending") };
        assert!(message.contains("Nothing was recorded") && !message.to_lowercase().contains("saved"));
        assert_eq!(ops(&calls).iter().filter(|o| **o == "native-begin").count(), 1);
        assert_eq!(ops(&calls).iter().filter(|o| **o == "native-begin-resolve").count(), 3, "bounded");
        assert_eq!(waits, vec![0, 300, 900]);
        assert_eq!(pending.kind(), &CompletionKind::Begin(ATTEMPT.into()));
        // Later: the host says it never started -> nothing to save, and no begin is ever sent.
        let mut later = Calls::new();
        let mut cancelled = std::collections::HashMap::from([("native-begin-resolve", vec![Ok(json!({"state":"cancelled"}))])]);
        assert_eq!(pending.attempt(&mut |op, d| scripted(&mut later, &mut cancelled, op, d)), Ok(Completed::NotStarted));
        assert_eq!(ops(&later), vec!["native-begin-resolve"]);
    }
    #[test]
    fn a_pending_begin_that_turns_out_active_closes_that_take_as_interrupted_and_retries_the_close() {
        let dir = std::env::temp_dir().join(format!("openrecorder-begin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("recording.mp4"), b"media").unwrap();
        let record = json!({"id":"take-9","directory":dir});
        let mut pending = PendingCompletion::begin(ATTEMPT);
        let mut calls = Calls::new();
        let mut answers = std::collections::HashMap::from([
            ("native-begin-resolve", vec![Ok(json!({"state":"active","record":record}))]),
            ("native-abort", vec![Err("No space left on device".to_string()), Ok(json!({"status":"interrupted"}))]),
        ]);
        let first = pending.attempt(&mut |op, d| scripted(&mut calls, &mut answers, op, d));
        assert_eq!(first.unwrap_err(), "No space left on device");
        // The identity is now the take itself: the retry is the abort alone (no resolve, no begin).
        assert!(matches!(pending.kind(), CompletionKind::Abort(_)) && pending.id == "take-9");
        let second = pending.attempt(&mut |op, d| scripted(&mut calls, &mut answers, op, d));
        assert_eq!(second, Ok(Completed::Partial));
        assert_eq!(ops(&calls), vec!["native-begin-resolve", "native-abort", "native-abort"]);
        assert!(calls[1].1["id"] == "take-9" && calls[2].1["id"] == "take-9");
        assert!(dir.join("recording.mp4").exists(), "media untouched");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_mangled_reply_body_is_ambiguous_not_a_refusal() {
        // A real local server answers 200 with a body that is not JSON.
        let server = std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let background = server.clone();
        let thread = std::thread::spawn(move || {
            if let Ok(request) = background.recv() {
                let _ = request.respond(tiny_http::Response::from_string("{\"id\":"));
            }
        });
        let conn = Connection::for_test(format!("http://127.0.0.1:{port}"));
        assert_eq!(conn.action("native-begin", json!({})).unwrap_err(), UNAVAILABLE);
        thread.join().unwrap();
    }
}
