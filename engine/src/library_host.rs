//! Local library owned by the native runtime, independent of a chat or launcher.
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tiny_http::{Header, Response, Server, StatusCode};
static QUITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn should_quit() -> bool {
    QUITTING.load(std::sync::atomic::Ordering::Relaxed)
}
// The per-home OS lease is held for the whole life of the host process (see `start`). It is released by the
// operating system if this process dies, so a crashed host never blocks recovery and a slow one is never replaced.
static LEASE: Mutex<Option<fs::File>> = Mutex::new(None);
fn lease_path(home: &Path) -> PathBuf {
    home.join("rust-host.lease")
}
fn open_lease(home: &Path) -> Result<fs::File, String> {
    let mut opts = fs::OpenOptions::new();
    opts.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(lease_path(home)).map_err(|e| e.to_string())
}
// True while another live process owns this home (running, starting, quitting, stalled or busy).
fn lease_held_elsewhere(home: &Path) -> bool {
    let Ok(file) = open_lease(home) else { return false };
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => {
            let _ = fs2::FileExt::unlock(&file);
            false
        }
        Err(_) => true,
    }
}
// What this host published, so the owner can restore it if the files are removed or damaged while it runs. The
// lease means no other host can have replaced them, and clients cannot otherwise find a live owner.
static PUBLISHED: Mutex<Option<Value>> = Mutex::new(None);
static LAST_MAINTAIN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Called from the host's existing main loop. At most every two seconds it restores missing or foreign descriptor
/// files; it starts no thread and does nothing once Quit was requested.
pub fn maintain() {
    let t = now();
    if should_quit() || t.saturating_sub(LAST_MAINTAIN.load(std::sync::atomic::Ordering::Relaxed)) < 2000 {
        return;
    }
    LAST_MAINTAIN.store(t, std::sync::atomic::Ordering::Relaxed);
    let Some(mut config) = PUBLISHED.lock().ok().and_then(|p| p.clone()) else { return };
    let h = home();
    config["setup_complete"] = json!(read(&h.join("onboarding.json"), Value::Null).is_ok_and(|v| v["completed"] == true));
    for name in ["rust-runtime.json", "rust-connection.json"] {
        let path = h.join(name);
        let intact = read(&path, Value::Null)
            .is_ok_and(|v| v["pid"] == config["pid"] && v["origin"] == config["origin"] && v["key"] == config["key"]);
        if !intact {
            let _ = save(&path, &config);
        }
    }
}
pub fn shutdown() {
    let h = home();
    for name in ["rust-runtime.json", "rust-connection.json"] {
        let p = h.join(name);
        if read(&p, Value::Null).ok().and_then(|v| v["pid"].as_u64())
            == Some(std::process::id() as u64)
        {
            let _ = fs::remove_file(p);
        }
    }
    // Descriptors go first; only then may a successor take over the lease and publish its own.
    if let Ok(mut lease) = LEASE.lock() {
        if let Some(file) = lease.take() {
            let _ = fs2::FileExt::unlock(&file);
        }
    }
}
const VERSION: &str = "0.6.0-preview.11";
// The only operations an agent may request through MCP. Every `native-*` op is deliberately absent. No
// operation, here or over HTTP, asks the toolbar to activate camera or microphone.
const PUBLIC_OPS: &[&str] = &["state", "open-studio", "open-library", "settings", "folder", "update", "stop", "reveal", "open-file", "share", "unshare", "quit"];
// The resource address carries the release, so a host that cached an earlier Home template across an upgrade asks for
// this one by its new name instead of showing the old page.
const URI: &str = concat!("ui://openrecorder/home-v", env!("CARGO_PKG_VERSION"));
/// What an agent host calls this server and its tools: a plain black circle (monochrome, drawn in `currentColor` so the
/// host colours it like its own icons) and the name OpenRecorder, never an operation name.
const TITLE: &str = "OpenRecorder";
const ICON_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><circle cx="8" cy="8" r="5.5" fill="currentColor"/></svg>"#;
fn mime_of(name: &str) -> &'static str {
    if name.ends_with("jpg") {
        "image/jpeg"
    } else if name.ends_with("webm") {
        "video/webm"
    } else if name.ends_with("mov") {
        "video/quicktime"
    } else if name.ends_with("json") {
        "application/json"
    } else {
        "video/mp4"
    }
}
/// Headers a response to the scoped media capability carries so an embedded page (an opaque origin inside an agent
/// host, possibly cross-origin-isolated) may use the bytes it was given the address of. They apply only when the
/// request itself presented the media capability; nothing is granted to anyone else.
fn media_cors() -> Vec<Header> {
    vec![
        header("Cross-Origin-Resource-Policy", "cross-origin"),
        header("Access-Control-Allow-Origin", "*"),
        header("Access-Control-Expose-Headers", "Content-Range, Accept-Ranges, Content-Length, Content-Type"),
    ]
}
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}
fn icons() -> Value {
    json!([{"src":format!("data:image/svg+xml;base64,{}", base64(ICON_SVG.as_bytes())),"mimeType":"image/svg+xml","sizes":["any"]}])
}
fn server_info() -> Value {
    json!({"name":"OpenRecorder","title":TITLE,"version":VERSION,"icons":icons()})
}
// Operations only the library page itself needs. They are owner operations, never in `PUBLIC_OPS`: an agent-visible
// tool cannot name them, and the app-only tool that carries them is hidden from the model by its `visibility`.
const UI_OPS: &[&str] = &["media-read", "media-session", "capture-command", "start-recording"];
/// Largest piece of a file one media call returns (base64 grows it by a third). A poster is one piece.
const MEDIA_CHUNK: u64 = 192 * 1024;
// Initialize accepts either dated revision. Anything else is answered with the latest one.
const SUPPORTED_VERSIONS: &[&str] = &["2025-11-25", "2026-07-28"];
const LATEST_VERSION: &str = "2026-07-28";
const MAX_MESSAGE: usize = 131072;
// Returned (HTTP 503) once Quit was accepted so that bridges and starters stop adopting this host.
const QUIT_MESSAGE: &str = "OpenRecorder is quitting";
/// What the OS says about a process. Only `Gone` is a confirmed death; anything uncertain is `Unknown`, and
/// neither `Alive` nor `Unknown` may ever clear an active take or drop its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Liveness {
    Alive,
    Gone,
    Unknown,
}
/// Result of the POSIX zero-signal probe `kill(pid, 0)`: 0 = exists, ESRCH = gone, EPERM = exists but not
/// ours to signal, any other error = can't tell. Pids that are 0 or don't fit a positive `pid_t` are rejected
/// before the call, because 0 and negative values address process groups, not a process.
#[allow(dead_code)]
fn classify_posix_probe(pid: u32, probe: impl FnOnce(i32) -> Result<(), i32>) -> Liveness {
    const EPERM: i32 = 1;
    const ESRCH: i32 = 3;
    let Some(pid) = i32::try_from(pid).ok().filter(|p| *p > 0) else {
        return Liveness::Unknown;
    };
    match probe(pid) {
        Ok(()) => Liveness::Alive,
        Err(ESRCH) => Liveness::Gone,
        Err(EPERM) => Liveness::Alive,
        Err(_) => Liveness::Unknown,
    }
}
/// What opening a Windows process handle and reading its exit code reported.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowsProbe {
    /// A handle was opened; the exit code if it could be read.
    Opened(Option<u32>),
    /// OpenProcess failed with this GetLastError value.
    OpenFailed(u32),
}
/// ERROR_INVALID_PARAMETER from OpenProcess means no such pid; ERROR_ACCESS_DENIED means it exists. A process
/// that reports STILL_ACTIVE (259) is alive, and a readable other exit code means it has exited. Everything
/// else, including an unreadable exit code, is unknown. Pid 0 is the idle pseudo-process, never a native.
#[allow(dead_code)]
fn classify_windows_probe(pid: u32, probe: WindowsProbe) -> Liveness {
    const STILL_ACTIVE: u32 = 259;
    const ERROR_ACCESS_DENIED: u32 = 5;
    const ERROR_INVALID_PARAMETER: u32 = 87;
    if pid == 0 {
        return Liveness::Unknown;
    }
    match probe {
        WindowsProbe::Opened(Some(STILL_ACTIVE)) => Liveness::Alive,
        WindowsProbe::Opened(Some(_)) => Liveness::Gone,
        WindowsProbe::Opened(None) => Liveness::Unknown,
        WindowsProbe::OpenFailed(ERROR_INVALID_PARAMETER) => Liveness::Gone,
        WindowsProbe::OpenFailed(ERROR_ACCESS_DENIED) => Liveness::Alive,
        WindowsProbe::OpenFailed(_) => Liveness::Unknown,
    }
}
/// One native, non-blocking OS query by pid (no helper process, so nothing can hang while the store is locked).
fn process_liveness(pid: u32) -> Liveness {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        classify_posix_probe(pid, |pid| {
            // SAFETY: signal 0 only checks that the pid exists; `pid` is a positive pid_t.
            if unsafe { kill(pid, 0) } == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(-1))
            }
        })
    }
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
            fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
            fn CloseHandle(handle: *mut c_void) -> i32;
        }
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        // SAFETY: the handle is checked for null, used only for the exit-code query and closed once.
        let probe = unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                WindowsProbe::OpenFailed(std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32)
            } else {
                let mut code = 0u32;
                let read = GetExitCodeProcess(handle, &mut code) != 0;
                CloseHandle(handle);
                WindowsProbe::Opened(read.then_some(code))
            }
        };
        classify_windows_probe(pid, probe)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        Liveness::Unknown
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn read(p: &Path, fallback: Value) -> Result<Value, String> {
    match fs::read(p) {
        Ok(v) => serde_json::from_slice(&v).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(fallback),
        Err(e) => Err(e.to_string()),
    }
}
fn save(p: &Path, v: &Value) -> Result<(), String> {
    let tmp = p.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(v.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    fs::rename(tmp, p).map_err(|e| e.to_string())
}
pub fn home() -> PathBuf {
    if let Ok(v) = std::env::var("OPENRECORDER_HOME") {
        return v.into();
    }
    let h = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        PathBuf::from(h).join("Library/Application Support/OpenRecorder/Native")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or(h)).join("OpenRecorder/Native")
    } else {
        PathBuf::from(std::env::var("XDG_DATA_HOME").unwrap_or(format!("{h}/.local/share")))
            .join("openrecorder/native")
    }
}
fn defaults() -> Value {
    json!({"quality":1920,"fps":30,"camera":false,"microphone":false,"systemAudio":false,"keepOriginals":true,"cameraSize":168,"cameraShape":"rounded","mirror":true,"showCamera":true,"rememberInputs":true,"countdownSeconds":3,"fpsChosen":false,"source":"display","theme":"system","cameraDeviceId":null,"microphoneDeviceId":null})
}
// Accepted values, kept in one place so validation, migration and the contract document agree. The old camera
// sizes and the three older shapes stay valid next to the new ones.
const CAMERA_SIZES: [u64; 6] = [120, 168, 224, 140, 200, 280];
const CAMERA_SHAPES: [&str; 5] = ["circle", "rounded", "square", "wide", "portrait"];
const COUNTDOWN_SECONDS: [u64; 2] = [0, 3];
/// Defaults plus whatever was stored, with the legacy `countdown` key folded into `countdownSeconds` only when
/// the new key is absent (or damaged). A damaged `rememberInputs` becomes false, never true: an ambiguous value must
/// not make the native app restore remembered camera or microphone intent. Explicit invalid *requests* are
/// rejected by the `settings` operation, never coerced.
fn normalize_preferences(stored: &Value) -> Value {
    let mut p = defaults();
    if let Some(old) = stored.as_object() {
        for (k, v) in old {
            p[k] = v.clone();
        }
    }
    let current = stored["countdownSeconds"].as_u64().filter(|n| COUNTDOWN_SECONDS.contains(n));
    let migrated = if stored["countdown"].as_u64() == Some(0) { 0 } else { 3 };
    p["countdownSeconds"] = json!(current.unwrap_or(migrated));
    if let Some(o) = p.as_object_mut() {
        o.remove("countdown");
    }
    if !p["rememberInputs"].is_boolean() {
        p["rememberInputs"] = json!(false);
    }
    // Whether a person ever picked the frame rate: a default must never overwrite a choice. A file written by an
    // earlier build that stores a rate without saying whether it was picked is read as picked: it cannot be told
    // from an explicit 30, 24 or 60, and nothing is ever raised on a guess.
    if !stored["fpsChosen"].is_boolean() {
        p["fpsChosen"] = json!(stored.get("fps").is_some());
    }
    p
}
fn storage_root(value: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(value);
    if !p.is_absolute() {
        return Err("Choose an absolute storage path".into());
    }
    if cfg!(target_os = "macos") && value.starts_with("/Volumes/") {
        let volume = value.split('/').nth(2).ok_or("Invalid drive")?;
        if !Path::new("/Volumes").join(volume).is_dir() {
            return Err("Reconnect the selected external drive before recording".into());
        }
    }
    Ok(p)
}
fn free_space(root: &str) -> Option<u64> {
    let mut p = PathBuf::from(root);
    for _ in 0..30 {
        if let Ok(n) = fs2::available_space(&p) {
            return Some(n);
        }
        if !p.pop() {
            break;
        }
        if cfg!(target_os = "macos") && p == Path::new("/Volumes") {
            break;
        }
    }
    None
}
// Copy metadata only; media and the previous version's index remain untouched.
fn migrate_legacy(home: &Path) -> Result<(), String> {
    if std::env::var_os("OPENRECORDER_HOME").is_some()
        || !["Native", "native"].iter().any(|n| home.file_name().is_some_and(|x| x == *n)) {
        return Ok(());
    }
    let Some(parent) = home.parent() else { return Ok(()); };
    if home.join("library.json").exists() || !parent.join("library.json").is_file() { return Ok(()); }
    let index = read(&parent.join("library.json"), Value::Null)?;
    if !index["folders"].is_array() || !index["recordings"].is_array() {
        return Err("The previous library index needs recovery. Your videos are unchanged.".into());
    }
    if !home.join("settings.json").exists() && parent.join("settings.json").exists() {
        let config = read(&parent.join("settings.json"), Value::Null)?;
        if config["root"].as_str().is_none_or(|r| !Path::new(r).is_absolute()) || !config["settings"].is_object() {
            return Err("The previous storage settings need recovery. Your videos are unchanged.".into());
        }
        save(&home.join("settings.json"), &config)?;
    }
    save(&home.join("library.json"), &index)?;
    save(&home.join("migration.json"), &json!({"from":parent,"at":now(),"mediaMoved":false}))
}
struct Store {
    home: PathBuf,
    config: Value,
    index: Value,
    active: Value,
    pid: Option<u32>,
    // The native process that began the active take, so its death can be confirmed later.
    take_owner: Option<u32>,
    // Recording attempts a client gave up on (an unanswered native-begin): a late arrival is refused.
    cancelled_attempts: Vec<String>,
    // The capture state the native app last published (phase, generation, inputs, notice, capabilities). Memory only:
    // it is the single thing Home renders for capture, and it is cleared when the native process registers anew or is
    // confirmed gone, so a stale timer can never outlive its process.
    capture: Value,
    capture_at: u64,
    /// When the owner's Home page last read the state (the page polls while it is open). 0 = never.
    home_seen_at: u64,
    last_poll: u64,
    launched_at: u64,
}
/// A client-chosen identity for one native-begin, so an unanswered request can be reconciled instead of
/// retried as a new take. Absent for legacy callers.
fn recording_attempt(value: &Value) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) if (8..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') => Ok(Some(s.clone())),
        _ => Err("Invalid recording attempt".into()),
    }
}
/// A process id the client claims as its own: a positive integer that fits a `pid_t`. Zero, negatives, fractions,
/// strings and anything beyond i32::MAX are invalid and never recorded (a signal to pid 0 or a negative value
/// would address a process group, not a process).
fn process_id(value: &Value) -> Result<Option<u32>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_u64()
            .filter(|p| (1..=i32::MAX as u64).contains(p))
            .map(|p| Some(p as u32))
            .ok_or_else(|| "Invalid owner process".to_string()),
        _ => Err("Invalid owner process".into()),
    }
}
const CAPTURE_PHASES: [&str; 7] = ["idle", "countdown", "starting", "recording", "stopping", "saving", "unavailable"];
/// The one status vocabulary for a camera or microphone: off, armed (on, not live), starting, live, and three reasons it
/// needs attention. `attention` is the older catch-all.
const INPUT_MARKS: [&str; 8] = ["off", "armed", "starting", "live", "attention", "access-off", "not-connected", "no-video"];
const FPS60_STATUSES: [&str; 4] = ["unknown", "qualified", "failed-last", "unavailable"];
fn short(v: &Value, max: usize) -> Option<String> {
    v.as_str().filter(|s| s.chars().count() <= max && !s.chars().any(|c| c.is_control() && c != '\n')).map(str::to_owned)
}
/// The native app's published capture view, reduced to known fields of bounded size. Nothing else is stored or
/// echoed back to Home (no paths, no credentials).
fn sanitize_capture(v: &Value) -> Result<Value, String> {
    let phase = v["phase"].as_str().filter(|p| CAPTURE_PHASES.contains(p)).ok_or("Invalid capture phase")?;
    let generation = v["generation"].as_u64().ok_or("Invalid capture generation")?;
    let mut out = json!({"phase":phase,"generation":generation});
    // Identifies this app launch. Controls made for another launch are refused.
    if let Some(launch) = v["launch"].as_str() {
        if !(8..=64).contains(&launch.len()) || !launch.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("Invalid launch".into());
        }
        out["launch"] = json!(launch);
    }
    for (key, max) in [("elapsedMs", 100 * 3600 * 1000u64), ("fpsSet", 240), ("sourceFrames", u64::MAX / 2), ("outputFrames", u64::MAX / 2)] {
        if let Some(n) = v[key].as_u64() {
            if n > max {
                return Err(format!("Invalid {key}"));
            }
            out[key] = json!(n);
        }
    }
    for key in ["fpsDelivered", "sourceFps"] {
        if let Some(n) = v[key].as_f64() {
            if !n.is_finite() || !(0.0..=1000.0).contains(&n) {
                return Err(format!("Invalid {key}"));
            }
            out[key] = json!(n);
        }
    }
    for key in ["trayAvailable", "screenAllowed"] {
        if let Some(b) = v[key].as_bool() {
            out[key] = json!(b);
        }
    }
    if let Some(note) = short(&v["trayNote"], 160) {
        out["trayNote"] = json!(note);
    }
    if let Some(inputs) = v["inputs"].as_object() {
        let mut clean = serde_json::Map::new();
        for kind in ["camera", "microphone"] {
            if let Some(input) = inputs.get(kind) {
                let mark = input["mark"].as_str().filter(|m| INPUT_MARKS.contains(m)).ok_or("Invalid input mark")?;
                let mut item = json!({"mark":mark});
                if let Some(name) = short(&input["name"], 120) {
                    item["name"] = json!(name);
                }
                if let Some(note) = short(&input["note"], 160) {
                    item["note"] = json!(note);
                }
                clean.insert(kind.into(), item);
            }
        }
        out["inputs"] = Value::Object(clean);
    }
    // The real devices the app listed, by name, for the pickers. A device without a stable id is never listed, and no
    // name is made up here: an unresolved one simply is not in the list.
    if let Some(devices) = v["devices"].as_object() {
        let mut clean = serde_json::Map::new();
        for kind in ["camera", "microphone"] {
            if let Some(list) = devices.get(kind).and_then(Value::as_array) {
                let mut items = Vec::new();
                for d in list.iter().take(16) {
                    let uid = d["uid"].as_str().filter(|u| !u.is_empty() && u.len() <= 512 && !u.chars().any(char::is_control)).ok_or("Invalid device list")?;
                    let name = short(d.get("name").unwrap_or(&Value::Null), 80).filter(|n| !n.is_empty()).ok_or("Invalid device list")?;
                    items.push(json!({"uid":uid,"name":name,"selected":d["selected"].as_bool().unwrap_or(false)}));
                }
                clean.insert(kind.into(), Value::Array(items));
            }
        }
        out["devices"] = Value::Object(clean);
    }
    // The camera's look as the app applies it now (it can differ from the saved preference during a take).
    if let Some(a) = v.get("appearance") {
        let a = a.as_object().ok_or("Invalid appearance")?;
        let shape = a.get("shape").and_then(Value::as_str).filter(|s| CAMERA_SHAPES.contains(s)).ok_or("Invalid appearance")?;
        let size = a.get("size").and_then(Value::as_u64).filter(|n| CAMERA_SIZES.contains(n)).ok_or("Invalid appearance")?;
        let mirror = a.get("mirror").and_then(Value::as_bool).ok_or("Invalid appearance")?;
        let show = a.get("showCamera").and_then(Value::as_bool).ok_or("Invalid appearance")?;
        out["appearance"] = json!({"shape":shape,"size":size,"mirror":mirror,"showCamera":show});
    }
    if let Some(state) = v["screenState"].as_str() {
        out["screenState"] = json!(["allowed", "not-asked", "denied"].contains(&state).then_some(state).ok_or("Invalid screen state")?);
    }
    if let Some(list) = v["sources"].as_array() {
        let mut clean = Vec::new();
        for s in list.iter().take(16) {
            let kind = s["kind"].as_str().filter(|k| ["display", "window"].contains(k)).ok_or("Invalid source list")?;
            let id = s["id"].as_u64().filter(|n| *n <= u32::MAX as u64).ok_or("Invalid source list")?;
            clean.push(json!({"id":id,"kind":kind,"name":short(s.get("name").unwrap_or(&Value::Null), 80).unwrap_or_default(),"selected":s["selected"].as_bool().unwrap_or(false)}));
        }
        out["sources"] = Value::Array(clean);
    }
    if let Some(source) = v["source"].as_object() {
        let kind = source.get("kind").and_then(Value::as_str).filter(|k| ["display", "window", "system"].contains(k)).ok_or("Invalid source")?;
        out["source"] = json!({"kind":kind,"name":short(source.get("name").unwrap_or(&Value::Null), 120).unwrap_or_default()});
    }
    if let Some(notice) = v["notice"].as_object() {
        let text = short(notice.get("text").unwrap_or(&Value::Null), 240).ok_or("Invalid notice")?;
        let kind = short(notice.get("kind").unwrap_or(&Value::Null), 32).ok_or("Invalid notice")?;
        out["notice"] = json!({"kind":kind,"text":text,"details":short(notice.get("details").unwrap_or(&Value::Null), 1200),"generation":notice.get("generation").and_then(Value::as_u64).unwrap_or(0),"kept":notice.get("kept").and_then(Value::as_bool).unwrap_or(true)});
    }
    if let Some(f) = v["capabilities"]["fps60"].as_object() {
        let status = f.get("status").and_then(Value::as_str).filter(|s| FPS60_STATUSES.contains(s)).ok_or("Invalid capability")?;
        out["capabilities"] = json!({"fps60":{"status":status,"basis":short(f.get("basis").unwrap_or(&Value::Null), 200).unwrap_or_default(),
            "measuredFps":f.get("measuredFps").and_then(Value::as_f64).filter(|n| n.is_finite())}});
        // The encoder-only check, kept beside (never inside) the answer for the workload.
        if let Some(e) = f.get("encoder").and_then(Value::as_object) {
            let es = e.get("status").and_then(Value::as_str).filter(|s| FPS60_STATUSES.contains(s)).ok_or("Invalid capability")?;
            out["capabilities"]["fps60"]["encoder"] = json!({"status":es,"measuredFps":e.get("measuredFps").and_then(Value::as_f64).filter(|n| n.is_finite())});
        }
    }
    Ok(out)
}
impl Store {
    /// What Home renders for capture. Never invented: before the native app publishes, the phase is `unavailable`.
    fn capture_view(&self) -> Value {
        if self.capture.is_null() {
            return json!({"phase":"unavailable","generation":0,"known":false});
        }
        let mut v = self.capture.clone();
        let age = now().saturating_sub(self.capture_at);
        v["known"] = json!(true);
        v["ageMs"] = json!(age);
        // The native app republishes while a take runs; a silent one is shown as stale, never as a running timer.
        v["stale"] = json!(age > 6000 && ["countdown", "starting", "recording", "stopping", "saving"].contains(&v["phase"].as_str().unwrap_or("")));
        v
    }
    /// Records which native process is the registered controller. It repairs a registration that never arrived
    /// (the first `native-window-ready` was lost) but never takes the registration from another live process, and
    /// never from the process that owns the active take. Only a confirmed-gone registration is replaced.
    fn register_native(&mut self, pid: u32, alive: impl Fn(u32) -> Liveness) {
        match self.pid {
            None => self.pid = Some(pid),
            Some(current) if current == pid => {}
            Some(current) => {
                let owns_take = !self.active.is_null() && self.take_owner == Some(current);
                if !owns_take && alive(current) == Liveness::Gone {
                    self.pid = Some(pid);
                }
            }
        }
    }
    fn new(home: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&home).map_err(|e| e.to_string())?;
        let user = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        let root = PathBuf::from(user).join(if cfg!(target_os = "macos") {
            "Movies/OpenRecorder"
        } else {
            "Videos/OpenRecorder"
        });
        migrate_legacy(&home)?;
        let mut config = read(
            &home.join("settings.json"),
            json!({"root":root,"settings":defaults()}),
        )?;
        config["settings"] = normalize_preferences(&config["settings"]);
        let mut index = read(
            &home.join("library.json"),
            json!({"version":1,"folders":[],"recordings":[]}),
        )?;
        if !index["folders"].is_array() || !index["recordings"].is_array() {
            return Err("Library index needs recovery; videos remain on disk.".into());
        }
        let mut recovered = false;
        for r in index["recordings"].as_array_mut().unwrap() {
            if ["preparing", "recording", "saving"].contains(&r["status"].as_str().unwrap_or("")) {
                r["status"] = json!("interrupted");
                r["error"] =
                    json!("Recording was interrupted. Available files have been preserved.");
                let mut files = vec![];
                if let Some(dir) = r["directory"].as_str() {
                    for name in [
                        "recording.mp4",
                        "screen.mp4",
                        "camera.mp4",
                        "recording.webm",
                        "screen.webm",
                        "camera.webm",
                        "camera.mov",
                    ] {
                        if let Ok(m) = fs::metadata(Path::new(dir).join(name)) {
                            if m.len() > 0 {
                                files.push(json!({"name":name,"bytes":m.len()}));
                            }
                        }
                    }
                }
                r["bytes"] = json!(files
                    .iter()
                    .map(|f| f["bytes"].as_u64().unwrap_or(0))
                    .sum::<u64>());
                r["files"] = json!(files);
                recovered = true;
            }
        }
        if recovered {
            save(&home.join("library.json"), &index)?;
        }
        Ok(Self {
            home,
            config,
            index,
            active: Value::Null,
            pid: None,
            take_owner: None,
            cancelled_attempts: Vec::new(),
            capture: Value::Null,
            capture_at: 0,
            home_seen_at: 0,
            last_poll: 0,
            launched_at: 0,
        })
    }
    // Only the index: operations that change settings save settings.json themselves. Writing both here could leave
    // one file new and the other old when the second write fails after the in-memory state was rolled back.
    fn persist(&self) -> Result<(), String> {
        save(&self.home.join("library.json"), &self.index)
    }
    fn snapshot(&self) -> Value {
        json!({"version":VERSION,"platform":if cfg!(target_os="macos"){"darwin"}else if cfg!(target_os="windows"){"win32"}else{"linux"},"root":self.config["root"],"settings":self.config["settings"],"folders":self.index["folders"],"recordings":self.index["recordings"],"totalBytes":self.index["recordings"].as_array().unwrap().iter().map(|r|r["bytes"].as_u64().unwrap_or(0)).sum::<u64>(),"freeBytes":free_space(self.config["root"].as_str().unwrap_or("")),"active":self.active,"shares":[],"capture":self.capture_view(),"homeAgeMs":if self.home_seen_at == 0 { Value::Null } else { json!(now().saturating_sub(self.home_seen_at)) }})
    }
    fn position(&self, id: &Value) -> Result<usize, String> {
        self.index["recordings"]
            .as_array()
            .unwrap()
            .iter()
            .position(|r| r["id"] == *id)
            .ok_or("Recording not found".into())
    }
    fn file(&self, id: &str, name: &str) -> Result<PathBuf, String> {
        if ![
            "recording.webm",
            "screen.webm",
            "camera.webm",
            "recording.mp4",
            "screen.mp4",
            "camera.mp4",
            "camera.mov",
            "thumbnail.jpg",
            "recording.json",
        ]
        .contains(&name)
        {
            return Err("File not found".into());
        }
        let i = self.position(&json!(id))?;
        let base = PathBuf::from(
            self.index["recordings"][i]["directory"]
                .as_str()
                .ok_or("Invalid directory")?,
        )
        .canonicalize()
        .map_err(|e| e.to_string())?;
        let p = base
            .join(name)
            .canonicalize()
            .map_err(|_| "File unavailable. Check its drive.".to_string())?;
        if p.parent() != Some(base.as_path()) {
            return Err("File outside recording folder".into());
        }
        Ok(p)
    }
    fn main_name(&self, id: &Value) -> Result<&str, String> {
        let i = self.position(id)?;
        let files = self.index["recordings"][i]["files"]
            .as_array()
            .ok_or("Missing media index")?;
        files
            .iter()
            .filter_map(|f| f["name"].as_str())
            .find(|n| ["recording.mp4", "recording.webm"].contains(n))
            .ok_or("Combined video is unavailable".into())
    }
    /// A take stays active until native-finish/abort is stored, and a failed store deliberately keeps it
    /// active. If the native process that began it is *confirmed* gone (`alive` returns Gone; an
    /// unknown answer never counts), nobody can finish it: store it as interrupted with every file kept so
    /// recording and Quit are not blocked forever. A failed store leaves it active to be tried again.
    fn reconcile_dead_native(&mut self, alive: impl Fn(u32) -> Liveness) -> Option<Result<Value, String>> {
        if self.active.is_null() {
            return None;
        }
        let owner = self.take_owner?;
        if alive(owner) != Liveness::Gone {
            return None;
        }
        let id = self.active["id"].clone();
        let result = self.action(
            "native-abort",
            json!({"id":id,"error":"OpenRecorder closed unexpectedly during this recording. Available files have been preserved."}),
        );
        if result.is_ok() {
            self.take_owner = None;
            self.pid = None;
            self.capture = Value::Null;
        }
        Some(result)
    }
    fn action(&mut self, op: &str, d: Value) -> Result<Value, String> {
        let previous = (self.config.clone(), self.index.clone(), self.active.clone());
        let owner = self.take_owner;
        let result = self.apply(op, d);
        if result.is_err() {
            (self.config, self.index, self.active) = previous;
            self.take_owner = owner;
        }
        result
    }
    fn apply(&mut self, op: &str, d: Value) -> Result<Value, String> {
        match op {
            "state" => Ok(self.snapshot()),
            "native-window-closed" => {
                self.pid = None;
                self.launched_at = 0;
                Ok(json!({"closed":true}))
            }
            "native-window-ready" => {
                if let Some(pid) = process_id(&d["pid"])? {
                    // A process registering anew has no capture: nothing from an earlier process may linger.
                    if self.pid != Some(pid) {
                        self.capture = Value::Null;
                    }
                    self.register_native(pid, process_liveness);
                }
                self.last_poll = now();
                Ok(json!({"ready":true}))
            }
            // The native app publishes its authoritative capture state. A message of an older generation is dropped,
            // so a late tick can never bring back a timer a newer take (or a terminal state) already replaced.
            "native-progress" => {
                let view = sanitize_capture(&d)?;
                let generation = view["generation"].as_u64().unwrap_or(0);
                let known = self.capture["generation"].as_u64().unwrap_or(0);
                // A different launch starts its own numbering, so it is never ordered against the previous one.
                let relaunch = view.get("launch") != self.capture.get("launch");
                if self.capture.is_null() || relaunch || generation >= known {
                    self.capture = view;
                    self.capture_at = now();
                    Ok(json!({"accepted":true}))
                } else {
                    Ok(json!({"accepted":false,"reason":"older generation"}))
                }
            }
            "native-onboarding-complete" => {
                save(
                    &self.home.join("onboarding.json"),
                    &json!({"completed":true}),
                )?;
                for name in ["rust-connection.json", "rust-runtime.json"] {
                    let file = self.home.join(name);
                    let mut c = read(&file, Value::Null)?;
                    if c.is_object() {
                        c["setup_complete"] = json!(true);
                        save(&file, &c)?;
                    }
                }
                Ok(json!({"completed":true}))
            }
            "settings" => {
                let mut next = self.config.clone();
                if !self.active.is_null() {
                    return Err("Stop and save before changing storage or quality.".into());
                }
                if let Some(root) = d["root"].as_str() {
                    let p = PathBuf::from(root);
                    if !p.is_absolute() {
                        return Err("Choose an absolute folder path".into());
                    }
                    if cfg!(target_os = "macos") && root.starts_with("/Volumes/") {
                        let vol = root.split('/').nth(2).ok_or("Invalid volume")?;
                        if !Path::new("/Volumes").join(vol).exists() {
                            return Err("Reconnect your external drive".into());
                        }
                    }
                    fs::create_dir_all(&p).map_err(|e| e.to_string())?;
                    next["root"] = json!(p);
                }
                if let Some(p) = d["preferences"].as_object() {
                    for (k, v) in p {
                        let valid = match k.as_str() {
                            "quality" => {
                                v.as_u64().is_some_and(|n| [1280, 1920, 2560].contains(&n))
                            }
                            "fps" => v.as_u64().is_some_and(|n| [24, 30, 60].contains(&n)),
                            "cameraSize" => v.as_u64().is_some_and(|n| CAMERA_SIZES.contains(&n)),
                            "countdownSeconds" => v.as_u64().is_some_and(|n| COUNTDOWN_SECONDS.contains(&n)),
                            "countdown" => return Err("Use countdownSeconds (0 or 3)".into()),
                            "cameraDeviceId" | "microphoneDeviceId" => v.is_null() || v.as_str().is_some_and(|s|s.len()<=512 && !s.chars().any(char::is_control)),
                            "theme" => v
                                .as_str()
                                .is_some_and(|n| ["system", "light", "dark"].contains(&n)),
                            "cameraShape" => v.as_str().is_some_and(|n| CAMERA_SHAPES.contains(&n)),
                            "camera" | "microphone" | "keepOriginals" | "mirror" | "showCamera" | "rememberInputs" | "fpsChosen" => v.is_boolean(),
                            _ => continue,
                        };
                        if !valid {
                            return Err(format!("Invalid {k}"));
                        }
                        next["settings"][k] = v.clone();
                    }
                    // Any write that names a frame rate is a choice, from Home, an agent or the native app alike,
                    // unless it says otherwise itself. An explicit 30, 24 or 60 can never be raised later.
                    if p.contains_key("fps") && !p.contains_key("fpsChosen") {
                        next["settings"]["fpsChosen"] = json!(true);
                    }
                }
                save(&self.home.join("settings.json"), &next)?;
                self.config = next;
                Ok(self.snapshot())
            }
            "folder" => {
                let name = d["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty() && s.len() <= 60)
                    .ok_or("Enter a folder name")?;
                if self.index["folders"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f["name"].as_str().unwrap_or("").eq_ignore_ascii_case(name))
                {
                    return Err("Folder already exists".into());
                }
                let f = json!({"id":uuid::Uuid::new_v4().to_string(),"name":name});
                self.index["folders"]
                    .as_array_mut()
                    .unwrap()
                    .push(f.clone());
                self.persist()?;
                Ok(f)
            }
            "native-begin" => {
                let attempt = recording_attempt(&d["attempt"])?;
                if let Some(a) = &attempt {
                    if self.cancelled_attempts.contains(a) {
                        return Err("This recording attempt was cancelled".into());
                    }
                    // The same attempt again is a replay of a request whose answer was lost, never a second take.
                    if let Some(r) = self.index["recordings"].as_array().unwrap().iter().find(|r| r["attempt"] == json!(a)) {
                        return if !self.active.is_null() && self.active["attempt"] == json!(a) {
                            Ok(r.clone())
                        } else {
                            Err("This recording attempt already ended".into())
                        };
                    }
                }
                if !self.active.is_null() {
                    return Err("A recording is already active".into());
                }
                // The process that can finish this take is fixed here, atomically with the take, from the
                // client's own explicit identity (validated), else from a confirmed registration. A take with
                // no known owner is refused: its death could never be reconciled.
                let owner = process_id(&d["ownerPid"])?.or(self.pid).ok_or("OpenRecorder hasn't registered its process yet. Try again in a moment.")?;
                let id = uuid::Uuid::new_v4().to_string();
                let root = d["root"]
                    .as_str()
                    .or(self.config["root"].as_str())
                    .ok_or("Missing storage")?;
                let base = storage_root(root)?;
                if !base.is_absolute() {
                    return Err("Invalid recording path".into());
                }
                // A folder is metadata only: the take always lives at <root>/<uuid>, whatever folder it is filed in.
                let folder = d["folder"].clone();
                if !folder.is_null()
                    && !(folder.is_string() && self.index["folders"].as_array().unwrap().iter().any(|v| v["id"] == folder))
                {
                    return Err("Folder not found".into());
                }
                let dir = base.join(&id);
                fs::create_dir_all(&base).map_err(|e| e.to_string())?;
                fs::create_dir(&dir).map_err(|e| e.to_string())?;
                let r = json!({"id":id,"title":d["title"].as_str().unwrap_or("New recording"),"folder":folder,"root":base,"directory":dir,"createdAt":now(),"status":"recording","favorite":false,"archived":false,"duration":0,"bytes":0,"files":[],"settings":if d["preferences"].is_object(){d["preferences"].clone()}else{self.config["settings"].clone()}});
                let mut r = r;
                if let Some(a) = &attempt {
                    r["attempt"] = json!(a);
                }
                self.index["recordings"]
                    .as_array_mut()
                    .unwrap()
                    .insert(0, r.clone());
                self.active =
                    json!({"id":id,"engine":"native","status":"recording","startedAt":now()});
                if let Some(a) = &attempt {
                    self.active["attempt"] = json!(a);
                }
                self.take_owner = Some(owner);
                self.register_native(owner, process_liveness);
                if let Err(e) = self.persist() {
                    // The caller restores the in-memory index; remove the empty directory this call created.
                    let _ = fs::remove_dir(&dir);
                    return Err(e);
                }
                Ok(r)
            }
            // Settles an unanswered native-begin atomically under the store lock: the attempt is reported as
            // active (the take exists), completed (it began and ended), or cancelled, which also refuses any
            // later arrival of the same request. It never touches a take that belongs to another attempt.
            "native-begin-resolve" => {
                let attempt = recording_attempt(&d["attempt"])?.ok_or("Missing recording attempt")?;
                let found = self.index["recordings"].as_array().unwrap().iter().find(|r| r["attempt"] == json!(attempt)).cloned();
                match found {
                    Some(r) if !self.active.is_null() && self.active["attempt"] == json!(attempt) => Ok(json!({"state":"active","record":r})),
                    Some(r) => Ok(json!({"state":"completed","record":{"id":r["id"],"status":r["status"]}})),
                    None => {
                        if !self.cancelled_attempts.contains(&attempt) {
                            if self.cancelled_attempts.len() >= 64 {
                                self.cancelled_attempts.remove(0);
                            }
                            self.cancelled_attempts.push(attempt);
                        }
                        Ok(json!({"state":"cancelled"}))
                    }
                }
            }
            "native-finish" | "native-abort" => {
                if self.active["id"] != d["id"] || self.active.is_null() {
                    return Err("No matching active recording".into());
                }
                let i = self.position(&d["id"])?;
                let r = &mut self.index["recordings"][i];
                let dir = PathBuf::from(r["directory"].as_str().ok_or("Missing directory")?);
                let mut files = vec![];
                for name in ["recording.mp4", "screen.mp4", "camera.mp4"] {
                    if let Ok(m) = fs::metadata(dir.join(name)) {
                        if m.len() > 0 {
                            files.push(json!({"name":name,"bytes":m.len()}))
                        }
                    }
                }
                r["bytes"] = json!(files
                    .iter()
                    .map(|f| f["bytes"].as_u64().unwrap())
                    .sum::<u64>());
                r["files"] = json!(files);
                r["duration"] = d["duration"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .map_or(json!(0), |n| json!(n));
                r["status"] = json!(if op == "native-abort"
                    || d["partial"] == true
                    || !r["files"].as_array().unwrap().iter().any(|f| f["name"] == "recording.mp4")
                    || (r["settings"]["camera"] == true && r["settings"]["keepOriginals"] == true
                        && (!r["files"].as_array().unwrap().iter().any(|f| f["name"] == "screen.mp4")
                            || (r["settings"]["camera"] == true && !r["files"].as_array().unwrap().iter().any(|f| f["name"] == "camera.mp4"))))
                {
                    "interrupted"
                } else {
                    "ready"
                });
                if r["status"] == "interrupted" {
                    r["error"] = json!(d["error"].as_str().unwrap_or("Recording is incomplete. Available files have been preserved."));
                }
                // Whether the files were closed properly (the encoder finished and the container was finalised), as the
                // app reported it. Only a boolean it actually sent is stored; without it the take is simply "not known".
                if let Some(finalized) = d["finalized"].as_bool() {
                    r["finalized"] = json!(finalized);
                }
                if let Some(capture) = d["capture"].as_object() {
                    let mut clean = serde_json::Map::new();
                    for (key, v) in capture {
                        let ok = match key.as_str() {
                            "fps" | "width" | "height" | "frames" | "outputFps" | "sourceFps" => v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.),
                            "finalization" | "status" => v.as_str().is_some_and(|s| s.len() <= 24 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')),
                            "encoders" => v.as_object().is_some_and(|m| m.len() <= 3 && m.values().all(|x| x.as_str().is_some_and(|s| s.len() <= 32))),
                            _ => false,
                        };
                        if ok {
                            clean.insert(key.clone(), v.clone());
                        }
                    }
                    r["capture"] = json!(clean);
                }
                let result = r.clone();
                save(&dir.join("recording.json"), &result)?;
                self.active = Value::Null;
                self.take_owner = None;
                self.persist()?;
                Ok(result)
            }
            "stop" => {
                let i = self.position(&self.active["id"])?;
                fs::write(
                    PathBuf::from(self.index["recordings"][i]["directory"].as_str().unwrap())
                        .join(".stop"),
                    "stop",
                )
                .map_err(|e| e.to_string())?;
                // Home must stop counting at once: the host marks the take stopping before the native app confirms.
                if self.capture["phase"] == "recording" || self.capture["phase"] == "starting" || self.capture["phase"] == "countdown" {
                    self.capture["phase"] = json!("stopping");
                    self.capture_at = now();
                }
                Ok(json!({"stopping":true}))
            }
            "update" => {
                // Everything is validated before anything changes. Filing into a folder is metadata only: the
                // recording's directory, root and files are never moved or touched.
                let patch = d["patch"].as_object().ok_or("Invalid update")?;
                if let Some(folder) = patch.get("folder") {
                    if !folder.is_null()
                        && !(folder.is_string() && self.index["folders"].as_array().unwrap().iter().any(|f| f["id"] == *folder))
                    {
                        return Err("Folder not found".into());
                    }
                }
                if let Some(title) = patch.get("title").filter(|v| !v.is_null()) {
                    if title.as_str().is_none_or(|s| s.trim().is_empty() || s.len() > 120) {
                        return Err("Invalid title".into());
                    }
                }
                for k in ["favorite", "archived"] {
                    if patch.get(k).is_some_and(|v| !v.is_null() && !v.is_boolean()) {
                        return Err("Invalid update".into());
                    }
                }
                if !self.active.is_null() && self.active["id"] == d["id"] {
                    return Err("Stop and save first".into());
                }
                let i = self.position(&d["id"])?;
                let r = &mut self.index["recordings"][i];
                for k in ["title", "favorite", "archived"] {
                    if let Some(v) = patch.get(k).filter(|v| !v.is_null()) {
                        r[k] = v.clone();
                    }
                }
                if let Some(folder) = patch.get("folder") {
                    r["folder"] = folder.clone();
                }
                let result = r.clone();
                self.persist()?;
                Ok(result)
            }
            "open-file" | "reveal" => {
                let file = self.file(
                    d["id"].as_str().ok_or("Missing recording")?,
                    d["name"].as_str().unwrap_or(self.main_name(&d["id"])?),
                )?;
                open(&file.display().to_string(), op == "reveal")?;
                Ok(json!({"opened":true}))
            }
            _ => Err(format!("Unsupported native library operation: {op}")),
        }
    }
}
fn open(value: &str, reveal: bool) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("/usr/bin/open");
        if reveal {
            c.arg("-R");
        }
        c
    } else if cfg!(target_os = "windows") {
        Command::new("explorer.exe")
    } else {
        Command::new("xdg-open")
    };
    let child = cmd
        .arg(value)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    reap_in_background(child);
    Ok(())
}
// Wait for a short-lived child this host spawned so it never lingers as a zombie. It is never killed.
fn reap_in_background(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}
/// Arguments for the passive native toolbar launch. There is deliberately no flag that activates inputs.
fn desktop_args(config: &Path) -> Vec<std::ffi::OsString> {
    vec!["--desktop".into(), "--config".into(), config.into()]
}
const NATIVE_POLL_FRESH_MS: u64 = 35000;
const LAUNCH_DEBOUNCE_MS: u64 = 8000;
#[derive(Debug, PartialEq, Eq)]
enum StudioPlan {
    /// A registered toolbar is polling: pass it a passive show.
    Show,
    /// The registered toolbar is stale but not confirmed gone: never launch a second one.
    Busy,
    /// A launch is already in flight.
    Opening,
    Launch,
}
/// The toolbar that counts is the one that began the active take, else the one that announced itself. A stale
/// poll alone never authorises a launch while that process is `Alive` or `Unknown`, and an active take whose
/// owner is not known blocks a launch too. Only no registration, or a confirmed `Gone`, allows one.
fn studio_plan(
    pid: Option<u32>,
    take_owner: Option<u32>,
    take_active: bool,
    poll_age_ms: u64,
    since_launch_ms: u64,
    liveness: impl Fn(u32) -> Liveness,
) -> StudioPlan {
    let registered = if take_active { take_owner.or(pid) } else { pid };
    match registered {
        Some(_) if poll_age_ms < NATIVE_POLL_FRESH_MS => return StudioPlan::Show,
        Some(owner) if liveness(owner) != Liveness::Gone => return StudioPlan::Busy,
        None if take_active => return StudioPlan::Busy,
        _ => {}
    }
    if since_launch_ms < LAUNCH_DEBOUNCE_MS {
        StudioPlan::Opening
    } else {
        StudioPlan::Launch
    }
}
fn launch_desktop(config: &Path, recording: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let child = if cfg!(target_os = "macos") {
        let app = super::installation::stable_mac_app_checked(&exe, recording)?;
        Command::new("/usr/bin/open").arg("-n").arg(app).arg("--args").args(desktop_args(config)).spawn()
    } else {
        Command::new(exe).args(desktop_args(config)).spawn()
    };
    child.map(reap_in_background).map_err(|e| e.to_string())
}
/// A person's capture command from Home, queued for the native app. Short-lived by design: it is dropped unread once
/// it is older than `HUMAN_TTL_MS`, so a command queued while the app was away can never start anything later.
#[derive(Clone, Debug, PartialEq)]
struct Human {
    id: u64,
    name: String,
    payload: Value,
    issued_at: u64,
}
const HUMAN_TTL_MS: u64 = 5000;
const HUMAN_QUEUE_MAX: usize = 8;
/// How long a Record press made while the app was not running may wait for the app to start and report. One press, one
/// start: it is used up when it starts a take, when it expires, or when anything else (a Stop, a Quit, a take already
/// running) says it no longer applies.
const PENDING_START_MS: u64 = 20_000;
/// A control is refused if the app's last report is older than this (it publishes at least every 4 s when idle).
const HUMAN_REPORT_MAX_AGE_MS: u64 = 12_000;
/// Browser and host share one clock; this only absorbs rounding.
const HUMAN_SKEW_MS: u64 = 1_000;
/// Owner-only capture controls. They are deliberately NOT in `PUBLIC_OPS`: the MCP bridge refuses everything outside
/// that list, so no agent tool can name, queue or see them.
const HUMAN_OPS: &[&str] = &["capture-command"];
const HUMAN_COMMANDS: &[&str] = &["record", "stop", "cancel", "input", "source", "device", "shape", "size", "mirror", "fps", "capacity-check", "dismiss-notice", "choose-window", "show-camera", "open-privacy"];
fn validate_human(d: &Value) -> Result<(String, Value), String> {
    let name = d["command"].as_str().filter(|c| HUMAN_COMMANDS.contains(c)).ok_or("Unknown capture command")?;
    let mut payload = serde_json::Map::new();
    // Every control is bound to the app launch it was made for, the take number it saw, an expiry and a one-time
    // nonce. The host and the app both check them; a request that arrives late, twice, or after a restart is dropped.
    let token = |v: &Value| v.as_str().filter(|s| (8..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).map(str::to_owned);
    payload.insert("launch".into(), json!(token(&d["launch"]).ok_or("This control is missing its session. Reload Home and try again.")?));
    payload.insert("nonce".into(), json!(token(&d["nonce"]).ok_or("This control is missing its request number")?));
    payload.insert("expires".into(), json!(d["expires"].as_u64().ok_or("This control has no expiry")?));
    if let Some(g) = d.get("generation").filter(|g| !g.is_null()) {
        payload.insert("generation".into(), json!(g.as_u64().ok_or("Invalid generation")?));
    }
    if ["record", "stop", "cancel"].contains(&name) && !payload.contains_key("generation") {
        return Err("This control doesn't say which recording it is for".into());
    }
    let kind = |v: &Value| v.as_str().filter(|k| ["camera", "mic"].contains(k)).map(str::to_owned).ok_or("Invalid input");
    match name {
        "input" => {
            payload.insert("kind".into(), json!(kind(&d["kind"])?));
            payload.insert("on".into(), json!(d["on"].as_bool().ok_or("Invalid input state")?));
        }
        "device" => {
            payload.insert("kind".into(), json!(kind(&d["kind"])?));
            let uid = &d["uid"];
            if !(uid.is_null() || uid.as_str().is_some_and(|s| s.len() <= 512 && !s.chars().any(char::is_control))) {
                return Err("Invalid device".into());
            }
            payload.insert("uid".into(), uid.clone());
        }
        "source" => {
            let kind = d["kind"].as_str().filter(|k| ["display", "window", "default"].contains(k)).ok_or("Invalid source")?;
            payload.insert("kind".into(), json!(kind));
            if kind != "default" {
                payload.insert("id".into(), json!(d["id"].as_u64().filter(|n| *n <= u32::MAX as u64).ok_or("Invalid source")?));
            }
        }
        "shape" => {
            payload.insert("value".into(), json!(d["value"].as_str().filter(|n| CAMERA_SHAPES.contains(n)).ok_or("Invalid shape")?));
        }
        "size" => {
            payload.insert("value".into(), json!(d["value"].as_u64().filter(|n| CAMERA_SIZES.contains(n)).ok_or("Invalid size")?));
        }
        "mirror" => {
            payload.insert("value".into(), json!(d["value"].as_bool().ok_or("Invalid mirror")?));
        }
        "fps" => {
            payload.insert("value".into(), json!(d["value"].as_u64().filter(|n| [24, 30, 60].contains(n)).ok_or("Invalid frame rate")?));
        }
        "show-camera" => {
            payload.insert("value".into(), json!(d["value"].as_bool().ok_or("Invalid camera choice")?));
        }
        "open-privacy" => {
            payload.insert("kind".into(), json!(d["kind"].as_str().filter(|k| ["screen", "camera", "mic"].contains(k)).ok_or("Invalid privacy page")?));
        }
        _ => {}
    }
    Ok((name.to_owned(), Value::Object(payload)))
}
struct Host {
    shares: Arc<Mutex<super::shares::Shares>>,
    store: Arc<Mutex<Store>>,
    human: Mutex<std::collections::VecDeque<Human>>,
    human_seq: std::sync::atomic::AtomicU64,
    human_nonces: Mutex<std::collections::VecDeque<(String, u64)>>,
    /// A person's Record press made while the app was not running: when it may still start a take, if at all.
    pending_start: Mutex<Option<u64>>,
    command: Mutex<Option<String>>,
    wake: Condvar,
    origin: String,
    key: String,
    // Read-only capability for /file/ media only. It is the only credential the embedded MCP App resource carries;
    // it is rejected by every /api route, so it can never run an owner or native operation.
    media_key: String,
    // Set under the store lock when Quit is accepted, so a take cannot begin after the active check.
    quit_requested: std::sync::atomic::AtomicBool,
}
impl Host {
    fn quitting(&self) -> bool {
        self.quit_requested.load(std::sync::atomic::Ordering::SeqCst)
    }
    // Hand one command to the toolbar's next poll. A stronger pending command is never downgraded.
    fn post_command(&self, command: &str) {
        let rank = |c: &str| match c {
            "quit" => 3,
            "preferences" => 2,
            "show" => 1,
            _ => 0,
        };
        let mut pending = self.command.lock().unwrap();
        if pending.as_deref().map_or(0, rank) <= rank(command) {
            *pending = Some(command.into());
        }
        drop(pending);
        self.wake.notify_one();
    }
    /// Queues one person's capture command. Nothing is opened here: the native app decides, through its own
    /// state machine and the same preflight as its menu, whether and how to act.
    fn queue_human(&self, d: &Value) -> Result<Value, String> {
        let (name, payload) = validate_human(d)?;
        if self.quitting() {
            return Err(QUIT_MESSAGE.into());
        }
        {
            let s = self.store.lock().unwrap();
            let phase = s.capture["phase"].as_str().unwrap_or("");
            if s.pid.is_none() {
                return Err("OpenRecorder isn't running. Open the app, then use the control again.".into());
            }
            // The control must belong to the app that is running now, and to what it last reported: a request
            // minted for an earlier launch, or delivered long after it was made, is never turned into a fresh one.
            let t = now();
            if s.capture.is_null() || s.capture["launch"].as_str() != payload["launch"].as_str() {
                return Err("OpenRecorder was restarted or isn't reporting. Reload Home and try again.".into());
            }
            if t.saturating_sub(s.capture_at) > HUMAN_REPORT_MAX_AGE_MS {
                return Err("OpenRecorder isn't responding. Try again in a moment.".into());
            }
            if payload.get("generation").is_some() && payload["generation"] != s.capture["generation"] {
                return Err("That recording has already changed. Check Home and try again.".into());
            }
            let expires = payload["expires"].as_u64().unwrap_or(0);
            if expires <= t || expires > t + HUMAN_TTL_MS + HUMAN_SKEW_MS {
                return Err("This control expired. Press it again.".into());
            }
            // Cheap host-side refusals; the native app re-checks everything against its own state.
            if name == "record" && !["idle", ""].contains(&phase) {
                return Ok(json!({"queued":false,"ignored":"A recording is already in progress"}));
            }
            if (name == "stop" || name == "cancel") && ["idle", "unavailable", "saving", "stopping"].contains(&phase) && !phase.is_empty() {
                return Ok(json!({"queued":false,"ignored":"Nothing to stop"}));
            }
        }
        let id = self.human_seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        {
            // The request number is consumed once. The same number again (a replay, a retry after a lost answer) is refused.
            let mut seen = self.human_nonces.lock().unwrap();
            let t = now();
            while seen.front().is_some_and(|(_, at)| t.saturating_sub(*at) > 3 * HUMAN_TTL_MS) {
                seen.pop_front();
            }
            let nonce = payload["nonce"].as_str().unwrap_or_default().to_owned();
            if seen.iter().any(|(n, _)| *n == nonce) {
                return Err("This control was already used".into());
            }
            let mut queue = self.human.lock().unwrap();
            if queue.len() >= HUMAN_QUEUE_MAX {
                return Err("Too many pending controls. Try again in a moment.".into());
            }
            if seen.len() >= 256 {
                seen.pop_front();
            }
            seen.push_back((nonce, t));
            queue.push_back(Human { id, name, payload, issued_at: t });
        }
        // Wake the native long poll (the condvar pairs with the command mutex, so no wakeup is lost).
        drop(self.command.lock().unwrap());
        self.wake.notify_one();
        Ok(json!({"queued":true,"id":id,"ttlMs":HUMAN_TTL_MS}))
    }
    /// The person pressed Record on the library page. If the app is up and reporting this is exactly a Record command
    /// (and needs the page's launch, take, expiry and number). If it is not, the press starts the app and waits for
    /// its first idle report, then starts the countdown once: the person pressed once and records once.
    fn start_recording_with(&self, d: &Value, start_app: impl FnOnce() -> Result<Value, String>) -> Result<Value, String> {
        if self.quitting() {
            return Err(QUIT_MESSAGE.into());
        }
        let live = {
            let s = self.store.lock().unwrap();
            s.pid.is_some() && !s.capture.is_null() && now().saturating_sub(s.capture_at) <= HUMAN_REPORT_MAX_AGE_MS
        };
        if live {
            let mut body = d.clone();
            body["command"] = json!("record");
            return self.queue_human(&body);
        }
        let token = |v: &Value| v.as_str().filter(|s| (8..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).map(str::to_owned);
        let nonce = token(&d["nonce"]).ok_or("This control is missing its request number")?;
        let t = now();
        let expires = d["expires"].as_u64().ok_or("This control has no expiry")?;
        if expires <= t || expires > t + HUMAN_TTL_MS + HUMAN_SKEW_MS {
            return Err("This control expired. Press it again.".into());
        }
        {
            let mut seen = self.human_nonces.lock().unwrap();
            if seen.iter().any(|(n, _)| *n == nonce) {
                return Err("This control was already used".into());
            }
            if seen.len() >= 256 {
                seen.pop_front();
            }
            seen.push_back((nonce, t));
        }
        *self.pending_start.lock().unwrap() = Some(t + PENDING_START_MS);
        let opened = match start_app() {
            Ok(v) => v,
            Err(e) => {
                *self.pending_start.lock().unwrap() = None;
                return Err(e);
            }
        };
        Ok(json!({"queued":false,"starting":true,"ttlMs":PENDING_START_MS,"app":opened}))
    }
    /// Called with every report from the app. A waiting press is turned into one Record for the launch that reported,
    /// made here, now (so it names that launch and take and lives five seconds). Never over a take in progress.
    fn deliver_pending_start(&self) {
        let mut pending = self.pending_start.lock().unwrap();
        let Some(until) = *pending else { return };
        let t = now();
        if t > until {
            *pending = None;
            return;
        }
        let (phase, launch, generation) = {
            let s = self.store.lock().unwrap();
            (s.capture["phase"].as_str().unwrap_or("").to_owned(), s.capture["launch"].as_str().map(str::to_owned), s.capture["generation"].as_u64())
        };
        match (phase.as_str(), launch, generation) {
            ("idle", Some(launch), Some(generation)) => {
                *pending = None;
                drop(pending);
                let id = self.human_seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                {
                    let mut queue = self.human.lock().unwrap();
                    if queue.len() < HUMAN_QUEUE_MAX {
                        queue.push_back(Human { id, name: "record".into(), payload: json!({"launch":launch,"generation":generation,"nonce":uuid::Uuid::new_v4().to_string(),"expires":t + HUMAN_TTL_MS}), issued_at: t });
                    }
                }
                drop(self.command.lock().unwrap());
                self.wake.notify_one();
            }
            // Anything but a quiet, idle app (a countdown, a take, an app that isn't ready) means this press no longer applies.
            ("unavailable" | "", ..) => {}
            _ => *pending = None,
        }
    }
    /// A piece of a library file for a page that cannot reach the address itself (an agent host's panel). Bounded:
    /// at most `MEDIA_CHUNK` bytes, only the fixed set of file names the library serves, only inside a recording folder.
    fn media_read(&self, d: &Value) -> Result<Value, String> {
        let id = d["id"].as_str().ok_or("Invalid recording")?;
        let name = d["name"].as_str().ok_or("Invalid file")?;
        let start = d["start"].as_u64().unwrap_or(0);
        let want = d["length"].as_u64().unwrap_or(MEDIA_CHUNK).clamp(1, MEDIA_CHUNK);
        let path = self.store.lock().unwrap().file(id, name)?;
        let mut file = fs::File::open(path).map_err(|_| "File unavailable. Check its drive.".to_string())?;
        let total = file.metadata().map_err(|e| e.to_string())?.len();
        if start >= total {
            return Err("Past the end of the file".into());
        }
        let length = want.min(total - start) as usize;
        file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; length];
        file.read_exact(&mut buffer).map_err(|e| e.to_string())?;
        Ok(json!({"mime":mime_of(name),"total":total,"start":start,"length":length,"data":base64(&buffer)}))
    }
    /// The oldest still-fresh command, if any; stale ones are dropped unread.
    fn next_human(&self) -> Option<Human> {
        let mut queue = self.human.lock().unwrap();
        let t = now();
        while let Some(first) = queue.pop_front() {
            if t.saturating_sub(first.issued_at) <= HUMAN_TTL_MS && first.payload["expires"].as_u64().is_none_or(|e| e > t) {
                return Some(first);
            }
        }
        None
    }
    // Passive launch/show only. Camera and microphone intent belongs to the human native controls: no HTTP or
    // MCP operation and no startup flag asks the toolbar to open an input.
    fn open_studio(&self) -> Result<Value, String> {
        self.open_studio_with(process_liveness, launch_desktop)
    }
    // The decision and the launch are separate so tests run the clock/liveness cases with a counting launcher.
    fn open_studio_with(
        &self,
        liveness: impl Fn(u32) -> Liveness,
        launch: impl FnOnce(&Path, bool) -> Result<(), String>,
    ) -> Result<Value, String> {
        let mut s = self.store.lock().unwrap();
        let plan = studio_plan(
            s.pid,
            s.take_owner,
            !s.active.is_null(),
            now().saturating_sub(s.last_poll),
            now().saturating_sub(s.launched_at),
            liveness,
        );
        match plan {
            StudioPlan::Show => {
                drop(s);
                self.post_command("show");
                Ok(json!({"opened":true,"engine":"rust"}))
            }
            // A toolbar that owns a take (or whose owner can't be told apart from a live one) is never
            // replaced because its poll is old: the request waits as a passive Show and the reply says so.
            StudioPlan::Busy => {
                drop(s);
                self.post_command("show");
                Ok(json!({"opened":false,"busy":true,"queued":true,"engine":"rust","message":"OpenRecorder is recording or not responding. Home will open as soon as it responds."}))
            }
            StudioPlan::Opening => Ok(json!({"opening":true,"engine":"rust"})),
            StudioPlan::Launch => {
                let config = s.home.join("rust-connection.json");
                s.launched_at = now();
                let recording = !s.active.is_null();
                drop(s);
                if let Err(e) = launch(&config, recording) {
                    // A failed launch must not hold the eight-second launch window closed.
                    self.store.lock().unwrap().launched_at = 0;
                    return Err(e);
                }
                Ok(json!({"opened":true,"engine":"rust"}))
            }
        }
    }
    // The owner key never appears in markup: the browser library reads it from the URL fragment it was opened
    // with, and the embedded MCP App reaches actions through `tools/call` and media through `mediaKey`.
    fn markup(&self, embedded: bool) -> String {
        include_str!("../../ui/index.html").replace("/* STYLES */",&include_str!("../../ui/style.css").replace("url(/font.ttf)",&format!("url({}/font.ttf)",self.origin))).replace("/* CONFIG */",&format!("const CONNECTION={};",json!({"embedded":embedded,"origin":self.origin,"key":Value::Null,"mediaKey":if embedded{json!(self.media_key)}else{Value::Null}}))).replace("/* APP */",include_str!("../../ui/app.js"))
    }
    fn action(&self, op: &str, d: Value) -> Result<Value, String> {
        if self.quitting() && !["quit", "native-command", "native-window-closed"].contains(&op) {
            return Err(QUIT_MESSAGE.into());
        }
        // A native process registering anew has never seen the queue: nothing queued before it may run after it.
        if op == "native-window-ready" {
            self.human.lock().unwrap().clear();
        }
        // Ops that a stranded take would block (or report wrongly) first reconcile a confirmed-dead native.
        if ["state", "native-begin", "quit", "open-studio"].contains(&op) {
            let mut s = self.store.lock().unwrap();
            if let Some(Err(error)) = s.reconcile_dead_native(process_liveness) {
                eprintln!("openrecorder-core: could not store the interrupted take: {error}");
            }
        }
        match op {
            "native-begin" => {
                let mut s = self.store.lock().unwrap();
                if self.quitting() {
                    return Err(QUIT_MESSAGE.into());
                }
                s.action(op, d)
            }
            "native-finish" | "native-abort" => {
                let r = self.store.lock().unwrap().action(op, d)?;
                if let Some(dir) = r["directory"].as_str() {
                    let dir = PathBuf::from(dir);
                    let home = self.store.lock().unwrap().home.clone();
                    let store = self.store.clone();
                    let id = r["id"].clone();
                    std::thread::spawn(move || {
                        let Ok(c) = read(&home.join("rust-connection.json"), Value::Null) else {
                            return;
                        };
                        let Some(exe) = c["ffmpeg"].as_str() else {
                            return;
                        };
                        let Ok(mut child) = Command::new(exe)
                            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                            .arg(dir.join("recording.mp4"))
                            .args(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "5"])
                            .arg(dir.join("thumbnail.jpg"))
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .spawn()
                        else {
                            return;
                        };
                        let until = std::time::Instant::now() + Duration::from_secs(8);
                        loop {
                            if child.try_wait().ok().flatten().is_some() {
                                break;
                            }
                            if std::time::Instant::now() >= until {
                                let _ = child.kill();
                                let _ = child.wait();
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        if dir.join("thumbnail.jpg").is_file() {
                            let mut s = store.lock().unwrap();
                            if let Ok(i) = s.position(&id) {
                                s.index["recordings"][i]["thumbnailReady"] = json!(true);
                                let _ = s.persist();
                            }
                        }
                    });
                }
                Ok(r)
            }

            "settings" => {
                let result = self.store.lock().unwrap().action(op, d)?;
                self.post_command("preferences");
                Ok(result)
            }

            "quit" => {
                {
                    let s = self.store.lock().unwrap();
                    if !s.active.is_null() {
                        return Err("Stop and save first".into());
                    }
                    self.quit_requested.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                self.post_command("quit");
                super::shares::clear(&self.shares);
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(400));
                    QUITTING.store(true, std::sync::atomic::Ordering::Relaxed);
                });
                Ok(json!({"quitting":true}))
            }
            "state" => {
                let mut v = self.store.lock().unwrap().snapshot();
                v["shares"] = super::shares::list(&self.shares);
                Ok(v)
            }
            "share" => {
                // The store lock spans validation and reservation, so archive and quit cannot interleave between
                // "this take may be shared" and "a share exists"; the tunnel then starts with every lock released.
                let s = self.store.lock().unwrap();
                if self.quitting() {
                    return Err(QUIT_MESSAGE.into());
                }
                let id = d["id"].as_str().ok_or("Choose a recording")?;
                let minutes = d["minutes"].as_u64().ok_or("Choose a link duration")?;
                let i = s.position(&d["id"])?;
                let r = &s.index["recordings"][i];
                if r["status"] != "ready" || r["archived"] == true {
                    return Err("Share a completed recording".into());
                }
                let file = s.file(id, s.main_name(&d["id"])?)?;
                super::shares::start(&self.shares, cloudflared(), id.to_string(), &file, minutes)
            }
            "unshare" => {
                super::shares::revoke(&self.shares, d["id"].as_str().ok_or("Choose a recording")?, d["shareId"].as_str());
                Ok(json!({"revoked":true}))
            }
            "update" => {
                let result = self.store.lock().unwrap().action(op, d.clone())?;
                if d["patch"]["archived"] == true {
                    // The record is already archived, so no new share can be reserved; this cancels earlier ones,
                    // including any still starting.
                    super::shares::revoke(&self.shares, d["id"].as_str().unwrap_or(""), None);
                }
                Ok(result)
            }
            "capture-command" => {
                if d["command"] == "cancel" {
                    *self.pending_start.lock().unwrap() = None;
                }
                self.queue_human(&d)
            }
            "start-recording" => self.start_recording_with(&d, || self.open_studio()),
            "media-session" => Ok(json!({"origin":self.origin,"mediaKey":self.media_key,"version":VERSION})),
            "media-read" => self.media_read(&d),
            "native-progress" => {
                let reply = self.store.lock().unwrap().action(op, d)?;
                self.deliver_pending_start();
                Ok(reply)
            }
            "stop" => {
                *self.pending_start.lock().unwrap() = None;
                let (result, generation, launch) = {
                    let mut s = self.store.lock().unwrap();
                    let generation = s.capture["generation"].as_u64();
                    let launch = s.capture["launch"].as_str().map(str::to_owned);
                    (s.action(op, d)?, generation, launch)
                };
                // Tell the native app too, so its menu and timer leave "recording" at the same moment as Home's.
                let mut queue = self.human.lock().unwrap();
                if queue.len() < HUMAN_QUEUE_MAX {
                    let id = self.human_seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    // Made here, now, for the running app; an app that has not reported has no launch to name.
                    if let (Some(g), Some(launch)) = (generation, launch) {
                        let t = now();
                        queue.push_back(Human { id, name: "stop".into(), payload: json!({"generation":g,"launch":launch,"nonce":uuid::Uuid::new_v4().to_string(),"expires":t + HUMAN_TTL_MS}), issued_at: t });
                    }
                }
                drop(queue);
                drop(self.command.lock().unwrap());
                self.wake.notify_one();
                Ok(result)
            }
            "native-command" => {
                {
                    let mut s = self.store.lock().unwrap();
                    s.last_poll = now();
                    // A poll repairs a registration whose first request never reached this host.
                    if let Ok(Some(pid)) = process_id(&d["pid"]) {
                        s.register_native(pid, process_liveness);
                    }
                }
                let mut guard = self.command.lock().unwrap();
                if guard.is_none() && self.human.lock().unwrap().is_empty() {
                    guard = self
                        .wake
                        .wait_timeout(guard, Duration::from_secs(25))
                        .unwrap()
                        .0;
                }
                // A person's control goes first; stale ones were dropped unread.
                if let Some(h) = self.next_human() {
                    return Ok(json!({"command":"human","human":{"id":h.id,"name":h.name,"payload":h.payload,"issuedAt":h.issued_at}}));
                }
                Ok(json!({"command":guard.take().unwrap_or("idle".into())}))
            }
            "open-library" => {
                // The route is a fixed word, never free text: Home, its Settings, or the newest stopped recording.
                let route = library_route(&d["route"])?;
                open(&format!("{}/#{}{}", self.origin, self.key, route), false)?;
                Ok(json!({"opened":true}))
            }
            "open-studio" => self.open_studio(),
            _ => self.store.lock().unwrap().action(op, d),
        }
    }
}
fn cloudflared() -> PathBuf {
    std::env::var_os("OPENRECORDER_CLOUDFLARED").map(PathBuf::from).unwrap_or_else(|| {
        if cfg!(target_os = "macos") { "/opt/homebrew/bin/cloudflared".into() } else { "cloudflared".into() }
    })
}
/// The page `open-library` may open: a fixed word, never free text or a path.
fn library_route(route: &Value) -> Result<&'static str, String> {
    match route {
        Value::Null => Ok(""),
        Value::String(s) => match s.as_str() {
            "" | "library" => Ok(""),
            "settings" => Ok("/settings"),
            "last" => Ok("/last"),
            _ => Err("Unknown page".into()),
        },
        _ => Err("Unknown page".into()),
    }
}
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).unwrap()
}
fn handle(mut req: tiny_http::Request, h: Arc<Host>) {
    let host = req
        .headers()
        .iter()
        .find(|x| x.field.equiv("Host"))
        .map(|x| x.value.as_str())
        .unwrap_or("");
    if format!("http://{host}") != h.origin {
        let _ = req.respond(Response::empty(403));
        return;
    }
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("");
    if path == "/font.ttf" && req.method().as_str() == "GET" {
        let _ = req.respond(
            Response::from_data(include_bytes!("../../assets/fonts/Inter.ttf").as_slice())
                .with_header(header("Content-Type", "font/ttf")),
        );
        return;
    }
    if path == "/" && req.method().as_str() == "GET" {
        let _=req.respond(Response::from_string(h.markup(false)).with_header(header("Content-Type","text/html; charset=utf-8")).with_header(header("Content-Security-Policy","default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; media-src 'self'; font-src 'self'; base-uri 'none'; frame-ancestors 'none'")));
        return;
    }
    let query = |name: &str| -> Option<String> {
        let prefix = format!("{name}=");
        url.split('?').nth(1).and_then(|q| q.split('&').find_map(|x| x.strip_prefix(prefix.as_str()).map(str::to_owned)))
    };
    let owner = req
        .headers()
        .iter()
        .find(|x| x.field.equiv("Authorization"))
        .map(|x| x.value.as_str().strip_prefix("Bearer ").unwrap_or("").to_owned())
        .or_else(|| query("key"));
    let media = path.starts_with("/file/");
    let owner_ok = owner.as_deref() == Some(h.key.as_str());
    // The scoped media capability opens only GET/HEAD /file/<id>/<name>; everything else needs the owner key.
    let media_ok = media && ["GET", "HEAD", "OPTIONS"].contains(&req.method().as_str()) && query("media").as_deref() == Some(h.media_key.as_str());
    if !owner_ok && !media_ok {
        let _ = req.respond(Response::empty(403));
        return;
    }
    if !media
        && req
            .headers()
            .iter()
            .any(|x| x.field.equiv("Origin") && x.value.as_str() != h.origin)
    {
        let _ = req.respond(Response::empty(403));
        return;
    }
    // A browser asks first when a page from another origin (or a private network) reads this address with a header
    // such as Range. Answered only for the media capability, and only with what GET/HEAD already allow.
    if media_ok && req.method().as_str() == "OPTIONS" {
        let mut response = Response::empty(204)
            .with_header(header("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS"))
            .with_header(header("Access-Control-Allow-Headers", "Range"))
            .with_header(header("Access-Control-Allow-Private-Network", "true"))
            .with_header(header("Access-Control-Max-Age", "600"));
        for extra in media_cors() {
            response.add_header(extra);
        }
        let _ = req.respond(response);
        return;
    }
    if media && ["GET", "HEAD"].contains(&req.method().as_str()) {
        let capability = query("media").as_deref() == Some(h.media_key.as_str());
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() == 4 {
            let file = h.store.lock().unwrap().file(parts[2], parts[3]);
            if let Ok(p) = file {
                if let Ok(mut f) = fs::File::open(p) {
                    let total = f.metadata().map(|m| m.len()).unwrap_or(0);
                    let range = req
                        .headers()
                        .iter()
                        .find(|x| x.field.equiv("Range"))
                        .map(|x| x.value.as_str());
                    let mut start = 0;
                    let mut end = total.saturating_sub(1);
                    let mut status = 200;
                    if let Some(range) = range {
                        let parsed = range.strip_prefix("bytes=").and_then(|r| r.split_once('-'));
                        if let Some((a, b)) = parsed {
                            if let Ok(n) = a.parse::<u64>() {
                                start = n;
                                if !b.is_empty() {
                                    end = b.parse().unwrap_or(total);
                                }
                                status = 206;
                            } else {
                                let _ = req.respond(Response::empty(416));
                                return;
                            }
                        } else {
                            let _ = req.respond(Response::empty(416));
                            return;
                        }
                    }
                    if start >= total || end < start || end >= total {
                        let _ = req.respond(
                            Response::empty(416)
                                .with_header(header("Content-Range", &format!("bytes */{total}"))),
                        );
                        return;
                    }
                    let len = end - start + 1;
                    let _ = f.seek(SeekFrom::Start(start));
                    let mut response = Response::new(
                        StatusCode(status),
                        vec![
                            header("Content-Type", mime_of(parts[3])),
                            header("Accept-Ranges", "bytes"),
                            header("Cache-Control", "no-store"),
                        ],
                        f.take(len),
                        Some(len as usize),
                        None,
                    );
                    if status == 206 {
                        response.add_header(header(
                            "Content-Range",
                            &format!("bytes {start}-{end}/{total}"),
                        ));
                    }
                    if capability {
                        for extra in media_cors() {
                            response.add_header(extra);
                        }
                    }
                    let _ = req.respond(response);
                    return;
                }
            }
        }
        let _ = req.respond(Response::empty(404));
        return;
    }
    let result = if path == "/api/state" && req.method().as_str() == "GET" {
        // Only the owner's Home page reads state this way, so this is how the app learns whether Home is open.
        h.store.lock().unwrap().home_seen_at = now();
        h.action("state", json!({}))
    } else if path == "/api/view" {
        let _ = req.respond(
            Response::from_string(h.markup(true)).with_header(header("Content-Type", "text/html")),
        );
        return;
    } else if path == "/api/action" && req.method().as_str() == "POST" {
        if req.body_length().is_some_and(|n| n > 65536) {
            let _ = req.respond(Response::empty(413));
            return;
        }
        let mut body = String::new();
        match req.as_reader().take(65537).read_to_string(&mut body) {
            Ok(_) if body.len() <= 65536 => match serde_json::from_str::<Value>(&body) {
                Ok(v) => h.action(v["op"].as_str().unwrap_or(""), v["data"].clone()),
                Err(_) => Err("Invalid JSON".into()),
            },
            _ => Err("Request too large".into()),
        }
    } else {
        Err("Not found".into())
    };
    let (status, value) = match result {
        Ok(v) => (200, v),
        Err(e) if e == QUIT_MESSAGE => (503, json!({"error":e})),
        Err(e) => (400, json!({"error":e})),
    };
    let _ = req.respond(
        Response::from_string(value.to_string())
            .with_status_code(status)
            .with_header(header("Content-Type", "application/json"))
            .with_header(header("Cache-Control", "no-store")),
    );
}
/// Become the single host for this home. Ownership is the OS lease, not a liveness probe: an owner that is slow,
/// busy or still starting is waited for and adopted, never replaced. Only the caller that holds the lease may
/// construct a `Store` or publish descriptors. Returns the live owner's descriptor when another process owns it.
pub fn start() -> Result<Value, String> {
    let home = home();
    fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let lease = open_lease(&home)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    loop {
        if fs2::FileExt::try_lock_exclusive(&lease).is_ok() {
            break;
        }
        if let Some(v) = descriptor() {
            if request(&v, "state", json!({})).is_ok() {
                return Ok(v);
            }
        }
        if std::time::Instant::now() >= deadline {
            // The owner still holds the lease but does not answer. Starting another host would split the library.
            return Ok(json!({"pid":null,"owned":false,"busy":true}));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let runtime = home.join("rust-runtime.json");
    let server = Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
    let origin = format!("http://{}", server.server_addr());
    let key = token();
    let host = Arc::new(Host {
        shares: super::shares::managed(),
        store: Arc::new(Mutex::new(Store::new(home.clone())?)),
        human: Mutex::new(std::collections::VecDeque::new()),
        human_seq: std::sync::atomic::AtomicU64::new(0),
        human_nonces: Mutex::new(std::collections::VecDeque::new()),
        pending_start: Mutex::new(None),
        command: Mutex::new(None),
        wake: Condvar::new(),
        origin: origin.clone(),
        key: key.clone(),
        media_key: token(),
        quit_requested: std::sync::atomic::AtomicBool::new(false),
    });
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let helper = std::env::var("OPENRECORDER_FFMPEG")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| {
            exe.parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("Helpers/ffmpeg"))
                .filter(|p| p.is_file())
        })
        .or_else(|| {
            exe.parent()
                .map(|p| {
                    p.join(if cfg!(target_os = "windows") {
                        "ffmpeg.exe"
                    } else {
                        "ffmpeg"
                    })
                })
                .filter(|p| p.is_file())
        });
    let completed = read(&home.join("onboarding.json"), Value::Null)?["completed"] == true;
    let config = json!({"pid":std::process::id(),"origin":origin,"key":key,"setup_complete":completed,"ffmpeg":helper,"home":home});
    save(&home.join("rust-connection.json"), &config)?;
    save(&runtime, &config)?;

    let concurrent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            if concurrent.load(std::sync::atomic::Ordering::Relaxed) >= 24 {
                let _ = req.respond(Response::empty(503));
                continue;
            }
            concurrent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let h = host.clone();
            let c = concurrent.clone();
            std::thread::spawn(move || {
                handle(req, h);
                c.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            });
        }
    });
    // Retain the lease until this process exits or shuts down.
    *PUBLISHED.lock().unwrap() = Some(config.clone());
    *LEASE.lock().unwrap() = Some(lease);
    Ok(config)
}
pub fn gui_config() -> Result<PathBuf, String> {
    ensure_host()?;
    Ok(home().join("rust-connection.json"))
}
/// Why a request to the library host failed. Only `Stale` guarantees the operation did not run.
enum HostError {
    Stale(String),
    Rejected(String),
    Failed(String),
}
impl HostError {
    fn retry_safe(&self) -> bool {
        matches!(self, HostError::Stale(_))
    }
    fn message(self) -> String {
        match self {
            HostError::Stale(m) | HostError::Rejected(m) | HostError::Failed(m) => m,
        }
    }
}
fn host_error(e: ureq::Error) -> HostError {
    match e {
        ureq::Error::Status(code, response) => {
            let message = response.into_json::<Value>().ok().and_then(|v| v["error"].as_str().map(str::to_owned));
            match (code, message) {
                // The descriptor names a different (replaced) host, or this one is quitting: nothing ran.
                (403, _) => HostError::Stale("The local library key was not accepted".into()),
                (503, Some(m)) if m == QUIT_MESSAGE => HostError::Stale(m),
                (_, m) => HostError::Rejected(m.unwrap_or_else(|| format!("The local library refused the request ({code})"))),
            }
        }
        // The connection was never established, so the request cannot have been delivered.
        ureq::Error::Transport(t) if t.kind() == ureq::ErrorKind::ConnectionFailed => HostError::Stale(t.to_string()),
        ureq::Error::Transport(t) => HostError::Failed(t.to_string()),
    }
}
fn connection(c: &Value) -> Result<(&str, &str), HostError> {
    match (c["origin"].as_str(), c["key"].as_str()) {
        (Some(origin), Some(key)) => Ok((origin, key)),
        _ => Err(HostError::Stale("No local library".into())),
    }
}
fn request_raw(c: &Value, op: &str, data: Value) -> Result<Value, HostError> {
    let (origin, key) = connection(c)?;
    ureq::post(&format!("{origin}/api/action"))
        .set("Authorization", &format!("Bearer {key}"))
        .timeout(Duration::from_secs(if op == "share" { 40 } else { 5 }))
        .send_json(json!({"op":op,"data":data}))
        .map_err(host_error)?
        .into_json()
        .map_err(|e| HostError::Failed(e.to_string()))
}
fn request(c: &Value, op: &str, data: Value) -> Result<Value, String> {
    request_raw(c, op, data).map_err(HostError::message)
}
fn fetch_view(c: &Value) -> Result<(String, String), HostError> {
    let (origin, key) = connection(c)?;
    let text = ureq::get(&format!("{origin}/api/view"))
        .set("Authorization", &format!("Bearer {key}"))
        .timeout(Duration::from_secs(5))
        .call()
        .map_err(host_error)?
        .into_string()
        .map_err(|e| HostError::Failed(e.to_string()))?;
    Ok((text, origin.to_owned()))
}
// Redundant `--library-host` starters spawned by this process. They are only reaped, never killed:
// a loser of the singleton race exits on its own, and the winner is the owner's long-lived host.
static STARTERS: Mutex<Vec<(std::process::Child, std::time::Instant)>> = Mutex::new(Vec::new());
const STARTER_GRACE: Duration = Duration::from_secs(15);
fn reap_starters() {
    if let Ok(mut starters) = STARTERS.lock() {
        starters.retain_mut(|(child, _)| matches!(child.try_wait(), Ok(None)));
    }
}
// A starter is pending while it is young and has not published a descriptor. One whose pid is the (unusable,
// e.g. quitting) published host is not starting anything.
fn starter_pending(published_pid: Option<u64>) -> bool {
    STARTERS.lock().is_ok_and(|s| {
        s.iter().any(|(child, at)| at.elapsed() < STARTER_GRACE && Some(child.id() as u64) != published_pid)
    })
}
// The published descriptor, only if it names a loopback origin. A damaged file counts as absent.
fn descriptor() -> Option<Value> {
    let c = read(&home().join("rust-runtime.json"), Value::Null).ok()?;
    let port = c["origin"].as_str()?.strip_prefix("http://127.0.0.1:")?.parse::<u16>().ok()?;
    (port > 0 && c["key"].as_str().is_some()).then_some(c)
}
fn spawn_host() -> Result<(), String> {
    #[cfg(test)]
    return Err("Starting a host is disabled in unit tests".into());
    #[cfg(not(test))]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut cmd = Command::new(exe);
        cmd.arg("--library-host").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let child = cmd.spawn().map_err(|e| e.to_string())?;
        if let Ok(mut starters) = STARTERS.lock() {
            starters.push((child, std::time::Instant::now()));
        }
        Ok(())
    }
}
/// Adopt the live library host, starting one only when none answers. Concurrent callers converge on one
/// host: the host's own start lock admits a single winner, and callers never spawn while one is starting.
fn ensure_host() -> Result<Value, String> {
    reap_starters();
    let published = descriptor();
    if let Some(c) = &published {
        if request(c, "state", json!({})).is_ok() {
            return Ok(c.clone());
        }
    }
    let home = home();
    fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    // A host is spawned only while nobody owns the home. A slow, busy, starting or quitting owner is waited for:
    // it keeps the lease, so a probe that merely times out can never authorize a second host.
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        let published_pid = descriptor().and_then(|c| c["pid"].as_u64());
        if !starter_pending(published_pid) && !lease_held_elsewhere(&home) {
            spawn_host()?;
        }
        std::thread::sleep(Duration::from_millis(100));
        reap_starters();
        if let Some(c) = descriptor() {
            if request(&c, "state", json!({})).is_ok() {
                return Ok(c);
            }
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
    }
    Err(if lease_held_elsewhere(&home) {
        "OpenRecorder is busy and did not answer. Try again shortly."
    } else {
        "Local library did not start"
    }
    .into())
}
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
struct RpcError(i64, String);
fn invalid_params(message: impl Into<String>) -> RpcError {
    RpcError(INVALID_PARAMS, message.into())
}
fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
const CACHEABLE_METHODS: &[&str] = &["server/discover", "tools/list", "resources/list", "resources/templates/list", "resources/read"];
// Add the server identity beside any metadata the result already carries (for example the UI link of Home).
fn merge_server_info(result: &mut Value) {
    if !result["_meta"].is_object() {
        result["_meta"] = json!({});
    }
    result["_meta"]["io.modelcontextprotocol/serverInfo"] = server_info();
}
fn negotiate(requested: &str) -> &'static str {
    SUPPORTED_VERSIONS.iter().copied().find(|v| *v == requested).unwrap_or(LATEST_VERSION)
}
fn tools_list() -> Value {
    let home_meta = json!({"ui":{"resourceUri":URI},"openai/outputTemplate":URI,"openai/ui":{"entrypoints":[{"type":"global"},{"type":"thread"}]}});
    json!({"tools":[
        {"name":"openrecorder_home","title":TITLE,"description":"Open your local recording library.","icons":icons(),"inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":{"readOnlyHint":true},"_meta":home_meta},
        {"name":"openrecorder_action","title":"OpenRecorder library","description":"Manage the local native recorder and library. Recording starts only when the person presses Record.","inputSchema":{"type":"object","properties":{"op":{"type":"string","enum":PUBLIC_OPS},"data":{"type":"object"}},"required":["op"],"additionalProperties":false},"annotations":{"readOnlyHint":false,"destructiveHint":false,"openWorldHint":true}},
        // Used only by the OpenRecorder page itself (poster and playback bytes, a fresh media address, the person's Record
        // press). The host hides app-only tools from the model; every operation also carries its own launch, take,
        // expiry and single-use binding and is checked again by the library, so being listed is not what protects it.
        {"name":"openrecorder_ui","title":"OpenRecorder page","description":"For the OpenRecorder page only.","inputSchema":{"type":"object","properties":{"op":{"type":"string","enum":UI_OPS},"data":{"type":"object"},"ui":{"type":"string"}},"required":["op","ui"],"additionalProperties":false},"annotations":{"readOnlyHint":false,"destructiveHint":false,"openWorldHint":false},"_meta":{"ui":{"visibility":["app"]}}}
    ]})
}
enum Message {
    Line(Vec<u8>),
    TooLong,
}
// Reads one newline-terminated message of at most MAX_MESSAGE bytes (terminator excluded). An oversized
// message is discarded through its newline without being buffered.
fn read_message(input: &mut impl BufRead) -> std::io::Result<Option<Message>> {
    let mut bytes = Vec::new();
    let count = input.by_ref().take(MAX_MESSAGE as u64 + 2).read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    let terminated = bytes.last() == Some(&b'\n');
    if terminated {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    if bytes.len() <= MAX_MESSAGE {
        return Ok(Some(Message::Line(bytes)));
    }
    let mut done = terminated;
    while !done {
        let chunk = input.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        let used = chunk.iter().position(|b| *b == b'\n').map_or(chunk.len(), |i| {
            done = true;
            i + 1
        });
        input.consume(used);
    }
    Ok(Some(Message::TooLong))
}
struct Bridge {
    conn: Option<Value>,
    /// Names this MCP session to the page it served: put in the app window's own copy of the page (the resource the
    /// host reads for it, never a tool answer the model sees) and required by the page-only tool. It lives with the
    /// bridge, not the library, so restarting the library behind it leaves a page that is still open working.
    ui_key: String,
}
impl Default for Bridge {
    fn default() -> Self {
        Bridge { conn: None, ui_key: token() }
    }
}
impl Bridge {
    // Run one host request. The library is restarted and the request retried once only when the host
    // provably did not run it (refused connection, replaced host, or quitting host).
    fn with_host<T>(&mut self, f: impl Fn(&Value) -> Result<T, HostError>) -> Result<T, String> {
        for attempt in 0..2 {
            let c = match self.conn.clone() {
                Some(c) => c,
                None => {
                    let c = ensure_host()?;
                    self.conn = Some(c.clone());
                    c
                }
            };
            match f(&c) {
                Ok(v) => return Ok(v),
                Err(e) if attempt == 0 && e.retry_safe() => self.conn = None,
                Err(e) => return Err(e.message()),
            }
        }
        Err("Local library did not start".into())
    }
    fn handle_line(&mut self, line: &[u8]) -> Option<Value> {
        let Ok(text) = std::str::from_utf8(line) else {
            return Some(failure(Value::Null, PARSE_ERROR, "Parse error: input is not UTF-8"));
        };
        if text.trim().is_empty() {
            return None;
        }
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            return Some(failure(Value::Null, PARSE_ERROR, "Parse error"));
        };
        let Some(object) = message.as_object() else {
            return Some(failure(Value::Null, INVALID_REQUEST, "Invalid Request: expected one JSON-RPC object"));
        };
        let id = object.get("id");
        let usable = id.filter(|v| v.is_string() || v.is_i64() || v.is_u64()).cloned().unwrap_or(Value::Null);
        let Some(method) = object.get("method") else {
            // A reply to a server request is ignored; this server sends none.
            if id.is_some() && (object.contains_key("result") || object.contains_key("error")) {
                return None;
            }
            return Some(failure(usable, INVALID_REQUEST, "Invalid Request: missing method"));
        };
        // Notifications never get a response, whatever they contain.
        id?;
        let Some(method) = method.as_str() else {
            return Some(failure(usable, INVALID_REQUEST, "Invalid Request: method must be a string"));
        };
        if usable.is_null() {
            return Some(failure(usable, INVALID_REQUEST, "Invalid Request: id must be a string or integer"));
        }
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(failure(usable, INVALID_REQUEST, "Invalid Request: jsonrpc must be \"2.0\""));
        }
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        if !(params.is_null() || params.is_object() || params.is_array()) {
            return Some(failure(usable, INVALID_REQUEST, "Invalid Request: params must be structured"));
        }
        Some(match self.dispatch(method, &params) {
            Ok(mut result) => {
                // 2026-07-28 requires non-negative caching hints on complete results of these methods. Everything
                // here is user-bound (library state, embedded resource), so it is private and immediately stale.
                if CACHEABLE_METHODS.contains(&method) {
                    result["ttlMs"] = json!(0);
                    result["cacheScope"] = json!("private");
                }
                if method == "server/discover" || params["_meta"]["io.modelcontextprotocol/protocolVersion"] == "2026-07-28" {
                    result["resultType"] = json!("complete");
                    merge_server_info(&mut result);
                }
                json!({"jsonrpc":"2.0","id":usable,"result":result})
            }
            Err(RpcError(code, message)) => failure(usable, code, &message),
        })
    }
    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, RpcError> {
        let info = server_info();
        let capabilities = json!({"tools":{},"resources":{}});
        match method {
            "initialize" => {
                let requested = params["protocolVersion"].as_str().ok_or_else(|| invalid_params("initialize requires a protocolVersion string"))?;
                Ok(json!({"protocolVersion":negotiate(requested),"serverInfo":info,"capabilities":capabilities}))
            }
            "server/discover" => Ok(json!({"supportedVersions":["2026-07-28"],"serverInfo":info,"capabilities":capabilities})),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(tools_list()),
            "resources/list" => Ok(json!({"resources":[{"uri":URI,"name":"OpenRecorder","title":TITLE,"icons":icons(),"mimeType":"text/html;profile=mcp-app"}]})),
            "resources/templates/list" => Ok(json!({"resourceTemplates":[]})),
            "resources/read" => {
                let uri = params["uri"].as_str().ok_or_else(|| invalid_params("resources/read requires a uri string"))?;
                if uri != URI {
                    return Err(invalid_params(format!("Unknown resource: {uri}")));
                }
                let (text, origin) = self.with_host(fetch_view).map_err(|e| RpcError(-32603, format!("Local library unavailable: {e}")))?;
                let text = text.replacen("const CONNECTION={", &format!("const CONNECTION={{\"uiKey\":{},", json!(self.ui_key)), 1);
                Ok(json!({"contents":[{"uri":URI,"mimeType":"text/html;profile=mcp-app","text":text,"_meta":{"ui":{"csp":{"connectDomains":[],"resourceDomains":[origin,"data:","blob:"]},"permissions":{"clipboardWrite":{}},"prefersBorder":false}}}]}))
            }
            "tools/call" => {
                let name = params["name"].as_str().ok_or_else(|| invalid_params("tools/call requires a tool name"))?;
                let arguments = match params.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(a) if a.is_object() => a.clone(),
                    Some(_) => return Err(invalid_params("tools/call arguments must be an object")),
                };
                let outcome = match name {
                    // Model-visible output never carries the owner key. The browser fallback is the `open-library`
                    // operation, which the host performs itself.
                    "openrecorder_home" => self.with_host(|c| {
                        request_raw(c, "state", json!({})).map(|mut v| {
                            v["origin"] = json!(c["origin"].as_str().unwrap_or(""));
                            v["browser"] = json!("Call openrecorder_action with op open-library to open the private library in the browser. No access link is returned.");
                            v
                        })
                    }),
                    "openrecorder_ui" => {
                        let op = arguments["op"].as_str().unwrap_or("");
                        let data = arguments.get("data").cloned().unwrap_or(Value::Null);
                        if arguments["ui"].as_str() != Some(self.ui_key.as_str()) {
                            Err("This tool belongs to the OpenRecorder window of this session and cannot be used from here.".into())
                        } else if !UI_OPS.contains(&op) {
                            Err("Not an operation of the OpenRecorder page".into())
                        } else if !(data.is_null() || data.is_object()) {
                            Err("data must be an object".into())
                        } else {
                            self.with_host(|c| request_raw(c, op, data.clone()))
                        }
                    }
                    "openrecorder_action" => {
                        let op = arguments["op"].as_str().unwrap_or("");
                        let data = arguments.get("data").cloned().unwrap_or(Value::Null);
                        if !PUBLIC_OPS.contains(&op) {
                            Err("This operation is not available to agents".into())
                        } else if !(data.is_null() || data.is_object()) {
                            Err("data must be an object".into())
                        } else {
                            self.with_host(|c| request_raw(c, op, data.clone()))
                        }
                    }
                    _ => return Err(invalid_params(format!("Unknown tool: {name}"))),
                };
                Ok(match outcome {
                    Ok(v) => json!({"content":[{"type":"text","text":v.to_string()}],"structuredContent":v,"_meta":if name == "openrecorder_home" {json!({"ui":{"resourceUri":URI},"openai/outputTemplate":URI})}else{json!({})}}),
                    Err(e) => json!({"content":[{"type":"text","text":e}],"isError":true}),
                })
            }
            _ => Err(RpcError(METHOD_NOT_FOUND, "Method not found".into())),
        }
    }
}
pub fn stdio() -> Result<(), String> {
    let mut bridge = Bridge::default();
    // Loading the plugin brings the shared host up (no UI, helper or hardware). A failure is not fatal:
    // the next tool call tries again.
    match ensure_host() {
        Ok(c) => bridge.conn = Some(c),
        Err(e) => eprintln!("openrecorder: local library not ready ({e}); it will be retried on first use"),
    }
    let mut input = std::io::stdin().lock();
    let mut out = std::io::stdout().lock();
    loop {
        let reply = match read_message(&mut input).map_err(|e| e.to_string())? {
            None => break,
            Some(Message::TooLong) => {
                eprintln!("openrecorder: MCP message exceeds the local input limit and was rejected");
                Some(failure(Value::Null, INVALID_REQUEST, "Message exceeds the 131072-byte local input limit"))
            }
            Some(Message::Line(bytes)) => {
                reap_starters();
                bridge.handle_line(&bytes)
            }
        };
        // stdout carries protocol messages only; a closed pipe ends the bridge, never the host.
        if let Some(reply) = reply {
            if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_library_preserves_take_and_storage() {
        let home = std::env::temp_dir().join(format!("rust-library-test-{}", uuid::Uuid::new_v4()));
        let mut s = Store::new(home.clone()).unwrap();
        let root = home.join("media");
        let r = s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root})).unwrap();
        assert!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root})).is_err());
        fs::write(
            PathBuf::from(r["directory"].as_str().unwrap()).join("recording.mp4"),
            b"test",
        )
        .unwrap();
        let done = s
            .action("native-finish", json!({"id":r["id"],"duration":1}))
            .unwrap();
        assert_eq!(done["bytes"], 4);
        assert_eq!(
            Store::new(home.clone()).unwrap().snapshot()["recordings"][0]["id"],
            r["id"]
        );
        assert!(s
            .file(r["id"].as_str().unwrap(), "../settings.json")
            .is_err());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn failed_settings_do_not_change_preferences() {
        let home = std::env::temp_dir().join(format!("recorder-settings-{}", uuid::Uuid::new_v4()));
        let mut s = Store::new(home.clone()).unwrap();
        let before = s.config.clone();
        assert!(s.action("settings", json!({"preferences":{"quality":1280,"fps":99}})).is_err());
        assert_eq!(s.config, before);
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn empty_or_missing_originals_remain_interrupted() {
        let home = std::env::temp_dir().join(format!("recorder-partial-{}", uuid::Uuid::new_v4()));
        let mut s = Store::new(home.clone()).unwrap();
        let r = s.action("native-begin", json!({"ownerPid":std::process::id(),"root":home.join("media"),"preferences":{"camera":true,"keepOriginals":true}})).unwrap();
        let dir = Path::new(r["directory"].as_str().unwrap());
        fs::write(dir.join("recording.mp4"), b"combined").unwrap();
        fs::write(dir.join("camera.mp4"), b"").unwrap();
        fs::write(dir.join("screen.mp4"), b"screen").unwrap();
        let result = s.action("native-finish", json!({"id":r["id"]})).unwrap();
        assert_eq!(result["status"], "interrupted");
        assert!(dir.join("recording.mp4").exists());
        assert!(result["error"].is_string());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn restart_recovers_abandoned_take_without_losing_files() {
        let home = std::env::temp_dir().join(format!("recorder-recovery-{}", uuid::Uuid::new_v4()));
        let mut s = Store::new(home.clone()).unwrap();
        let r = s.action("native-begin", json!({"ownerPid":std::process::id(),"root":home.join("media")})).unwrap();
        let dir = Path::new(r["directory"].as_str().unwrap());
        fs::write(dir.join("screen.mp4"), b"partial").unwrap();
        drop(s);
        let s = Store::new(home.clone()).unwrap();
        let take = &s.index["recordings"][0];
        assert_eq!(take["status"], "interrupted");
        assert_eq!(take["bytes"], 7);
        assert!(s.active.is_null());
        assert_eq!(fs::read(dir.join("screen.mp4")).unwrap(), b"partial");
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn failed_save_keeps_active_take_retryable() {
        let home = std::env::temp_dir().join(format!("recorder-save-{}", uuid::Uuid::new_v4()));
        let mut s = Store::new(home.clone()).unwrap();
        let r = s.action("native-begin", json!({"ownerPid":std::process::id(),"root":home.join("media"),"preferences":{"keepOriginals":false}})).unwrap();
        fs::write(Path::new(r["directory"].as_str().unwrap()).join("recording.mp4"), b"media").unwrap();
        fs::remove_file(home.join("library.json")).unwrap();
        fs::create_dir(home.join("library.json")).unwrap();
        assert!(s.action("native-finish", json!({"id":r["id"]})).is_err());
        assert_eq!(s.active["id"], r["id"]);
        fs::remove_dir(home.join("library.json")).unwrap();
        assert_eq!(s.action("native-finish", json!({"id":r["id"]})).unwrap()["status"], "ready");
        fs::remove_dir_all(home).unwrap();
    }

    fn test_host() -> (Host, PathBuf) {
        let home = std::env::temp_dir().join(format!("recorder-host-test-{}", uuid::Uuid::new_v4()));
        let host = Host {
            shares: crate::shares::managed(),
            store: Arc::new(Mutex::new(Store::new(home.clone()).unwrap())),
            human: Mutex::new(std::collections::VecDeque::new()),
            human_seq: std::sync::atomic::AtomicU64::new(0),
            human_nonces: Mutex::new(std::collections::VecDeque::new()),
            pending_start: Mutex::new(None),
            command: Mutex::new(None),
            wake: Condvar::new(),
            origin: "http://127.0.0.1:1".into(),
            key: "k".repeat(64),
            media_key: "m".repeat(64),
            quit_requested: std::sync::atomic::AtomicBool::new(false),
        };
        (host, home)
    }
    fn rpc(line: &str) -> Value {
        Bridge::default().handle_line(line.as_bytes()).expect("a response")
    }
    #[test]
    fn initialize_negotiates_supported_versions_and_falls_back_to_latest() {
        for v in ["2025-11-25", "2026-07-28"] {
            let r = rpc(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":v}}).to_string());
            assert_eq!(r["result"]["protocolVersion"], v);
            assert_eq!(r["result"]["serverInfo"]["name"], "OpenRecorder");
            assert!(r["result"]["capabilities"]["tools"].is_object());
        }
        let r = rpc(r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#);
        assert_eq!(r["result"]["protocolVersion"], LATEST_VERSION);
        assert_eq!(rpc(r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{}}"#)["error"]["code"], -32602);
        assert_eq!(rpc(r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{"protocolVersion":5}}"#)["error"]["code"], -32602);
    }
    #[test]
    fn discover_keeps_its_result_contract() {
        let r = rpc(r#"{"jsonrpc":"2.0","id":"a","method":"server/discover"}"#);
        assert_eq!(r["id"], "a");
        assert_eq!(r["result"]["supportedVersions"], json!(["2026-07-28"]));
        assert_eq!(r["result"]["resultType"], "complete");
        assert_eq!(r["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "OpenRecorder");
    }
    #[test]
    fn standard_error_codes_are_distinguished() {
        assert_eq!(rpc("{not json")["error"]["code"], -32700);
        assert_eq!(Bridge::default().handle_line(&[0xff, 0xfe]).unwrap()["error"]["code"], -32700);
        assert!(rpc("{not json")["id"].is_null());
        for bad in ["[1]", "42", r#"{"jsonrpc":"2.0","id":1}"#, r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#, r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":7}"#, r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":3}"#] {
            assert_eq!(rpc(bad)["error"]["code"], -32600, "{bad}");
        }
        assert_eq!(rpc(r#"{"jsonrpc":"2.0","id":9,"method":"nope"}"#)["error"]["code"], -32601);
        assert_eq!(rpc(r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{}}"#)["error"]["code"], -32602);
        assert_eq!(rpc(r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"nope"}}"#)["error"]["code"], -32602);
        assert_eq!(rpc(r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"openrecorder_home","arguments":3}}"#)["error"]["code"], -32602);
        assert_eq!(rpc(r#"{"jsonrpc":"2.0","id":9,"method":"resources/read","params":{"uri":"ui://other"}}"#)["error"]["code"], -32602);
        assert_eq!(rpc(r#"{"jsonrpc":"2.0","id":9,"method":"resources/read","params":{}}"#)["error"]["code"], -32602);
        assert_eq!(rpc(r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#)["result"], json!({}));
    }
    #[test]
    fn notifications_blank_lines_and_client_replies_get_no_response() {
        let mut b = Bridge::default();
        for line in [r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, r#"{"jsonrpc":"2.0","method":"ping"}"#,
            r#"{"method":7}"#, "", "   ", r#"{"jsonrpc":"2.0","id":4,"result":{}}"#] {
            assert!(b.handle_line(line.as_bytes()).is_none(), "{line}");
        }
    }
    #[test]
    fn agents_cannot_reach_owner_or_internal_operations() {
        for op in ["prepare-recording", "capture-command", "native-progress", "native-begin", "native-begin-resolve", "native-finish", "native-abort", "native-command", "native-window-ready", "native-window-closed", "native-onboarding-complete"] {
            assert!(!PUBLIC_OPS.contains(&op), "{op}");
            let r = rpc(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"openrecorder_action","arguments":{"op":op,"data":{}}}}).to_string());
            assert_eq!(r["result"]["isError"], true, "{op}");
            assert_eq!(r["result"]["content"][0]["text"], "This operation is not available to agents");
        }
        assert!(PUBLIC_OPS.contains(&"open-studio"));
        let schema = tools_list()["tools"][1]["inputSchema"]["properties"]["op"]["enum"].clone();
        assert_eq!(schema, json!(PUBLIC_OPS));
        assert!(!tools_list().to_string().contains("prepare-recording"));
    }
    #[test]
    fn messages_are_bounded_and_oversize_is_discarded_safely() {
        let mut ok = vec![b'a'; MAX_MESSAGE];
        ok.push(b'\n');
        ok.extend_from_slice(b"next\n");
        let mut input = std::io::Cursor::new(ok);
        assert!(matches!(read_message(&mut input).unwrap(), Some(Message::Line(l)) if l.len() == MAX_MESSAGE));
        assert!(matches!(read_message(&mut input).unwrap(), Some(Message::Line(l)) if l == b"next"));
        assert!(read_message(&mut input).unwrap().is_none());
        let mut big = vec![b'a'; MAX_MESSAGE + 1];
        big.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
        let mut input = std::io::Cursor::new(big);
        assert!(matches!(read_message(&mut input).unwrap(), Some(Message::TooLong)));
        assert!(matches!(read_message(&mut input).unwrap(), Some(Message::Line(l)) if l.starts_with(b"{")));
        // No newline at all: drained to EOF without being kept.
        let mut input = std::io::Cursor::new(vec![b'b'; MAX_MESSAGE * 3]);
        assert!(matches!(read_message(&mut input).unwrap(), Some(Message::TooLong)));
        assert!(read_message(&mut input).unwrap().is_none());
        // CRLF framing is accepted.
        let mut input = std::io::Cursor::new(b"{}\r\n".to_vec());
        assert!(matches!(read_message(&mut input).unwrap(), Some(Message::Line(l)) if l == b"{}"));
    }
    #[test]
    fn quit_refuses_an_active_take_and_otherwise_stops_new_work() {
        let (h, home) = test_host();
        let r = h.action("native-begin", json!({"ownerPid":std::process::id(),"root":home.join("media")})).unwrap();
        assert!(h.action("quit", json!({})).is_err());
        assert!(!h.quitting(), "a refused Quit must leave the host fully usable");
        assert!(h.action("state", json!({})).is_ok());
        h.action("native-abort", json!({"id":r["id"]})).unwrap();
        assert_eq!(h.action("quit", json!({})).unwrap(), json!({"quitting":true}));
        assert_eq!(h.action("state", json!({})).unwrap_err(), QUIT_MESSAGE);
        assert_eq!(h.action("native-begin", json!({"ownerPid":std::process::id(),"root":home.join("media")})).unwrap_err(), QUIT_MESSAGE);
        assert_eq!(h.command.lock().unwrap().as_deref(), Some("quit"));
        // The toolbar can still collect the quit command.
        assert_eq!(h.action("native-command", json!({})).unwrap(), json!({"command":"quit"}));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn lease_is_exclusive_per_home_and_released_with_its_holder() {
        let home = std::env::temp_dir().join(format!("recorder-lease-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&home).unwrap();
        assert!(!lease_held_elsewhere(&home));
        let owner = open_lease(&home).unwrap();
        fs2::FileExt::try_lock_exclusive(&owner).unwrap();
        // A slow or busy owner is still an owner: nobody else may start, whatever its state probe says.
        assert!(lease_held_elsewhere(&home));
        let rival = open_lease(&home).unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&rival).is_err());
        drop(owner);
        assert!(!lease_held_elsewhere(&home));
        assert!(fs2::FileExt::try_lock_exclusive(&rival).is_ok());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn markup_never_carries_the_owner_key_and_only_the_embedded_view_has_the_media_capability() {
        let (h, home) = test_host();
        for embedded in [false, true] {
            let page = h.markup(embedded);
            assert!(!page.contains(&h.key), "owner key leaked into markup");
            assert_eq!(page.contains(&h.media_key), embedded);
        }
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn cache_hints_are_private_and_stale_for_every_cacheable_result_in_both_protocols() {
        for version in ["2025-11-25", "2026-07-28"] {
            let meta = json!({"_meta":{"io.modelcontextprotocol/protocolVersion":version}});
            for method in ["server/discover", "tools/list", "resources/list", "resources/templates/list"] {
                let r = rpc(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":meta}).to_string());
                assert_eq!(r["result"]["ttlMs"], 0, "{method} {version}");
                assert_eq!(r["result"]["cacheScope"], "private", "{method} {version}");
            }
            let call = rpc(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"openrecorder_action","arguments":{"op":"native-begin"},"_meta":meta["_meta"]}}).to_string());
            assert!(call["result"]["ttlMs"].is_null(), "tools/call is not cacheable");
            assert!(call["result"]["cacheScope"].is_null());
        }
        assert!(rpc(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)["result"]["ttlMs"].is_null());
    }
    #[test]
    fn protocol_metadata_is_merged_without_losing_existing_ui_metadata() {
        // The shape openrecorder_home returns, wrapped for 2026-07-28.
        let mut home = json!({"content":[],"structuredContent":{},"_meta":{"ui":{"resourceUri":URI},"openai/outputTemplate":URI}});
        merge_server_info(&mut home);
        assert_eq!(home["_meta"]["ui"]["resourceUri"], URI);
        assert_eq!(home["_meta"]["openai/outputTemplate"], URI);
        assert_eq!(home["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "OpenRecorder");
        let mut bare = json!({"tools":[]});
        merge_server_info(&mut bare);
        assert!(bare["_meta"]["io.modelcontextprotocol/serverInfo"].is_object());
        let discover = rpc(r#"{"jsonrpc":"2.0","id":1,"method":"server/discover"}"#);
        assert_eq!(discover["result"]["resultType"], "complete");
    }
    #[test]
    fn prepare_recording_no_longer_exists_over_http_or_mcp() {
        let (h, home) = test_host();
        assert!(h.action("prepare-recording", json!({})).unwrap_err().contains("Unsupported"));
        assert!(!PUBLIC_OPS.contains(&"prepare-recording"));
        assert!(!include_str!("library_host.rs").split("#[cfg(test)]").next().unwrap().contains("--prepare-recording"));
        assert_eq!(desktop_args(Path::new("/c")), vec![std::ffi::OsString::from("--desktop"), "--config".into(), "/c".into()], "the host launches the normal desktop entry, never the development toolbar");
        fs::remove_dir_all(home).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn archive_cannot_be_overtaken_by_a_share_and_share_needs_a_duration() {
        use std::os::unix::fs::PermissionsExt;
        let (h, home) = test_host();
        let fake = home.join("fake-cloudflared");
        fs::write(&fake, "#!/bin/sh\nexec sleep 20\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("OPENRECORDER_CLOUDFLARED", &fake);
        let take = h.action("native-begin", json!({"ownerPid":std::process::id(),"root":home.join("media")})).unwrap();
        fs::write(Path::new(take["directory"].as_str().unwrap()).join("recording.mp4"), b"synthetic").unwrap();
        h.action("native-finish", json!({"id":take["id"],"duration":1})).unwrap();
        assert!(h.action("share", json!({"id":take["id"]})).is_err(), "no silent default duration");
        let item = h.action("share", json!({"id":take["id"],"minutes":15})).unwrap();
        assert_eq!(item["status"], "starting");
        assert_eq!(h.action("state", json!({})).unwrap()["shares"][0]["status"], "starting");
        h.action("update", json!({"id":take["id"],"patch":{"archived":true}})).unwrap();
        assert!(h.action("state", json!({})).unwrap()["shares"].as_array().unwrap().is_empty());
        assert_eq!(h.action("share", json!({"id":take["id"],"minutes":15})).unwrap_err(), "Share a completed recording");
        h.action("update", json!({"id":take["id"],"patch":{"archived":false}})).unwrap();
        h.action("share", json!({"id":take["id"],"minutes":15})).unwrap();
        h.action("quit", json!({})).unwrap();
        assert!(super::super::shares::list(&h.shares).as_array().unwrap().is_empty(), "quit clears pending shares");
        assert_eq!(h.action("share", json!({"id":take["id"],"minutes":15})).unwrap_err(), QUIT_MESSAGE);
        std::env::remove_var("OPENRECORDER_CLOUDFLARED");
        fs::remove_dir_all(home).unwrap();
    }

    fn temp_home(tag: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("recorder-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&home).unwrap();
        home
    }
    #[test]
    fn preferences_migrate_countdown_once_and_default_remembered_inputs_to_true() {
        // Legacy files (a `countdown` key, no new keys) keep their intent and end with one canonical field.
        for (legacy, expected) in [(json!(3), 3), (json!(0), 0), (json!(5), 3), (json!("x"), 3)] {
            let p = normalize_preferences(&json!({"countdown":legacy,"cameraDeviceId":"cam-1","microphoneDeviceId":"mic-1","camera":true}));
            assert_eq!(p["countdownSeconds"], expected);
            assert!(p.get("countdown").is_none(), "legacy key must not survive");
            assert_eq!(p["rememberInputs"], true, "missing means true");
            assert_eq!((p["cameraDeviceId"].as_str(), p["microphoneDeviceId"].as_str(), p["camera"].as_bool()), (Some("cam-1"), Some("mic-1"), Some(true)));
        }
        // The new key wins over a stale legacy one; an explicit stored false is kept.
        let p = normalize_preferences(&json!({"countdown":0,"countdownSeconds":3,"rememberInputs":false}));
        assert_eq!((p["countdownSeconds"].as_u64(), p["rememberInputs"].as_bool()), (Some(3), Some(false)));
        // A damaged stored value is never turned into "remember": ambiguity must not restore inputs.
        assert_eq!(normalize_preferences(&json!({"rememberInputs":"yes"}))["rememberInputs"], false);
        assert_eq!(normalize_preferences(&json!({"countdownSeconds":7}))["countdownSeconds"], 3);
        let fresh = normalize_preferences(&Value::Null);
        assert_eq!((fresh["rememberInputs"].as_bool(), fresh["countdownSeconds"].as_u64(), fresh["cameraSize"].as_u64()), (Some(true), Some(3), Some(168)), "fresh install defaults to design size M");
    }
    #[test]
    fn settings_accept_new_and_legacy_values_and_reject_invalid_ones_without_change() {
        let home = temp_home("prefs");
        let mut s = Store::new(home.clone()).unwrap();
        for size in [120, 168, 224, 140, 200, 280] {
            assert_eq!(s.action("settings", json!({"preferences":{"cameraSize":size}})).unwrap()["settings"]["cameraSize"], size);
        }
        for shape in ["circle", "rounded", "square", "wide"] {
            assert_eq!(s.action("settings", json!({"preferences":{"cameraShape":shape}})).unwrap()["settings"]["cameraShape"], shape);
        }
        s.action("settings", json!({"preferences":{"countdownSeconds":0,"rememberInputs":false,"cameraDeviceId":"cam-9"}})).unwrap();
        let saved = read(&home.join("settings.json"), Value::Null).unwrap();
        assert_eq!((saved["settings"]["countdownSeconds"].as_u64(), saved["settings"]["rememberInputs"].as_bool()), (Some(0), Some(false)));
        assert!(saved["settings"].get("countdown").is_none());
        assert_eq!(saved["settings"]["cameraDeviceId"], "cam-9");
        let before = s.config.clone();
        for bad in [json!({"cameraSize":150}), json!({"cameraSize":"168"}), json!({"cameraShape":"oval"}), json!({"cameraShape":"Wide"}),
            json!({"countdownSeconds":5}), json!({"countdownSeconds":"3"}), json!({"countdownSeconds":null}), json!({"rememberInputs":"yes"}),
            json!({"rememberInputs":null}), json!({"rememberInputs":0}), json!({"countdown":3}), json!({"rememberInputs":true,"cameraSize":999})] {
            assert!(s.action("settings", json!({"preferences":bad.clone()})).is_err(), "{bad}");
            assert_eq!(s.config, before, "an invalid request changed state: {bad}");
        }
        // The saved file survives a restart exactly: remembered-off stays off, devices untouched.
        let again = Store::new(home.clone()).unwrap();
        assert_eq!((again.config["settings"]["rememberInputs"].as_bool(), again.config["settings"]["cameraDeviceId"].as_str()), (Some(false), Some("cam-9")));
        fs::remove_dir_all(home).unwrap();
    }
    // A library as older builds wrote it: a take physically inside Folders/<folder>/.
    fn legacy_library() -> (PathBuf, PathBuf, Value) {
        let home = temp_home("legacy");
        let root = home.join("media");
        let dir = root.join("Folders").join("f1").join("take-1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("recording.mp4"), b"fake combined").unwrap();
        fs::write(dir.join("screen.mp4"), b"fake screen").unwrap();
        let take = json!({"id":"take-1","title":"Old take","folder":"f1","root":root,"directory":dir,"createdAt":1,"status":"ready","favorite":false,"archived":false,"duration":2,"bytes":24,
            "files":[{"name":"recording.mp4","bytes":13},{"name":"screen.mp4","bytes":11}],"settings":{}});
        fs::write(home.join("library.json"), json!({"version":1,"folders":[{"id":"f1","name":"One"},{"id":"f2","name":"Two"}],"recordings":[take.clone()]}).to_string()).unwrap();
        fs::write(home.join("settings.json"), json!({"root":root,"settings":{"countdown":3,"quality":1920}}).to_string()).unwrap();
        (home, dir, take)
    }
    #[test]
    fn legacy_physical_folders_stay_readable_and_filing_never_moves_bytes() {
        let (home, dir, take) = legacy_library();
        let mut s = Store::new(home.clone()).unwrap();
        assert_eq!(s.config["settings"]["countdownSeconds"], 3);
        assert_eq!(s.file("take-1", "recording.mp4").unwrap(), dir.join("recording.mp4").canonicalize().unwrap());
        for target in [json!("f2"), Value::Null, json!("f1")] {
            let r = s.action("update", json!({"id":"take-1","patch":{"folder":target.clone()}})).unwrap();
            assert_eq!(r["folder"], target);
            assert_eq!(r["directory"], take["directory"], "directory must not change");
            assert_eq!(r["root"], take["root"]);
            assert_eq!(fs::read(dir.join("recording.mp4")).unwrap(), b"fake combined");
            assert_eq!(fs::read(dir.join("screen.mp4")).unwrap(), b"fake screen");
            assert!(s.file("take-1", "screen.mp4").is_ok());
        }
        s.action("update", json!({"id":"take-1","patch":{"folder":"f2","title":"Renamed","favorite":true}})).unwrap();
        // No directory was created or renamed anywhere under the root.
        let mut seen: Vec<_> = walk(&home.join("media"));
        seen.sort();
        assert_eq!(seen, ["Folders", "Folders/f1", "Folders/f1/take-1", "Folders/f1/take-1/recording.mp4", "Folders/f1/take-1/screen.mp4"]);
        // The change is on disk and survives a restart, with the same physical path.
        let reopened = Store::new(home.clone()).unwrap();
        let r = &reopened.index["recordings"][0];
        assert_eq!((r["folder"].as_str(), r["title"].as_str(), r["favorite"].as_bool()), (Some("f2"), Some("Renamed"), Some(true)));
        assert_eq!(r["directory"], take["directory"]);
        fs::remove_dir_all(home).unwrap();
    }
    fn walk(root: &Path) -> Vec<String> {
        let mut out = vec![];
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned());
                if p.is_dir() {
                    stack.push(p);
                }
            }
        }
        out
    }
    #[test]
    fn invalid_updates_fail_before_any_change_and_save_failure_rolls_back() {
        let (home, dir, _) = legacy_library();
        let mut s = Store::new(home.clone()).unwrap();
        let before = s.index.clone();
        for patch in [json!({"folder":"nope"}), json!({"folder":5}), json!({"folder":"nope","title":"Valid"}), json!({"favorite":true,"title":""}),
            json!({"favorite":"yes"}), json!({"archived":1,"folder":"f2"}), json!("not an object")] {
            assert!(s.action("update", json!({"id":"take-1","patch":patch.clone()})).is_err(), "{patch}");
            assert_eq!(s.index, before, "{patch}");
        }
        assert!(s.action("update", json!({"id":"missing","patch":{"folder":"f2"}})).is_err());
        // The index file cannot be written: the call fails and memory still matches the unchanged file.
        let file = home.join("library.json");
        let on_disk = fs::read(&file).unwrap();
        fs::remove_file(&file).unwrap();
        fs::create_dir(&file).unwrap();
        assert!(s.action("update", json!({"id":"take-1","patch":{"folder":"f2","title":"Changed","archived":true}})).is_err());
        assert_eq!(s.index, before, "memory must roll back when the save fails");
        assert!(dir.join("recording.mp4").is_file());
        fs::remove_dir(&file).unwrap();
        fs::write(&file, &on_disk).unwrap();
        assert_eq!(s.action("update", json!({"id":"take-1","patch":{"folder":"f2"}})).unwrap()["folder"], "f2");
        assert_eq!(Store::new(home.clone()).unwrap().index["recordings"][0]["folder"], "f2");
        // The settings file is not rewritten by an index-only change.
        assert!(!fs::read_to_string(home.join("settings.json")).unwrap().contains("countdownSeconds"));
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn new_takes_live_under_the_root_whatever_their_folder_and_validate_it_first() {
        let (home, _, _) = legacy_library();
        let mut s = Store::new(home.clone()).unwrap();
        let root = home.join("media");
        let before = s.index.clone();
        assert!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root,"folder":"nope"})).is_err());
        assert!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root,"folder":7})).is_err());
        assert_eq!(s.index, before);
        assert!(!root.join("Folders").join("nope").exists());
        let r = s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root,"folder":"f2"})).unwrap();
        let id = r["id"].as_str().unwrap();
        assert_eq!(r["folder"], "f2");
        assert_eq!(PathBuf::from(r["directory"].as_str().unwrap()), root.join(id), "uuid directly under the root");
        assert!(!root.join("Folders").join("f2").exists(), "no physical folder for a metadata folder");
        s.action("native-abort", json!({"id":id})).unwrap();
        // A failed save leaves no half-created take: memory restored and the empty directory removed.
        fs::remove_file(home.join("library.json")).unwrap();
        fs::create_dir(home.join("library.json")).unwrap();
        let attempt = s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root}));
        assert!(attempt.is_err());
        assert!(s.active.is_null());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2, "only Folders and the aborted take remain");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn default_camera_size_is_168_but_stored_legacy_sizes_are_never_changed() {
        for stored in [140, 200, 280, 120, 168, 224] {
            let p = normalize_preferences(&json!({"cameraSize":stored}));
            assert_eq!(p["cameraSize"], stored, "an explicit stored size must survive");
        }
        // Through a real settings file, as an existing install would load it.
        let home = temp_home("legacy-size");
        fs::write(home.join("settings.json"), json!({"root":home.join("media"),"settings":{"cameraSize":200,"countdown":3}}).to_string()).unwrap();
        assert_eq!(Store::new(home.clone()).unwrap().config["settings"]["cameraSize"], 200);
        fs::remove_dir_all(&home).unwrap();
        let fresh = temp_home("fresh-size");
        assert_eq!(Store::new(fresh.clone()).unwrap().config["settings"]["cameraSize"], 168);
        fs::remove_dir_all(&fresh).unwrap();
        // A settings file that predates the key gets the new default; it is not a stored choice.
        assert_eq!(normalize_preferences(&json!({"quality":1280}))["cameraSize"], 168);
    }
    impl Store {
        fn begin_for_test(&mut self, home: &Path, pid: Option<u32>) {
            begin_take(self, home, pid);
        }
    }
    fn begin_take(s: &mut Store, home: &Path, pid: Option<u32>) -> (Value, PathBuf) {
        // The fixture owner is explicit and registered, the way a real native process identifies itself.
        s.pid = pid;
        let r = s.action("native-begin", json!({"ownerPid":pid,"root":home.join("media")})).unwrap();
        let dir = PathBuf::from(r["directory"].as_str().unwrap());
        fs::write(dir.join("recording.mp4"), b"combined media").unwrap();
        fs::write(dir.join("screen.mp4"), b"screen").unwrap();
        fs::write(dir.join("capture.json"), b"{\"partial\":true}").unwrap();
        (r, dir)
    }
    #[test]
    fn a_confirmed_dead_native_releases_the_take_as_interrupted_and_keeps_every_file() {
        let home = temp_home("dead-native");
        let mut s = Store::new(home.clone()).unwrap();
        let (r, dir) = begin_take(&mut s, &home, Some(4242));
        assert_eq!(s.take_owner, Some(4242));
        assert!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":home.join("media")})).is_err(), "blocked while active");
        // Unknown or alive is not a confirmed death: nothing changes.
        for answer in [Liveness::Unknown, Liveness::Alive] {
            assert!(s.reconcile_dead_native(|_| answer).is_none());
            assert_eq!(s.active["id"], r["id"]);
        }
        let stored = s.reconcile_dead_native(|pid| { assert_eq!(pid, 4242); Liveness::Gone }).unwrap().unwrap();
        assert_eq!(stored["status"], "interrupted");
        assert!(stored["error"].as_str().unwrap().contains("closed unexpectedly"));
        assert!(s.active.is_null() && s.take_owner.is_none() && s.pid.is_none());
        let names: Vec<&str> = stored["files"].as_array().unwrap().iter().filter_map(|f| f["name"].as_str()).collect();
        assert_eq!(names, vec!["recording.mp4", "screen.mp4"]);
        assert_eq!(fs::read(dir.join("recording.mp4")).unwrap(), b"combined media");
        assert!(dir.join("capture.json").exists());
        // The library is usable again, and the interrupted take survives a restart.
        assert!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":home.join("media")})).is_ok());
        assert_eq!(Store::new(home.clone()).unwrap().snapshot()["recordings"].as_array().unwrap().iter().filter(|x| x["id"] == r["id"]).count(), 1);
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn a_dead_native_with_a_failing_save_keeps_the_take_active_and_retries_later() {
        let home = temp_home("dead-native-persist");
        let mut s = Store::new(home.clone()).unwrap();
        let (r, dir) = begin_take(&mut s, &home, Some(77));
        fs::remove_file(home.join("library.json")).unwrap();
        fs::create_dir(home.join("library.json")).unwrap();
        assert!(s.reconcile_dead_native(|_| Liveness::Gone).unwrap().is_err());
        assert_eq!(s.active["id"], r["id"], "a failed store deliberately keeps it active");
        assert_eq!(s.take_owner, Some(77));
        assert!(dir.join("recording.mp4").exists());
        fs::remove_dir(home.join("library.json")).unwrap();
        fs::write(home.join("library.json"), json!({"version":1,"folders":[],"recordings":s.index["recordings"]}).to_string()).unwrap();
        assert_eq!(s.reconcile_dead_native(|_| Liveness::Gone).unwrap().unwrap()["status"], "interrupted");
        assert!(s.active.is_null());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn a_take_needs_a_known_owner_and_an_invalid_explicit_owner_cannot_record() {
        let home = temp_home("owner-rules");
        let mut s = Store::new(home.clone()).unwrap();
        let root = home.join("media");
        // Nothing registered and nothing claimed: refused, because its death could never be reconciled.
        let refused = s.action("native-begin", json!({"root":root})).unwrap_err();
        assert!(refused.contains("hasn't registered"), "{refused}");
        for bad in [json!(0), json!(-1), json!(2147483648u64), json!(4294967296u64), json!("12"), json!(1.5), json!(true), json!([1])] {
            assert_eq!(s.action("native-begin", json!({"root":root,"ownerPid":bad.clone()})).unwrap_err(), "Invalid owner process", "{bad}");
        }
        assert!(s.active.is_null() && s.index["recordings"].as_array().unwrap().is_empty(), "nothing was recorded");
        // An older client that never sends it still works through its confirmed registration.
        let live = std::process::id(); // a real, live registered controller
        s.pid = Some(live);
        let legacy = s.action("native-begin", json!({"root":root})).unwrap();
        assert_eq!(s.take_owner, Some(live));
        s.action("native-finish", json!({"id":legacy["id"],"duration":1})).unwrap();
        assert!(s.take_owner.is_none() && s.reconcile_dead_native(|_| Liveness::Gone).is_none(), "a finished take needs no owner");
        // An explicit owner wins over, and does not displace, a live registration.
        let r = s.action("native-begin", json!({"root":root,"ownerPid":888})).unwrap();
        assert_eq!((s.take_owner, s.pid), (Some(888), Some(live)));
        s.action("native-abort", json!({"id":r["id"]})).unwrap();
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn a_registration_that_never_reached_the_host_still_leaves_a_recoverable_owner_and_unblocks_after_death() {
        const OWNER: u32 = 31337;
        let home = temp_home("dropped-registration");
        let mut s = Store::new(home.clone()).unwrap();
        assert_eq!(s.pid, None, "native-window-ready was dropped before the host");
        let (r, dir) = {
            let r = s.action("native-begin", json!({"root":home.join("media"),"ownerPid":OWNER})).unwrap();
            let dir = PathBuf::from(r["directory"].as_str().unwrap());
            fs::write(dir.join("recording.mp4"), b"combined media").unwrap();
            (r, dir)
        };
        assert_eq!((s.take_owner, s.pid), (Some(OWNER), Some(OWNER)), "the take carries its owner, and registration is repaired");
        assert!(s.action("native-begin", json!({"root":home.join("media"),"ownerPid":OWNER})).is_err(), "blocked while active");
        // Alive or unknown never releases it.
        for answer in [Liveness::Alive, Liveness::Unknown] {
            assert!(s.reconcile_dead_native(|pid| { assert_eq!(pid, OWNER); answer }).is_none());
            assert_eq!(s.active["id"], r["id"]);
        }
        // Confirmed death: interrupted, media kept, and recording and Quit are free again.
        let stored = s.reconcile_dead_native(|_| Liveness::Gone).unwrap().unwrap();
        assert_eq!(stored["status"], "interrupted");
        assert_eq!(fs::read(dir.join("recording.mp4")).unwrap(), b"combined media");
        assert!(s.active.is_null());
        assert!(s.action("native-begin", json!({"root":home.join("media"),"ownerPid":std::process::id()})).is_ok());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn registration_never_takes_over_a_live_controller_or_the_owner_of_an_active_take() {
        let home = temp_home("registration-hijack");
        let mut s = Store::new(home.clone()).unwrap();
        s.register_native(100, |_| Liveness::Alive);
        assert_eq!(s.pid, Some(100));
        s.register_native(200, |_| Liveness::Alive);
        s.register_native(300, |_| Liveness::Unknown);
        assert_eq!(s.pid, Some(100), "a live or unknown controller keeps the registration");
        // The process that owns an active take keeps it even if it looks gone.
        let r = s.action("native-begin", json!({"root":home.join("media"),"ownerPid":100})).unwrap();
        s.register_native(200, |_| Liveness::Gone);
        assert_eq!((s.pid, s.take_owner), (Some(100), Some(100)));
        s.action("native-abort", json!({"id":r["id"]})).unwrap();
        // Only a confirmed-gone registration is replaced.
        s.register_native(200, |_| Liveness::Gone);
        assert_eq!(s.pid, Some(200));
        // Same pid again is a no-op; an invalid claimed pid is rejected at the boundary.
        s.register_native(200, |_| panic!("no liveness query for the same pid"));
        assert_eq!(s.action("native-window-ready", json!({"pid":0})).unwrap_err(), "Invalid owner process");
        assert_eq!(s.action("native-window-ready", json!({"pid":-5})).unwrap_err(), "Invalid owner process");
        assert_eq!(s.pid, Some(200));
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn a_command_poll_repairs_a_lost_registration_so_open_studio_stays_one_controller() {
        use std::cell::Cell;
        let (host, home) = test_host();
        let launches = Cell::new(0);
        let launcher = |_: &Path, _: bool| { launches.set(launches.get() + 1); Ok(()) };
        assert_eq!(host.store.lock().unwrap().pid, None, "startup registration was lost");
        host.post_command("show"); // so the long poll returns at once
        host.action("native-command", json!({"pid":std::process::id()})).unwrap();
        assert_eq!(host.store.lock().unwrap().pid, Some(std::process::id()));
        let reply = host.open_studio_with(|_| Liveness::Alive, launcher).unwrap();
        assert_eq!((launches.get(), reply["opened"].as_bool()), (0, Some(true)), "a polling toolbar gets a show, not a second toolbar");
        // A poll with a bad pid is ignored, and one from another process cannot take the registration.
        host.post_command("show");
        host.action("native-command", json!({"pid":0})).unwrap();
        host.post_command("show");
        host.action("native-command", json!({"pid":std::process::id() + 1})).unwrap();
        assert_eq!(host.store.lock().unwrap().pid, Some(std::process::id()));
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn this_process_is_alive_and_a_reaped_child_is_never_reported_alive() {
        assert_eq!(process_liveness(std::process::id()), Liveness::Alive);
        let mut child = Command::new(std::env::current_exe().unwrap()).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert_ne!(process_liveness(pid), Liveness::Alive, "a reaped child is gone or unknowable, never alive");
        // Pids that must never reach a signal call are unknown, not gone: 0 and negative values address groups.
        for pid in [0u32, i32::MAX as u32 + 1, u32::MAX] {
            assert_eq!(process_liveness(pid), Liveness::Unknown, "{pid}");
        }
    }
    #[test]
    fn posix_probe_results_are_classified_conservatively() {
        let called = std::cell::Cell::new(0);
        let probe = |result: Result<(), i32>| move |_: i32| result;
        assert_eq!(classify_posix_probe(4242, probe(Ok(()))), Liveness::Alive);
        assert_eq!(classify_posix_probe(4242, probe(Err(3))), Liveness::Gone, "ESRCH is the only confirmed absence");
        assert_eq!(classify_posix_probe(4242, probe(Err(1))), Liveness::Alive, "EPERM: it exists");
        for errno in [-1, 0, 2, 4, 5, 12, 22, 35, 9999] {
            assert_eq!(classify_posix_probe(4242, probe(Err(errno))), Liveness::Unknown, "errno {errno} cannot confirm a death");
        }
        // The probe never runs for a pid that could mean a process group.
        for pid in [0u32, i32::MAX as u32 + 1, u32::MAX] {
            assert_eq!(classify_posix_probe(pid, |_| { called.set(called.get() + 1); Err(3) }), Liveness::Unknown, "{pid}");
        }
        assert_eq!(called.get(), 0);
        let seen = std::cell::Cell::new(0);
        classify_posix_probe(7, |pid| { seen.set(pid); Ok(()) });
        assert_eq!(seen.get(), 7, "the exact pid is probed");
    }
    #[test]
    fn windows_probe_results_are_classified_conservatively() {
        assert_eq!(classify_windows_probe(4242, WindowsProbe::Opened(Some(259))), Liveness::Alive);
        assert_eq!(classify_windows_probe(4242, WindowsProbe::Opened(Some(0))), Liveness::Gone);
        assert_eq!(classify_windows_probe(4242, WindowsProbe::Opened(Some(1))), Liveness::Gone);
        assert_eq!(classify_windows_probe(4242, WindowsProbe::Opened(None)), Liveness::Unknown, "unreadable exit code");
        assert_eq!(classify_windows_probe(4242, WindowsProbe::OpenFailed(87)), Liveness::Gone, "no such pid");
        assert_eq!(classify_windows_probe(4242, WindowsProbe::OpenFailed(5)), Liveness::Alive, "access denied: it exists");
        for error in [0, 6, 8, 31, 1450, u32::MAX] {
            assert_eq!(classify_windows_probe(4242, WindowsProbe::OpenFailed(error)), Liveness::Unknown, "error {error}");
        }
        assert_eq!(classify_windows_probe(0, WindowsProbe::OpenFailed(87)), Liveness::Unknown);
    }
    #[test]
    fn an_uncertain_liveness_answer_can_never_clear_an_active_take_or_drop_its_owner() {
        let home = temp_home("unknown-liveness");
        let mut s = Store::new(home.clone()).unwrap();
        let (r, dir) = begin_take(&mut s, &home, Some(4242));
        for answer in [Liveness::Alive, Liveness::Unknown] {
            for _ in 0..3 {
                assert!(s.reconcile_dead_native(|_| answer).is_none());
            }
            assert_eq!((s.active["id"].clone(), s.take_owner), (r["id"].clone(), Some(4242)), "{answer:?}");
            assert!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":home.join("media")})).is_err(), "still blocks a new take");
        }
        assert!(dir.join("recording.mp4").exists());
        // Only an explicit, confirmed Gone releases it.
        assert!(s.reconcile_dead_native(|_| Liveness::Gone).unwrap().is_ok());
        assert!(s.active.is_null());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn a_stale_poll_never_launches_a_second_toolbar_while_the_owner_may_be_alive() {
        let alive = |_| Liveness::Alive;
        let unknown = |_| Liveness::Unknown;
        let gone = |_| Liveness::Gone;
        let (stale, fresh, old_launch, recent) = (36_000, 1_000, 60_000, 1_000);
        // Active take: owner alive or unknown + stale poll => busy, for every launch age.
        for l in [old_launch, recent] {
            assert_eq!(studio_plan(Some(7), Some(7), true, stale, l, alive), StudioPlan::Busy);
            assert_eq!(studio_plan(Some(7), Some(7), true, stale, l, unknown), StudioPlan::Busy);
        }
        // Active take whose owner is not recorded: unknown owner, never a launch.
        assert_eq!(studio_plan(None, None, true, stale, old_launch, gone), StudioPlan::Busy);
        assert_eq!(studio_plan(Some(7), None, true, stale, old_launch, alive), StudioPlan::Busy);
        // No take, registered toolbar alive/unknown but wedged: busy as well, not a duplicate.
        assert_eq!(studio_plan(Some(7), None, false, stale, old_launch, alive), StudioPlan::Busy);
        assert_eq!(studio_plan(Some(7), None, false, stale, old_launch, unknown), StudioPlan::Busy);
        // A polling toolbar just gets Show.
        assert_eq!(studio_plan(Some(7), Some(7), true, fresh, old_launch, alive), StudioPlan::Show);
        // Confirmed death, or nothing registered: a launch is legitimate, debounced by the 8 s window.
        assert_eq!(studio_plan(Some(7), Some(7), true, stale, old_launch, gone), StudioPlan::Launch);
        assert_eq!(studio_plan(Some(7), None, false, stale, old_launch, gone), StudioPlan::Launch);
        assert_eq!(studio_plan(None, None, false, stale, old_launch, alive), StudioPlan::Launch);
        assert_eq!(studio_plan(None, None, false, stale, recent, alive), StudioPlan::Opening);
        // No registration at all but a take with an explicit owner: that owner still counts.
        assert_eq!(studio_plan(None, Some(7), true, stale, old_launch, alive), StudioPlan::Busy);
        assert_eq!(studio_plan(None, Some(7), true, stale, old_launch, gone), StudioPlan::Launch);
        // The take owner decides, not a different registered pid.
        let owner_only = |pid: u32| if pid == 9 { Liveness::Alive } else { Liveness::Gone };
        assert_eq!(studio_plan(Some(7), Some(9), true, stale, old_launch, owner_only), StudioPlan::Busy);
    }
    #[test]
    fn open_studio_counts_launches_and_queues_a_passive_show_instead() {
        use std::cell::Cell;
        let (host, home) = test_host();
        let launches = Cell::new(0);
        let launcher = |_: &Path, _: bool| { launches.set(launches.get() + 1); Ok(()) };
        {
            let mut s = host.store.lock().unwrap();
            s.pid = Some(4242);
            s.last_poll = now().saturating_sub(120_000); // stalled for two minutes
            s.begin_for_test(&home, Some(4242));
        }
        for liveness in [Liveness::Alive, Liveness::Unknown] {
            let reply = host.open_studio_with(|_| liveness, launcher).unwrap();
            assert_eq!((reply["busy"].as_bool(), reply["queued"].as_bool(), reply["opened"].as_bool()), (Some(true), Some(true), Some(false)));
            assert_eq!(launches.get(), 0, "{liveness:?}: no second toolbar");
            assert_eq!(host.command.lock().unwrap().as_deref(), Some("show"), "the request waits as a passive show");
            host.command.lock().unwrap().take();
            let s = host.store.lock().unwrap();
            assert_eq!((s.pid, s.take_owner), (Some(4242), Some(4242)), "owner and registration untouched");
            assert!(!s.active.is_null());
        }
        // Confirmed death: one launch, a second request inside the debounce window launches nothing more.
        assert!(host.open_studio_with(|_| Liveness::Gone, launcher).is_ok());
        assert_eq!(launches.get(), 1);
        let again = host.open_studio_with(|_| Liveness::Gone, launcher).unwrap();
        assert_eq!((launches.get(), again["opening"].as_bool()), (1, Some(true)));
        // A failed launch releases the debounce window and reports the error.
        host.store.lock().unwrap().launched_at = 0;
        assert!(host.open_studio_with(|_| Liveness::Gone, |_: &Path, _| Err("no app".into())).is_err());
        assert_eq!(host.store.lock().unwrap().launched_at, 0);
        fs::remove_dir_all(home).unwrap();
    }
    fn attempt_id(tag: &str) -> String {
        format!("{tag}{}", "0".repeat(16))
    }
    #[test]
    fn a_begin_attempt_is_idempotent_resolvable_and_cancellable_without_touching_other_takes() {
        let home = temp_home("attempts");
        let mut s = Store::new(home.clone()).unwrap();
        let root = home.join("media");
        let a = attempt_id("aaaaaaaa");
        // Legacy caller, no attempt: unchanged behaviour. Bad identities are rejected explicitly.
        let legacy = s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root})).unwrap();
        assert!(legacy.get("attempt").is_none());
        s.action("native-abort", json!({"id":legacy["id"]})).unwrap();
        for bad in [json!(5), json!("short"), json!("has space in it, long enough"), json!("x".repeat(65))] {
            assert_eq!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root,"attempt":bad})).unwrap_err(), "Invalid recording attempt");
        }
        // First begin commits; the same attempt again returns the same take instead of "already active".
        let first = s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root,"attempt":a})).unwrap();
        let replay = s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root,"attempt":a})).unwrap();
        assert_eq!((first["id"].clone(), first["attempt"].clone()), (replay["id"].clone(), json!(a)));
        assert_eq!(s.index["recordings"].as_array().unwrap().iter().filter(|r| r["attempt"] == json!(a)).count(), 1, "one take, never two");
        // Resolve says active; another attempt is told cancelled and the active take is untouched.
        let active = s.action("native-begin-resolve", json!({"attempt":a})).unwrap();
        assert_eq!((active["state"].as_str(), active["record"]["id"].clone()), (Some("active"), first["id"].clone()));
        let other = attempt_id("bbbbbbbb");
        assert_eq!(s.action("native-begin-resolve", json!({"attempt":other})).unwrap()["state"], "cancelled");
        assert_eq!(s.active["id"], first["id"], "someone else's take is never aborted");
        assert_eq!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root,"attempt":other})).unwrap_err(), "This recording attempt was cancelled");
        assert_eq!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root,"attempt":attempt_id("cccccccc")})).unwrap_err(), "A recording is already active");
        // After the take ends: resolve says completed, and a late replay of the begin is refused.
        s.action("native-abort", json!({"id":first["id"]})).unwrap();
        assert_eq!(s.action("native-begin-resolve", json!({"attempt":a})).unwrap()["state"], "completed");
        assert_eq!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":root,"attempt":a})).unwrap_err(), "This recording attempt already ended");
        assert!(s.active.is_null());
        // The identity survives a restart (it is stored with the take).
        assert_eq!(Store::new(home.clone()).unwrap().action("native-begin-resolve", json!({"attempt":a})).unwrap()["state"], "completed");
        assert_eq!(s.action("native-begin-resolve", json!({})).unwrap_err(), "Missing recording attempt");
        fs::remove_dir_all(home).unwrap();
    }
    /// A real local HTTP host over a real metadata-only Store. The first native-begin is held for
    /// `hold` (longer than the client's 4 s read timeout), either before or after it commits.
    fn serve_store(store: Arc<Mutex<Store>>, commit_before_hold: bool) -> (String, Arc<tiny_http::Server>) {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let origin = format!("http://127.0.0.1:{}", server.server_addr().to_ip().unwrap().port());
        let background = server.clone();
        let held = Arc::new(std::sync::atomic::AtomicBool::new(false));
        std::thread::spawn(move || {
            for mut request in background.incoming_requests() {
                let (store, held) = (store.clone(), held.clone());
                std::thread::spawn(move || {
                    let mut body = String::new();
                    request.as_reader().read_to_string(&mut body).unwrap();
                    let v: Value = serde_json::from_str(&body).unwrap();
                    let (op, data) = (v["op"].as_str().unwrap().to_owned(), v["data"].clone());
                    let slow = op == "native-begin" && !held.swap(true, std::sync::atomic::Ordering::SeqCst);
                    let mut result = None;
                    if slow && commit_before_hold {
                        result = Some(store.lock().unwrap().action(&op, data.clone()));
                    }
                    if slow {
                        std::thread::sleep(Duration::from_millis(4600));
                    }
                    let result = result.unwrap_or_else(|| store.lock().unwrap().action(&op, data));
                    let (status, value) = match result {
                        Ok(v) => (200, v),
                        Err(e) => (400, json!({"error":e})),
                    };
                    let _ = request.respond(Response::from_string(value.to_string()).with_status_code(status));
                });
            }
        });
        (origin, server)
    }
    #[test]
    fn a_begin_the_host_committed_but_answered_too_late_is_reconciled_and_starts_that_take() {
        let home = temp_home("late-reply-committed");
        let store = Arc::new(Mutex::new(Store::new(home.clone()).unwrap()));
        let (origin, server) = serve_store(store.clone(), true);
        let conn = super::super::connection::Connection::for_test(origin);
        let attempt = super::super::connection::new_attempt();
        let outcome = super::super::connection::begin_take(
            &mut |op, d| conn.action(op, d),
            &attempt,
            json!({"root":home.join("media"),"ownerPid":std::process::id()}),
            &mut |d| std::thread::sleep(d),
        );
        let super::super::connection::BeginOutcome::Started { id, directory } = outcome else { panic!("{outcome:?}") };
        let s = store.lock().unwrap();
        assert_eq!(s.active["id"], id, "the worker has the identity of the take the host holds");
        assert!(directory.unwrap().exists());
        assert_eq!(s.index["recordings"].as_array().unwrap().iter().filter(|r| r["attempt"] == json!(attempt)).count(), 1);
        drop(s);
        server.unblock();
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn a_begin_stuck_in_the_host_when_the_client_gives_up_is_cancelled_and_cannot_commit_afterwards() {
        let home = temp_home("late-reply-uncommitted");
        let store = Arc::new(Mutex::new(Store::new(home.clone()).unwrap()));
        let (origin, server) = serve_store(store.clone(), false);
        let conn = super::super::connection::Connection::for_test(origin);
        let attempt = super::super::connection::new_attempt();
        let outcome = super::super::connection::begin_take(
            &mut |op, d| conn.action(op, d),
            &attempt,
            json!({"root":home.join("media"),"ownerPid":std::process::id()}),
            &mut |d| std::thread::sleep(d),
        );
        assert!(matches!(outcome, super::super::connection::BeginOutcome::Refused(_)), "{outcome:?}");
        // The held request now reaches the store: it must be refused, so no take appears and Record/Quit stay free.
        std::thread::sleep(Duration::from_millis(1500));
        let mut s = store.lock().unwrap();
        assert!(s.active.is_null(), "a cancelled attempt can never commit late");
        assert!(s.index["recordings"].as_array().unwrap().is_empty());
        assert!(s.action("native-begin", json!({"ownerPid":std::process::id(),"root":home.join("media")})).is_ok(), "a new take is not blocked");
        drop(s);
        server.unblock();
        fs::remove_dir_all(home).unwrap();
    }
    fn running_host() -> (Host, PathBuf) {
        let (host, home) = test_host();
        host.action("native-window-ready", json!({"pid":std::process::id()})).unwrap();
        (host, home)
    }
    const LAUNCH: &str = "launch-host-0001";
    fn progress(phase: &str, generation: u64) -> Value {
        json!({"phase":phase,"generation":generation,"launch":LAUNCH,"elapsedMs":1500,"fpsSet":30})
    }
    /// A control as Home makes it: bound to the launch and take it saw, with an expiry and a fresh request number.
    fn control(body: Value, generation: Option<u64>, nonce: &str) -> Value {
        let mut v = body;
        v["launch"] = json!(LAUNCH);
        v["nonce"] = json!(nonce);
        v["expires"] = json!(now() + 4000);
        if let (Some(g), false) = (generation, v.get("generation").is_some()) {
            v["generation"] = json!(g);
        }
        v
    }
    #[test]
    fn capture_controls_are_not_agent_operations_and_validate_every_argument() {
        for op in HUMAN_OPS.iter().chain(["native-progress"].iter()) {
            assert!(!PUBLIC_OPS.contains(op), "{op} must never be an agent operation");
        }
        let schema = tools_list()["tools"][1]["inputSchema"]["properties"]["op"]["enum"].clone();
        assert!(!schema.to_string().contains("capture-command") && !schema.to_string().contains("record"));
        let ctl = |body: Value| control(body, Some(3), "nonce-0001");
        for good in [
            json!({"command":"record"}), json!({"command":"stop"}), json!({"command":"cancel"}),
            json!({"command":"input","kind":"camera","on":true}), json!({"command":"input","kind":"mic","on":false}),
            json!({"command":"source","kind":"display","id":7}), json!({"command":"source","kind":"default"}),
            json!({"command":"device","kind":"camera","uid":"UID-1"}), json!({"command":"device","kind":"mic","uid":null}),
            json!({"command":"shape","value":"portrait"}), json!({"command":"shape","value":"wide"}), json!({"command":"size","value":224}),
            json!({"command":"mirror","value":false}), json!({"command":"fps","value":24}), json!({"command":"capacity-check"}), json!({"command":"dismiss-notice"}), json!({"command":"choose-window"}),
        ] {
            assert!(validate_human(&ctl(good.clone())).is_ok(), "{good}");
        }
        for bad in [
            json!({}), json!({"command":"prepare-recording"}), json!({"command":"delete"}), json!({"command":"input","kind":"screen","on":true}),
            json!({"command":"input","kind":"camera"}), json!({"command":"source","kind":"display"}), json!({"command":"source","kind":"display","id":-1}),
            json!({"command":"source","kind":"display","id":4294967296u64}), json!({"command":"device","kind":"camera","uid":5}), json!({"command":"shape","value":"oval"}),
            json!({"command":"size","value":150}), json!({"command":"fps","value":120}), json!({"command":"stop","generation":"x"}),
            json!({"command":"device","kind":"mic","uid":"bad\u{0}id"}),
        ] {
            assert!(validate_human(&ctl(bad.clone())).is_err(), "{bad}");
        }
        // The session envelope is not optional: a bare request, or one without a take for Record/Stop/Cancel, is refused.
        for bare in [json!({"command":"mirror","value":true}), json!({"command":"record"}), json!({"command":"stop","generation":3})] {
            assert!(validate_human(&bare).is_err(), "{bare}");
        }
        assert!(validate_human(&control(json!({"command":"record"}), None, "nonce-0002")).is_err(), "Record must name the take it saw");
        assert!(validate_human(&control(json!({"command":"mirror","value":true}), None, "nonce-0003")).is_ok(), "a look change needs a session, not a take");
        for (field, value) in [("launch", json!("short")), ("nonce", json!("has space!")), ("expires", json!("soon"))] {
            let mut v = control(json!({"command":"mirror","value":true}), None, "nonce-0004");
            v[field] = value;
            assert!(validate_human(&v).is_err(), "{field}");
        }
    }
    #[test]
    fn a_capture_command_is_bound_to_the_running_launch_the_take_it_saw_an_expiry_and_a_single_use() {
        let (host, home) = test_host();
        let send = |host: &Host, body: Value, g: Option<u64>, nonce: &str| host.action("capture-command", control(body, g, nonce));
        // No native process registered: refused, nothing queued.
        let refused = send(&host, json!({"command":"mirror","value":true}), None, "nonce-a001").unwrap_err();
        assert!(refused.contains("isn't running"), "{refused}");
        assert!(host.human.lock().unwrap().is_empty());
        host.action("native-window-ready", json!({"pid":std::process::id()})).unwrap();
        // Registered but silent: there is no launch to bind to yet.
        assert!(send(&host, json!({"command":"record"}), Some(1), "nonce-a002").unwrap_err().contains("restarted or isn't reporting"));
        host.action("native-progress", progress("idle", 1)).unwrap();
        // A control made for another launch (a request that outlived a restart) is refused.
        let mut other = control(json!({"command":"record"}), Some(1), "nonce-a003");
        other["launch"] = json!("launch-before-0001");
        assert!(host.action("capture-command", other).unwrap_err().contains("restarted or isn't reporting"));
        // A control for an older take is refused; so is one that has expired or claims a long life.
        assert!(send(&host, json!({"command":"record"}), Some(0), "nonce-a004").unwrap_err().contains("already changed"));
        let mut expired = control(json!({"command":"record"}), Some(1), "nonce-a005");
        expired["expires"] = json!(now() - 1);
        assert!(host.action("capture-command", expired).unwrap_err().contains("expired"));
        let mut forever = control(json!({"command":"record"}), Some(1), "nonce-a006");
        forever["expires"] = json!(now() + 3_600_000);
        assert!(host.action("capture-command", forever).unwrap_err().contains("expired"));
        assert!(host.human.lock().unwrap().is_empty(), "none of those queued anything");
        // A silent app is not given work: its last report is too old.
        host.store.lock().unwrap().capture_at = now() - HUMAN_REPORT_MAX_AGE_MS - 1;
        assert!(send(&host, json!({"command":"record"}), Some(1), "nonce-a007").unwrap_err().contains("isn't responding"));
        host.store.lock().unwrap().capture_at = now();
        // The good one queues, once. The same request number again is a replay, even with fresh timestamps.
        let queued = send(&host, json!({"command":"record"}), Some(1), "nonce-good1").unwrap();
        assert_eq!((queued["queued"].as_bool(), queued["ttlMs"].as_u64()), (Some(true), Some(HUMAN_TTL_MS)));
        assert!(send(&host, json!({"command":"record"}), Some(1), "nonce-good1").unwrap_err().contains("already used"));
        // Polling returns it first with its envelope, as a human command; the legacy slot is untouched.
        host.post_command("show");
        let reply = host.action("native-command", json!({"pid":std::process::id()})).unwrap();
        assert_eq!((reply["command"].as_str(), reply["human"]["name"].as_str()), (Some("human"), Some("record")));
        let payload = &reply["human"]["payload"];
        assert_eq!((payload["launch"].as_str(), payload["nonce"].as_str(), payload["generation"].as_u64()), (Some(LAUNCH), Some("nonce-good1"), Some(1)));
        assert!(payload["expires"].as_u64().unwrap() > now() && reply["human"]["issuedAt"].as_u64().unwrap() > 0);
        assert_eq!(host.action("native-command", json!({})).unwrap()["command"], "show");
        // A command older than the time to live, or past its own expiry, is dropped unread.
        send(&host, json!({"command":"mirror","value":true}), None, "nonce-old01").unwrap();
        host.human.lock().unwrap().front_mut().unwrap().issued_at = now() - HUMAN_TTL_MS - 1;
        assert!(host.next_human().is_none() && host.human.lock().unwrap().is_empty());
        send(&host, json!({"command":"mirror","value":true}), None, "nonce-old02").unwrap();
        host.human.lock().unwrap().front_mut().unwrap().payload["expires"] = json!(now() - 1);
        assert!(host.next_human().is_none());
        // A queued command does not survive the native process registering anew.
        send(&host, json!({"command":"record"}), Some(1), "nonce-rest1").unwrap();
        host.action("native-window-ready", json!({"pid":std::process::id()})).unwrap();
        assert!(host.human.lock().unwrap().is_empty(), "no delayed autostart after launch");
        // The relaunched app publishes a new launch (and numbers its takes afresh). A request made for the
        // previous launch and delivered now is not a fresh start, whatever its timestamps say.
        let mut next = progress("idle", 0);
        next["launch"] = json!("launch-host-0002");
        host.action("native-progress", next).unwrap();
        assert_eq!(host.action("state", json!({})).unwrap()["capture"]["launch"], "launch-host-0002");
        let delayed = send(&host, json!({"command":"record"}), Some(0), "nonce-rest2").unwrap_err();
        assert!(delayed.contains("restarted or isn't reporting"), "{delayed}");
        let mut fresh = control(json!({"command":"record"}), Some(0), "nonce-rest3");
        fresh["launch"] = json!("launch-host-0002");
        assert_eq!(host.action("capture-command", fresh).unwrap()["queued"], true, "a control made for the running launch is fine");
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn the_pending_queue_is_bounded() {
        let (host, home) = running_host();
        host.action("native-progress", progress("idle", 1)).unwrap();
        for i in 0..HUMAN_QUEUE_MAX {
            host.action("capture-command", control(json!({"command":"mirror","value":true}), None, &format!("nonce-q{i:04}"))).unwrap();
        }
        assert!(host.action("capture-command", control(json!({"command":"mirror","value":true}), None, "nonce-qover")).unwrap_err().contains("Too many"));
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn the_published_capture_state_is_validated_generation_ordered_stale_aware_and_cleared_with_its_process() {
        let (host, home) = running_host();
        let view = || host.action("state", json!({})).unwrap()["capture"].clone();
        assert_eq!(view()["phase"], "unavailable", "never invented before the app publishes");
        host.action("native-progress", progress("recording", 4)).unwrap();
        assert_eq!((view()["phase"].clone(), view()["generation"].clone(), view()["elapsedMs"].clone()), (json!("recording"), json!(4), json!(1500)));
        // A late message from an older take is dropped.
        let late = host.action("native-progress", progress("recording", 3)).unwrap();
        assert_eq!(late["accepted"], false);
        assert_eq!(view()["generation"], 4);
        // The terminal state of the same generation replaces it, and carries no timer.
        host.action("native-progress", json!({"phase":"idle","generation":4})).unwrap();
        assert!(view().get("elapsedMs").is_none() && view()["phase"] == "idle");
        // Invalid or oversized content is refused and changes nothing.
        for bad in [json!({"phase":"exploding","generation":1}), json!({"phase":"idle"}), json!({"phase":"idle","generation":5,"fpsDelivered":-1}),
                    json!({"phase":"idle","generation":5,"inputs":{"camera":{"mark":"red"}}}), json!({"phase":"idle","generation":5,"notice":{"kind":"x","text":"y".repeat(241)}})] {
            assert!(host.action("native-progress", bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(view()["generation"], 4);
        // Unknown fields are never stored or echoed.
        host.action("native-progress", json!({"phase":"idle","generation":5,"secret":"x","notice":{"kind":"failed","text":"Couldn't keep up.","details":"raw","kept":false,"offer30":true}})).unwrap();
        let v = view();
        assert!(v.get("secret").is_none() && v["notice"]["text"] == "Couldn't keep up.");
        assert_eq!(v["notice"]["kept"], false); assert!(v["notice"].get("offer30").is_none(), "unknown notice fields are not echoed");
        // A silent app shows as stale, never as a running timer.
        host.action("native-progress", progress("recording", 6)).unwrap();
        host.store.lock().unwrap().capture_at = now() - 7000;
        assert_eq!(view()["stale"], true);
        // A process registering anew has no capture; a confirmed-gone one clears it too.
        host.action("native-window-ready", json!({"pid":std::process::id() + 1})).unwrap();
        host.store.lock().unwrap().register_native(std::process::id(), |_| Liveness::Gone);
        assert!(host.store.lock().unwrap().pid.is_some());
        let mut s = host.store.lock().unwrap();
        s.capture = json!({"phase":"recording","generation":1});
        drop(s);
        let (_, dir) = {
            let mut s = host.store.lock().unwrap();
            begin_take(&mut s, &home, Some(4242))
        };
        let _ = dir;
        assert!(host.store.lock().unwrap().reconcile_dead_native(|_| Liveness::Gone).unwrap().is_ok());
        assert_eq!(view()["phase"], "unavailable");
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn the_source_list_and_screen_state_are_bounded_typed_and_never_carry_anything_else() {
        let (host, home) = running_host();
        let sources: Vec<Value> = (0..40).map(|i| json!({"id":i,"kind":"display","name":format!("Display {i}"),"selected":i == 0,"thumbnail":"data:image/png;base64,AAAA"})).collect();
        host.action("native-progress", json!({"phase":"idle","generation":1,"sources":sources,"screenState":"denied"})).unwrap();
        let view = host.action("state", json!({})).unwrap()["capture"].clone();
        assert_eq!(view["sources"].as_array().unwrap().len(), 16, "bounded");
        assert!(view["sources"][0].get("thumbnail").is_none(), "only the known fields are kept");
        assert_eq!((view["sources"][0]["selected"].as_bool(), view["screenState"].as_str()), (Some(true), Some("denied")));
        for bad in [json!({"phase":"idle","generation":2,"screenState":"maybe"}), json!({"phase":"idle","generation":2,"sources":[{"id":-1,"kind":"display"}]}),
                    json!({"phase":"idle","generation":2,"sources":[{"id":1,"kind":"camera"}]})] {
            assert!(host.action("native-progress", bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(host.action("state", json!({})).unwrap()["capture"]["generation"], 1);
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn stop_marks_the_take_stopping_in_home_at_once_and_tells_the_native_app() {
        let (host, home) = running_host();
        let r = host.action("native-begin", json!({"root":home.join("media"),"ownerPid":std::process::id()})).unwrap();
        host.action("native-progress", progress("recording", 2)).unwrap();
        host.action("stop", json!({})).unwrap();
        let view = host.action("state", json!({})).unwrap()["capture"].clone();
        assert_eq!(view["phase"], "stopping", "Home stops counting before the native app confirms");
        let reply = host.action("native-command", json!({})).unwrap();
        let payload = &reply["human"]["payload"];
        assert_eq!((reply["human"]["name"].as_str(), payload["generation"].as_u64(), payload["launch"].as_str()), (Some("stop"), Some(2), Some(LAUNCH)));
        assert!(payload["nonce"].as_str().is_some_and(|n| n.len() >= 8) && payload["expires"].as_u64().unwrap() > now(), "a Stop made by the host carries a fresh envelope");
        // A second Stop does nothing further to the phase and is not queued as a new command by Home.
        let again = host.action("capture-command", control(json!({"command":"stop"}), Some(2), "nonce-stop1")).unwrap();
        assert_eq!(again["queued"], false);
        host.action("native-abort", json!({"id":r["id"]})).unwrap();
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn every_settings_path_and_every_legacy_file_keeps_an_explicit_frame_rate_unpromotable() {
        let (host, home) = test_host();
        let chosen = |h: &Host| h.action("state", json!({})).unwrap()["settings"].clone();
        // A fresh home has not chosen: the only state a qualified default may ever raise.
        assert_eq!((chosen(&host)["fps"].as_u64(), chosen(&host)["fpsChosen"].as_bool()), (Some(30), Some(false)));
        // Home, an agent and the native app all write `fps`; none of them says fpsChosen except the native app.
        for rate in [30u64, 24, 60] {
            host.action("settings", json!({"preferences":{"fpsChosen":false,"fps":30}})).unwrap();
            let after = host.action("settings", json!({"preferences":{"fps":rate}})).unwrap();
            assert_eq!((after["settings"]["fps"].as_u64(), after["settings"]["fpsChosen"].as_bool()), (Some(rate), Some(true)), "a Home write of {rate} is a choice");
        }
        // Unrelated writes leave the flag alone, and an explicit false is honoured when the writer says so.
        host.action("settings", json!({"preferences":{"fpsChosen":false,"fps":30}})).unwrap();
        assert_eq!(host.action("settings", json!({"preferences":{"mirror":false}})).unwrap()["settings"]["fpsChosen"], false);
        assert_eq!(host.action("settings", json!({"preferences":{"fps":30,"fpsChosen":false}})).unwrap()["settings"]["fpsChosen"], false);
        // Legacy files: a stored rate with no flag is read as chosen (30 included); no rate stored means nothing was chosen.
        for rate in [24, 30, 60] {
            assert_eq!(normalize_preferences(&json!({"fps":rate}))["fpsChosen"], true, "{rate}");
        }
        assert_eq!(normalize_preferences(&json!({"fps":30,"fpsChosen":false}))["fpsChosen"], false);
        assert_eq!(normalize_preferences(&json!({"cameraShape":"square"}))["fpsChosen"], false);
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn portrait_is_a_valid_shape_and_the_frame_rate_choice_flag_is_a_boolean() {
        let (host, home) = test_host();
        let ok = host.action("settings", json!({"preferences":{"cameraShape":"portrait","fpsChosen":true,"fps":24}})).unwrap();
        assert_eq!((ok["settings"]["cameraShape"].as_str(), ok["settings"]["fpsChosen"].as_bool(), ok["settings"]["fps"].as_u64()), (Some("portrait"), Some(true), Some(24)));
        assert!(host.action("settings", json!({"preferences":{"fpsChosen":"yes"}})).is_err());
        // Stored owner choices stay exactly as they are.
        for shape in ["rounded", "square", "circle", "wide"] {
            assert!(host.action("settings", json!({"preferences":{"cameraShape":shape}})).is_ok(), "{shape}");
        }
        assert_eq!(normalize_preferences(&json!({"fps":24,"cameraShape":"square"}))["fps"], 24);
        assert_eq!(normalize_preferences(&json!({}))["fpsChosen"], false);
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn reported_version_matches_the_package_version() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert_eq!(VERSION, "0.6.0-preview.11");
    }

    // ---- identity, the page-only tool, media and the one-press start (controlled fixtures only; nothing is recorded) ----
    #[test]
    fn the_server_tools_and_resource_are_named_openrecorder_with_one_black_circle_and_a_versioned_address() {
        let init = rpc(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#);
        let info = &init["result"]["serverInfo"];
        assert_eq!((info["name"].as_str(), info["title"].as_str(), info["version"].as_str()), (Some("OpenRecorder"), Some("OpenRecorder"), Some(VERSION)));
        let icon = info["icons"][0]["src"].as_str().unwrap();
        assert!(icon.starts_with("data:image/svg+xml;base64,"));
        assert_eq!(icon.trim_start_matches("data:image/svg+xml;base64,"), base64(ICON_SVG.as_bytes()));
        assert!(ICON_SVG.contains("<circle") && ICON_SVG.contains("currentColor"), "a plain monochrome circle");
        assert_eq!(URI, format!("ui://openrecorder/home-v{VERSION}"), "the address carries the release so a cached template cannot outlive it");
        let list = tools_list();
        let tools = list["tools"].as_array().unwrap();
        assert_eq!(tools[0]["name"], "openrecorder_home");
        assert_eq!(tools[0]["title"], "OpenRecorder");
        assert_eq!(tools[0]["_meta"]["ui"]["resourceUri"], URI);
        assert_eq!(tools[0]["icons"][0]["src"], icon);
        assert!(!list.to_string().contains("OpenRecorder_Home") && !list.to_string().contains("OpenRecorder Home"), "no fallback name");
        let resources = rpc(r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#);
        let resource = &resources["result"]["resources"][0];
        assert_eq!((resource["uri"].as_str(), resource["title"].as_str(), resource["name"].as_str()), (Some(URI), Some("OpenRecorder"), Some("OpenRecorder")));
        assert_eq!(resource["mimeType"], "text/html;profile=mcp-app");
        // The same release everywhere a person or an agent host could read it.
        for manifest in [include_str!("../../plugin.json"), include_str!("../../.claude-plugin/plugin.json")] {
            let m: Value = serde_json::from_str(manifest).unwrap();
            assert_eq!(m["version"], VERSION);
        }
        assert!(include_str!("../../ui/app.js").contains(&format!("version:'{VERSION}'")), "the page introduces itself with this release");
    }
    #[test]
    fn the_page_only_tool_is_hidden_from_the_model_is_not_an_agent_operation_and_needs_the_session_key() {
        let list = tools_list();
        let tools = list["tools"].as_array().unwrap();
        let ui = tools.iter().find(|t| t["name"] == "openrecorder_ui").unwrap();
        assert_eq!(ui["_meta"]["ui"]["visibility"], json!(["app"]));
        for t in tools.iter().filter(|t| t["name"] != "openrecorder_ui") {
            assert!(t["_meta"]["ui"].get("visibility").is_none(), "the model-visible tools are not app-only: {}", t["name"]);
        }
        assert_eq!(ui["inputSchema"]["properties"]["op"]["enum"], json!(UI_OPS));
        let action_ops = tools.iter().find(|t| t["name"] == "openrecorder_action").unwrap()["inputSchema"]["properties"]["op"]["enum"].to_string();
        for op in UI_OPS {
            assert!(!PUBLIC_OPS.contains(op), "{op} is not an agent operation");
            assert!(!action_ops.contains(op), "{op} is not in the agent-visible tool's list");
        }
        for op in ["record", "toggle", "input", "source", "grant", "device"] {
            assert!(!PUBLIC_OPS.contains(&op) && !action_ops.contains(&format!("\"{op}\"")), "{op}");
        }
        // Without this session's key (which only the app window's own copy of the page carries) nothing runs and
        // the library is never contacted; a wrong op is refused even with the key.
        let mut bridge = Bridge::default();
        let call = |bridge: &mut Bridge, args: Value| bridge.handle_line(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"openrecorder_ui","arguments":args}}).to_string().as_bytes()).unwrap();
        for args in [json!({"op":"start-recording","data":{}}), json!({"op":"start-recording","data":{},"ui":"wrong"}), json!({"op":"media-read","data":{},"ui":""})] {
            let r = call(&mut bridge, args.clone());
            assert_eq!(r["result"]["isError"], true, "{args}");
            assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("belongs to the OpenRecorder window"), "{r}");
        }
        let key = bridge.ui_key.clone();
        let r = call(&mut bridge, json!({"op":"state","data":{},"ui":key}));
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("Not an operation"), "{r}");
        assert!(!bridge.conn.is_some(), "no library was started by any refused call");
        // The key is not in anything the model can list or read about the server.
        for text in [tools_list().to_string(), rpc(r#"{"jsonrpc":"2.0","id":3,"method":"server/discover"}"#).to_string(), rpc(r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#).to_string()] {
            assert!(!text.contains(&key));
        }
        assert_ne!(Bridge::default().ui_key, Bridge::default().ui_key, "each session has its own");
    }
    #[test]
    fn base64_matches_the_rfc_vectors_and_every_served_name_has_a_media_type() {
        for (plain, coded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(plain.as_bytes()), coded);
        }
        assert_eq!(base64(&[0xff, 0xfe, 0xfd, 0x00]), "//79AA==");
        for (name, mime) in [("thumbnail.jpg", "image/jpeg"), ("recording.mp4", "video/mp4"), ("screen.mp4", "video/mp4"), ("recording.webm", "video/webm"), ("camera.mov", "video/quicktime"), ("recording.json", "application/json")] {
            assert_eq!(mime_of(name), mime, "{name}");
        }
    }
    fn debase64(text: &str) -> Vec<u8> {
        let value = |c: u8| b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().position(|x| *x == c).unwrap() as u32;
        let mut out = Vec::new();
        for quad in text.as_bytes().chunks(4) {
            let n = quad.iter().take_while(|c| **c != b'=').fold(0u32, |acc, c| (acc << 6) | value(*c)) << (6 * (4 - quad.iter().take_while(|c| **c != b'=').count() as u32));
            let bytes = quad.iter().take_while(|c| **c != b'=').count() * 6 / 8;
            out.extend_from_slice(&n.to_be_bytes()[1..1 + bytes]);
        }
        out
    }
    /// A finished take with a thumbnail and a recording of `bytes` bytes, in a throw-away library.
    fn host_with_take(bytes: usize) -> (Host, PathBuf, String, Vec<u8>) {
        let (host, home) = test_host();
        let root = home.join("media");
        let begun = host.store.lock().unwrap().action("native-begin", json!({"ownerPid":std::process::id(),"root":root})).unwrap();
        let dir = PathBuf::from(begun["directory"].as_str().unwrap());
        let content: Vec<u8> = (0..bytes).map(|i| (i % 251) as u8).collect();
        fs::write(dir.join("recording.mp4"), &content).unwrap();
        fs::write(dir.join("thumbnail.jpg"), b"\xff\xd8poster\xff\xd9").unwrap();
        host.store.lock().unwrap().action("native-finish", json!({"id":begun["id"],"duration":1})).unwrap();
        (host, home, begun["id"].as_str().unwrap().to_owned(), content)
    }
    #[test]
    fn media_is_read_in_bounded_pieces_from_a_fixed_set_of_files_and_never_past_the_end() {
        let total = MEDIA_CHUNK as usize * 2 + 1234;
        let (host, home, id, content) = host_with_take(total);
        let mut got = Vec::new();
        let mut start = 0u64;
        let mut pieces = Vec::new();
        while start < total as u64 {
            let r = host.action("media-read", json!({"id":id,"name":"recording.mp4","start":start})).unwrap();
            assert_eq!((r["total"].as_u64(), r["start"].as_u64(), r["mime"].as_str()), (Some(total as u64), Some(start), Some("video/mp4")));
            let length = r["length"].as_u64().unwrap();
            assert!(length <= MEDIA_CHUNK && length > 0);
            got.extend(debase64(r["data"].as_str().unwrap()));
            pieces.push(length);
            start += length;
        }
        assert_eq!(pieces, vec![MEDIA_CHUNK, MEDIA_CHUNK, 1234]);
        assert_eq!(got, content, "the pieces are the file, byte for byte");
        // A larger request is clamped; a smaller one is honoured.
        assert_eq!(host.action("media-read", json!({"id":id,"name":"recording.mp4","start":0,"length":u64::MAX})).unwrap()["length"], MEDIA_CHUNK);
        assert_eq!(host.action("media-read", json!({"id":id,"name":"recording.mp4","start":10,"length":5})).unwrap()["length"], 5);
        let poster = host.action("media-read", json!({"id":id,"name":"thumbnail.jpg"})).unwrap();
        assert_eq!((poster["mime"].as_str(), poster["length"].as_u64()), (Some("image/jpeg"), Some(10)));
        // Refusals: past the end, other names (including a path and the library's own files), unknown recordings.
        assert!(host.action("media-read", json!({"id":id,"name":"recording.mp4","start":total})).unwrap_err().contains("Past the end"));
        for name in ["library.json", "../library.json", "/etc/hosts", "recording.mp4/../recording.json.bak", ""] {
            assert!(host.action("media-read", json!({"id":id,"name":name})).is_err(), "{name:?}");
        }
        assert!(host.action("media-read", json!({"id":"nope","name":"recording.mp4"})).is_err());
        assert!(host.action("media-read", json!({"name":"recording.mp4"})).is_err());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn a_fresh_media_address_is_available_to_the_page_and_names_this_library() {
        let (host, home) = test_host();
        let r = host.action("media-session", json!({})).unwrap();
        assert_eq!((r["origin"].as_str(), r["mediaKey"].as_str(), r["version"].as_str()), (Some("http://127.0.0.1:1"), Some(host.media_key.as_str()), Some(VERSION)));
        assert!(!r.to_string().contains(&host.key), "the owner key is never part of it");
        // Browser Home never carries the media key and the app window's page does; neither carries the owner key.
        assert!(!host.markup(false).contains(&host.media_key) && host.markup(true).contains(&host.media_key));
        for embedded in [false, true] {
            assert!(!host.markup(embedded).contains(&host.key));
        }
        // The media headers are a fixed, narrow set: cross-origin reads of the scoped address, nothing else.
        let names: Vec<String> = media_cors().iter().map(|h| h.field.to_string().to_lowercase()).collect();
        assert_eq!(names, ["cross-origin-resource-policy", "access-control-allow-origin", "access-control-expose-headers"]);
        let values: Vec<String> = media_cors().iter().map(|h| h.value.to_string()).collect();
        assert_eq!(values[0], "cross-origin");
        assert!(!values.iter().any(|v| v.contains("Authorization") || v.contains("credentials")));
        fs::remove_dir_all(home).unwrap();
    }
    fn ready_app(host: &Host, phase: &str, generation: u64) {
        host.action("native-window-ready", json!({"pid":std::process::id()})).unwrap();
        host.action("native-progress", progress(phase, generation)).unwrap();
    }
    fn envelope(nonce: &str) -> Value {
        json!({"nonce":nonce,"expires":now() + 4000})
    }
    #[test]
    fn one_record_press_with_a_live_app_is_exactly_one_bound_record_and_never_a_second() {
        let (host, home) = test_host();
        ready_app(&host, "idle", 4);
        let mut press = envelope("press-0001");
        press["launch"] = json!(LAUNCH);
        press["generation"] = json!(4);
        let r = host.action("start-recording", press.clone()).unwrap();
        assert_eq!(r["queued"], true);
        {
            let queue = host.human.lock().unwrap();
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].name, "record");
            assert_eq!((queue[0].payload["launch"].as_str(), queue[0].payload["generation"].as_u64()), (Some(LAUNCH), Some(4)));
        }
        assert!(host.action("start-recording", press).unwrap_err().contains("already used"), "a replay is refused");
        assert_eq!(host.human.lock().unwrap().len(), 1);
        // Another launch or a take Home did not see is refused by the same checks as every control.
        let mut other = envelope("press-0002");
        other["launch"] = json!("launch-before-0001");
        other["generation"] = json!(4);
        assert!(host.action("start-recording", other).is_err());
        let mut old_take = envelope("press-0003");
        old_take["launch"] = json!(LAUNCH);
        old_take["generation"] = json!(3);
        assert!(host.action("start-recording", old_take).is_err());
        assert_eq!(host.human.lock().unwrap().len(), 1);
        assert!(host.pending_start.lock().unwrap().is_none(), "a live app has nothing pending");
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn a_press_while_the_app_is_not_running_waits_briefly_and_starts_one_countdown_when_the_app_reports_idle() {
        let (host, home) = test_host();
        let launched = std::cell::Cell::new(0);
        let start = || { launched.set(launched.get() + 1); Ok(json!({"launched":true})) };
        // No envelope, an expired one and an over-long one are refused and start nothing.
        assert!(host.start_recording_with(&json!({}), start).is_err());
        assert!(host.start_recording_with(&json!({"nonce":"press-1000","expires":now().saturating_sub(1)}), start).is_err());
        assert!(host.start_recording_with(&json!({"nonce":"press-1001","expires":now() + 60_000}), start).is_err());
        assert_eq!(launched.get(), 0);
        let r = host.start_recording_with(&envelope("press-1002"), start).unwrap();
        assert_eq!((r["queued"].as_bool(), r["starting"].as_bool(), launched.get()), (Some(false), Some(true), 1));
        assert!(host.start_recording_with(&envelope("press-1002"), start).unwrap_err().contains("already used"), "a replay does not launch twice");
        assert_eq!(launched.get(), 1);
        // The app comes up and reports idle: exactly one Record, made here, bound to that launch and take.
        host.action("native-window-ready", json!({"pid":std::process::id()})).unwrap();
        host.action("native-progress", progress("unavailable", 0)).unwrap();
        assert!(host.human.lock().unwrap().is_empty() && host.pending_start.lock().unwrap().is_some(), "an app that is not ready yet keeps waiting");
        host.action("native-progress", progress("idle", 2)).unwrap();
        {
            let queue = host.human.lock().unwrap();
            assert_eq!(queue.len(), 1);
            let p = &queue[0].payload;
            assert_eq!((queue[0].name.as_str(), p["launch"].as_str(), p["generation"].as_u64()), ("record", Some(LAUNCH), Some(2)));
            assert!(p["nonce"].as_str().unwrap().len() >= 8 && p["expires"].as_u64().unwrap() <= now() + HUMAN_TTL_MS);
        }
        host.action("native-progress", progress("idle", 2)).unwrap();
        assert_eq!(host.human.lock().unwrap().len(), 1, "delivered once, never again");
        assert!(host.pending_start.lock().unwrap().is_none());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn a_waiting_press_never_starts_over_a_take_and_cancel_stop_expiry_and_quit_all_clear_it() {
        for (clearing, phase) in [("recording", true), ("countdown", true), ("saving", true)] {
            let (host, home) = test_host();
            host.start_recording_with(&envelope("press-2000"), || Ok(json!({}))).unwrap();
            host.action("native-window-ready", json!({"pid":std::process::id()})).unwrap();
            host.action("native-progress", progress(clearing, 5)).unwrap();
            assert!(phase && host.human.lock().unwrap().is_empty(), "{clearing}: no Record is made over a take");
            assert!(host.pending_start.lock().unwrap().is_none(), "{clearing}: the press no longer applies");
            fs::remove_dir_all(home).unwrap();
        }
        // Expired by the clock: the report that arrives afterwards finds nothing to deliver.
        let (host, home) = test_host();
        host.start_recording_with(&envelope("press-2001"), || Ok(json!({}))).unwrap();
        *host.pending_start.lock().unwrap() = Some(now().saturating_sub(1));
        ready_app(&host, "idle", 1);
        assert!(host.human.lock().unwrap().is_empty() && host.pending_start.lock().unwrap().is_none());
        host.action("native-window-closed", json!({})).unwrap();
        // A person's Cancel clears it, and so does Stop.
        host.start_recording_with(&envelope("press-2002"), || Ok(json!({}))).unwrap();
        let _ = host.action("capture-command", control(json!({"command":"cancel"}), Some(1), "press-2003"));
        assert!(host.pending_start.lock().unwrap().is_none(), "Cancel clears a waiting press");
        host.start_recording_with(&envelope("press-2004"), || Ok(json!({}))).unwrap();
        let _ = host.action("stop", json!({}));
        assert!(host.pending_start.lock().unwrap().is_none(), "Stop clears a waiting press");
        // A launch that fails leaves nothing waiting; a quitting library takes no press.
        assert!(host.start_recording_with(&envelope("press-2005"), || Err("no app".into())).is_err());
        assert!(host.pending_start.lock().unwrap().is_none());
        host.quit_requested.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(host.start_recording_with(&envelope("press-2006"), || Ok(json!({}))).is_err());
        assert!(host.pending_start.lock().unwrap().is_none());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn open_library_opens_only_a_fixed_set_of_pages() {
        assert_eq!(library_route(&Value::Null), Ok(""));
        assert_eq!(library_route(&json!("")), Ok(""));
        assert_eq!(library_route(&json!("library")), Ok(""));
        assert_eq!(library_route(&json!("settings")), Ok("/settings"));
        assert_eq!(library_route(&json!("last")), Ok("/last"));
        for bad in [json!("/settings"), json!("settings/../x"), json!("https://example.com"), json!("Settings"), json!(3), json!({"a":1})] {
            assert!(library_route(&bad).is_err(), "{bad}");
        }
        assert!(PUBLIC_OPS.contains(&"open-library"), "opening the browser is an ordinary agent operation and returns no access link");
    }
    #[test]
    fn the_app_reports_names_the_one_status_vocabulary_and_whether_files_were_kept_all_bounded() {
        let mut report = progress("idle", 8);
        report["inputs"] = json!({"camera":{"mark":"armed","name":"FaceTime HD Camera"},"microphone":{"mark":"not-connected","name":"USB Mic"}});
        report["devices"] = json!({"camera":[{"uid":"CAM-A","name":"FaceTime HD Camera","selected":true},{"uid":"CAM-B","name":"Logitech C920","selected":false}],"microphone":[{"uid":"MIC-A","name":"USB Mic","selected":false}]});
        report["notice"] = json!({"kind":"failed","text":"Recording stopped. Originals are kept.","details":"raw","generation":8,"kept":true});
        let clean = sanitize_capture(&report).unwrap();
        assert_eq!(clean["inputs"]["microphone"]["mark"], "not-connected");
        assert_eq!(clean["devices"]["camera"][1], json!({"uid":"CAM-B","name":"Logitech C920","selected":false}));
        assert_eq!(clean["notice"]["kept"], true);
        for mark in INPUT_MARKS {
            let mut r = progress("idle", 1);
            r["inputs"] = json!({"camera":{"mark":mark}});
            assert!(sanitize_capture(&r).is_ok(), "{mark}");
        }
        for bad in [json!({"camera":{"mark":"Remembered"}}), json!({"camera":{"mark":"on"}}), json!({"camera":{}})] {
            let mut r = progress("idle", 1);
            r["inputs"] = bad.clone();
            assert!(sanitize_capture(&r).is_err(), "{bad}");
        }
        // At most sixteen devices are kept; a device must have an id and a name; both are bounded and clean.
        let many: Vec<Value> = (0..40).map(|i| json!({"uid":format!("U{i}"),"name":format!("Camera {i}"),"selected":false})).collect();
        let mut r = progress("idle", 1);
        r["devices"] = json!({"camera":many});
        assert_eq!(sanitize_capture(&r).unwrap()["devices"]["camera"].as_array().unwrap().len(), 16);
        for bad in [json!({"uid":"","name":"x"}), json!({"uid":"U","name":""}), json!({"uid":"U"}), json!({"uid":"U","name":"x".repeat(81)}), json!({"uid":"u".repeat(513),"name":"x"}), json!({"uid":"U\u{0}","name":"x"})] {
            let mut r = progress("idle", 1);
            r["devices"] = json!({"camera":[bad.clone()]});
            assert!(sanitize_capture(&r).is_err(), "{bad}");
        }
        // Unknown fields (a path, a credential) are not stored.
        let mut r = progress("idle", 1);
        r["devices"] = json!({"camera":[{"uid":"U","name":"x","selected":true,"path":"/Users/x","token":"t"}],"screen":[{"uid":"S","name":"y"}]});
        let kept = sanitize_capture(&r).unwrap();
        assert!(kept["devices"]["camera"][0].get("path").is_none() && kept["devices"]["camera"][0].get("token").is_none() && kept["devices"].get("screen").is_none());
    }
    #[test]
    fn the_applied_camera_look_is_validated_bounded_and_kept_only_when_complete() {
        let look = |a: Value| { let mut r = progress("recording", 3); r["appearance"] = a; r };
        let good = json!({"shape":"portrait","size":224,"mirror":false,"showCamera":false});
        assert_eq!(sanitize_capture(&look(good.clone())).unwrap()["appearance"], good);
        for shape in CAMERA_SHAPES {
            assert!(sanitize_capture(&look(json!({"shape":shape,"size":168,"mirror":true,"showCamera":true}))).is_ok(), "{shape}");
        }
        for size in CAMERA_SIZES {
            assert!(sanitize_capture(&look(json!({"shape":"circle","size":size,"mirror":true,"showCamera":true}))).is_ok(), "{size}");
        }
        for bad in [json!("x"), json!(null), json!([]), json!({}), json!({"shape":"oval","size":168,"mirror":true,"showCamera":true}), json!({"shape":"wide","size":150,"mirror":true,"showCamera":true}),
            json!({"shape":"wide","size":"168","mirror":true,"showCamera":true}), json!({"shape":"wide","size":168,"mirror":1,"showCamera":true}), json!({"shape":"wide","size":168,"mirror":true}), json!({"shape":"wide","mirror":true,"showCamera":true})] {
            assert!(sanitize_capture(&look(bad.clone())).is_err(), "{bad}");
        }
        // Unknown fields inside it are not stored; a report without one (an older app) stays valid and has none.
        let mut extra = good.clone();
        extra["path"] = json!("/Users/x");
        assert_eq!(sanitize_capture(&look(extra)).unwrap()["appearance"], good);
        assert!(sanitize_capture(&progress("idle", 1)).unwrap().get("appearance").is_none());
    }
    #[test]
    fn show_camera_and_the_privacy_page_are_validated_human_controls_and_not_agent_operations() {
        let ctl = |body: Value| control(body, None, "nonce-5001");
        assert!(validate_human(&ctl(json!({"command":"show-camera","value":false}))).is_ok());
        assert!(validate_human(&ctl(json!({"command":"show-camera","value":"no"}))).is_err());
        assert!(validate_human(&ctl(json!({"command":"show-camera"}))).is_err());
        for kind in ["screen", "camera", "mic"] {
            assert!(validate_human(&ctl(json!({"command":"open-privacy","kind":kind}))).is_ok(), "{kind}");
        }
        for kind in [json!("files"), json!("microphone"), json!(null), json!(7)] {
            assert!(validate_human(&ctl(json!({"command":"open-privacy","kind":kind}))).is_err(), "{kind}");
        }
        for command in ["show-camera", "open-privacy"] {
            assert!(HUMAN_COMMANDS.contains(&command) && !PUBLIC_OPS.contains(&command));
        }
    }
    #[test]
    fn a_stopped_take_keeps_what_the_app_reported_about_its_files_and_nothing_else() {
        let (host, home) = test_host();
        let root = home.join("media");
        let begun = host.store.lock().unwrap().action("native-begin", json!({"ownerPid":std::process::id(),"root":root})).unwrap();
        let dir = PathBuf::from(begun["directory"].as_str().unwrap());
        fs::write(dir.join("screen.mp4"), b"abc").unwrap();
        let done = host.store.lock().unwrap().action("native-abort", json!({"id":begun["id"],"error":"Recorder stopped unexpectedly.","finalized":false,
            "capture":{"fps":60,"width":2560,"height":1440,"outputFps":56.8,"finalization":"not_finished","encoders":{"screen":"h264_videotoolbox"},"secret":"x","path":"/Users/x"}})).unwrap();
        assert_eq!((done["status"].as_str(), done["finalized"].as_bool()), (Some("interrupted"), Some(false)));
        assert_eq!(done["capture"]["fps"], 60);
        assert!(done["capture"].get("secret").is_none() && done["capture"].get("path").is_none(), "only the known figures are kept");
        assert!(done.get("duration").is_some_and(|d| d == &json!(0)), "no duration was reported, none is made up");
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn show_camera_is_a_saved_boolean_and_nothing_else() {
        let (host, home) = test_host();
        let saved = host.action("settings", json!({"preferences":{"showCamera":false}})).unwrap();
        assert_eq!(saved["settings"]["showCamera"], false);
        assert_eq!(host.action("state", json!({})).unwrap()["settings"]["showCamera"], false);
        for bad in [json!("yes"), json!(1), json!(null)] {
            assert!(host.action("settings", json!({"preferences":{"showCamera":bad.clone()}})).is_err(), "{bad}");
        }
        assert_eq!(host.action("state", json!({})).unwrap()["settings"]["showCamera"], false, "a refused write changed nothing");
        fs::remove_dir_all(home).unwrap();
    }

}
