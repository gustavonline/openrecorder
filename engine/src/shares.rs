//! Temporary guest listeners contain no owner API routes. Secrets live only in memory.
//!
//! Lifecycle: `start` reserves a slot in a few microseconds and returns a `starting` entry; a worker thread
//! launches the tunnel with the manager lock released and publishes `ready` (or `failed`) only if the entry still
//! exists and was not cancelled. Revoke, expiry, archive and quit remove the entry and raise its cancel flag, so a
//! pending worker can never publish late. Torn-down links are dropped after the lock is released.
#[path = "share_access.rs"]
mod share_access;
use serde_json::{json, Value};
use share_access::{Access, GuestServer, Media};
use std::{
    collections::HashMap,
    io::Read,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const MAX_ACTIVE: usize = 4;
const START_TIMEOUT: Duration = Duration::from_secs(30);
const FAILED_VISIBLE_MS: u64 = 60_000;

// An empty private cloudflared configuration, so the user's own config and credentials never apply.
struct IsolatedConfig(PathBuf);
impl IsolatedConfig {
    fn new() -> std::io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("openrecorder-tunnel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        std::fs::write(dir.join("config.yml"), b"")?;
        Ok(Self(dir))
    }
    fn file(&self) -> PathBuf {
        self.0.join("config.yml")
    }
}
impl Drop for IsolatedConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct ChildGuard(Child, Option<IsolatedConfig>);
impl ChildGuard {
    fn finish(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.finish();
    }
}
/// Quick Tunnel command: fresh loopback port only, private empty config, no inherited `TUNNEL_*` settings.
fn tunnel_command(exe: &Path, port: u16, config: &Path) -> Command {
    let mut command = Command::new(exe);
    command
        .arg("tunnel")
        .arg("--config")
        .arg(config)
        .args(["--no-autoupdate", "--protocol", "http2", "--url", &format!("http://127.0.0.1:{port}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().to_ascii_uppercase().starts_with("TUNNEL_") {
            command.env_remove(name);
        }
    }
    command
}
struct Link {
    id: String,
    url: String,
    access: Arc<Access>,
    child: Arc<Mutex<ChildGuard>>,
    monitor: Option<JoinHandle<()>>,
    _server: GuestServer,
}
impl Drop for Link {
    fn drop(&mut self) {
        self.access.stop();
        // Reap even if readiness, the listener, or the tunnel has already failed.
        let mut child = self.child.lock().unwrap();
        child.finish();
        drop(child);
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
    }
}
enum State {
    Starting,
    Ready(Link),
    Failed { error: String, at: u64 },
}
struct Entry {
    recording: String,
    minutes: u64,
    cancel: Arc<AtomicBool>,
    state: State,
}
/// Entries removed from the manager. Drop them only after the manager lock has been released.
pub struct Retired(#[allow(dead_code)] Vec<Entry>);
#[derive(Default)]
pub struct Shares {
    entries: HashMap<String, Entry>,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// UUID v4 uses the OS random source. Only bytes before its version/variant bits
// are used: 6 + 6 + 4 independent bytes give a full 128 random bits.
fn secret() -> String {
    let mut out = String::with_capacity(32);
    for count in [6, 6, 4] {
        let uuid = uuid::Uuid::new_v4();
        for b in &uuid.as_bytes()[..count] {
            use std::fmt::Write;
            let _ = write!(out, "{b:02X}");
        }
    }
    out
}
// 256-bit viewer session secret, generated independently of the 128-bit URL token. Only fully random UUID v4 bytes
// are used (0-5 and 9-15; bytes 6-8 carry the version/variant bits): 13 bytes from each of three UUIDs, first 32 kept.
fn session_secret() -> String {
    let mut bytes = Vec::with_capacity(39);
    for _ in 0..3 {
        let uuid = uuid::Uuid::new_v4();
        bytes.extend_from_slice(&uuid.as_bytes()[..6]);
        bytes.extend_from_slice(&uuid.as_bytes()[9..]);
    }
    bytes[..32].iter().map(|b| format!("{b:02X}")).collect()
}
// Eight symbols of a 32-letter alphabet without 0/O/1/I (40 bits). It is only a second factor for a short-lived,
// rate-limited link; the URL token and the session are the independent 128-bit secrets.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
fn access_code() -> String {
    let mut bytes = Vec::with_capacity(8);
    bytes.extend_from_slice(&uuid::Uuid::new_v4().as_bytes()[..6]);
    bytes.extend_from_slice(&uuid::Uuid::new_v4().as_bytes()[..2]);
    bytes.iter().map(|b| CODE_ALPHABET[(b & 31) as usize] as char).collect()
}
/// The end of a link: `minutes` from now, or never (0). Never means no deadline while the host and tunnel run; the link still
/// ends on revoke, archive, quit, host exit or when the tunnel drops.
const NEVER: u64 = u64::MAX;
fn expiry(minutes: u64) -> u64 {
    if minutes == 0 { NEVER } else { now() + minutes * 60000 }
}
fn display_code(code: &str) -> String {
    format!("{}-{}", &code[..4], &code[4..])
}
fn tunnel_origin(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let url = line[start..]
        .split(|c: char| !c.is_ascii_alphanumeric() && ![':', '/', '.', '-'].contains(&c))
        .next()?;
    let name = url
        .strip_prefix("https://")?
        .strip_suffix(".trycloudflare.com")?;
    if name.is_empty()
        || name.len() > 63
        || name.eq_ignore_ascii_case("api")
        || name.starts_with('-')
        || name.ends_with('-')
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return None;
    }
    Some(url.to_owned())
}
// Waits for the tunnel to register, polling so that cancellation, tunnel exit and the deadline are each noticed
// within about 100 ms. Any failure reaps the child.
fn wait_ready(
    child: &mut ChildGuard,
    rx: &std::sync::mpsc::Receiver<String>,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let result = loop {
        if cancel.load(Ordering::Acquire) {
            break Err("Sharing was cancelled".to_owned());
        }
        match rx.recv_timeout(Duration::from_millis(100).min(timeout)) {
            Ok(origin) => {
                break match child.0.try_wait() {
                    Ok(None) => Ok(origin),
                    _ => Err("Temporary tunnel exited".into()),
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break Err("Temporary tunnel exited".into()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !matches!(child.0.try_wait(), Ok(None)) {
                    break Err("Temporary tunnel exited".into());
                }
                if Instant::now() >= deadline {
                    break Err("Temporary tunnel could not connect".into());
                }
            }
        }
    };
    if result.is_err() {
        child.finish();
    }
    result
}
fn monitor_tunnel(
    child: Arc<Mutex<ChildGuard>>,
    access: Arc<Access>,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("share-tunnel".into())
        .spawn(move || {
            while access.live() {
                let running = matches!(child.lock().unwrap().0.try_wait(), Ok(None));
                if !running {
                    access.stop();
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            child.lock().unwrap().finish();
        })
}
impl Shares {
    /// Drop-safe expiry sweep: the returned entries must be dropped outside the lock.
    fn sweep(&mut self) -> Retired {
        let stale: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| match &e.state {
                State::Starting => false,
                State::Ready(l) => !l.access.live(),
                State::Failed { at, .. } => now().saturating_sub(*at) > FAILED_VISIBLE_MS,
            })
            .map(|(k, _)| k.clone())
            .collect();
        Retired(self.remove(stale))
    }
    fn remove(&mut self, keys: Vec<String>) -> Vec<Entry> {
        keys.iter()
            .filter_map(|k| self.entries.remove(k))
            .inspect(|e| e.cancel.store(true, Ordering::Release))
            .collect()
    }
    fn active(&self) -> usize {
        self.entries.values().filter(|e| matches!(e.state, State::Starting | State::Ready(_))).count()
    }
    fn item(share: &str, e: &Entry) -> Value {
        let mut v = json!({"shareId":share,"id":e.recording,"minutes":e.minutes,"protected":true});
        match &e.state {
            State::Starting => v["status"] = json!("starting"),
            State::Ready(l) => {
                v["status"] = json!("ready");
                v["url"] = json!(l.url);
                v["accessCode"] = json!(display_code(&l.access.code));
                // No deadline is stated as such, never as a far-off time.
                if l.access.expires == NEVER {
                    v["never"] = json!(true);
                } else {
                    v["expiresAt"] = json!(l.access.expires);
                }
            }
            State::Failed { error, .. } => {
                v["status"] = json!("failed");
                v["error"] = json!(error);
            }
        }
        v
    }
    fn snapshot(&self) -> Value {
        let mut items: Vec<_> = self.entries.iter().map(|(k, e)| Self::item(k, e)).collect();
        items.sort_by_key(|v| v["shareId"].as_str().unwrap_or("").to_owned());
        json!(items)
    }
    #[cfg(test)]
    fn insert_ready(&mut self, link: Link, minutes: u64) -> String {
        let share = uuid::Uuid::new_v4().to_string();
        self.entries.insert(
            share.clone(),
            Entry { recording: link.id.clone(), minutes, cancel: Arc::new(AtomicBool::new(false)), state: State::Ready(link) },
        );
        share
    }
}
/// Current shares: `starting`, `ready` (url + accessCode + expiresAt) or `failed` (error), each with `shareId`,
/// recording `id` and `minutes`. Expired links are swept first.
pub fn list(shares: &Arc<Mutex<Shares>>) -> Value {
    let (value, retired) = {
        let mut guard = shares.lock().unwrap();
        let retired = guard.sweep();
        (guard.snapshot(), retired)
    };
    drop(retired);
    value
}
/// Cancel and remove every share of one recording (all of them, or only `share` when given).
pub fn revoke(shares: &Arc<Mutex<Shares>>, recording: &str, share: Option<&str>) {
    let retired = {
        let mut guard = shares.lock().unwrap();
        let keys = guard
            .entries
            .iter()
            .filter(|(k, e)| e.recording == recording && share.is_none_or(|s| s == k.as_str()))
            .map(|(k, _)| k.clone())
            .collect();
        Retired(guard.remove(keys))
    };
    drop(retired);
}
/// Cancel and remove everything (Quit).
pub fn clear(shares: &Arc<Mutex<Shares>>) {
    let retired = {
        let mut guard = shares.lock().unwrap();
        let keys = guard.entries.keys().cloned().collect();
        Retired(guard.remove(keys))
    };
    drop(retired);
}
/// Reserve a share and start its tunnel in the background. Returns at once with the `starting` entry. The caller
/// must hold whatever lock establishes that the recording may be shared, so that archive/quit cannot slip in
/// between the check and the reservation; the tunnel itself is started with every lock released.
pub fn start(shares: &Arc<Mutex<Shares>>, exe: PathBuf, recording: String, file: &Path, minutes: u64) -> Result<Value, String> {
    if ![0, 15, 60, 480].contains(&minutes) {
        return Err("Choose a valid link duration".into());
    }
    // The selected combined file is pinned now; the guest listener never opens a path.
    let media = Media::open(file).map_err(|_| "The recording file is unavailable".to_owned())?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let share = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    let (item, retired) = {
        let mut guard = shares.lock().unwrap();
        let retired = guard.sweep();
        if guard.active() >= MAX_ACTIVE {
            drop(guard);
            drop(retired);
            return Err("Stop an existing share before creating another".into());
        }
        guard.entries.insert(
            share.clone(),
            Entry { recording: recording.clone(), minutes, cancel: cancel.clone(), state: State::Starting },
        );
        (Shares::item(&share, &guard.entries[&share]), retired)
    };
    drop(retired);
    let handle = shares.clone();
    let worker_share = share.clone();
    let spawned = std::thread::Builder::new().name("share-start".into()).spawn(move || {
        let outcome = launch(&exe, listener, media, recording, minutes, &cancel);
        let mut late = None;
        {
            let mut guard = handle.lock().unwrap();
            match guard.entries.get_mut(&worker_share) {
                Some(entry) if !cancel.load(Ordering::Acquire) && matches!(entry.state, State::Starting) => match outcome {
                    Ok(link) => entry.state = State::Ready(link),
                    Err(error) => entry.state = State::Failed { error, at: now() },
                },
                // Revoked, expired or quit while starting: never publish; tear the link down off-lock.
                _ => late = outcome.ok(),
            }
        }
        drop(late);
    });
    if spawned.is_err() {
        let retired = shares.lock().unwrap().remove(vec![share]);
        drop(retired);
        return Err("Could not start sharing".into());
    }
    Ok(item)
}
// Runs entirely without the manager lock.
fn launch(exe: &Path, listener: TcpListener, media: Media, recording: String, minutes: u64, cancel: &AtomicBool) -> Result<Link, String> {
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let config = IsolatedConfig::new().map_err(|e| e.to_string())?;
    let mut child = ChildGuard(
        tunnel_command(exe, port, &config.file())
            .spawn()
            .map_err(|_| "Install cloudflared to create a temporary link".to_string())?,
        Some(config),
    );
    let mut stderr = child.0.stderr.take().ok_or("No tunnel output")?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        // Drain output without logging it or allocating unbounded lines.
        let mut buf = [0; 1024];
        let mut line = Vec::with_capacity(4096);
        let mut overflow = false;
        let mut origin = None;
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            for &b in &buf[..n] {
                if b == b'\n' {
                    if !overflow {
                        let text = String::from_utf8_lossy(&line);
                        if let Some(u) = tunnel_origin(&text) {
                            origin = Some(u);
                        }
                        if text.contains("Registered tunnel connection") {
                            if let Some(u) = origin.take() {
                                let _ = tx.try_send(u);
                            }
                        }
                    }
                    line.clear();
                    overflow = false;
                } else if line.len() < 4096 {
                    line.push(b);
                } else {
                    overflow = true;
                }
            }
        }
    });
    let origin = wait_ready(&mut child, &rx, START_TIMEOUT, cancel)?;
    let token = secret();
    let access = Arc::new(Access::new(token.clone(), access_code(), origin.clone(), expiry(minutes)));
    let server = GuestServer::start(listener, media, access.clone()).map_err(|e| e.to_string())?;
    let child = Arc::new(Mutex::new(child));
    let monitor = monitor_tunnel(child.clone(), access.clone()).map_err(|e| e.to_string())?;
    Ok(Link { id: recording, url: format!("{origin}/{token}"), access, child, monitor: Some(monitor), _server: server })
}
pub fn managed() -> Arc<Mutex<Shares>> {
    let s = Arc::new(Mutex::new(Shares::default()));
    let weak = Arc::downgrade(&s);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(15));
        let Some(s) = weak.upgrade() else { break };
        let retired = s.lock().unwrap().sweep();
        drop(retired);
    });
    s
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    // A local script standing in for cloudflared. It only prints, sleeps and records its own pid; no network.
    struct Fake {
        dir: PathBuf,
        exe: PathBuf,
        media: PathBuf,
    }
    impl Fake {
        fn new(body: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("share-fake-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).unwrap();
            let exe = dir.join("fake-cloudflared");
            std::fs::write(&exe, format!("#!/bin/sh\necho $$ > '{}/pid'\n{body}\n", dir.display())).unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
            let media = dir.join("recording.mp4");
            std::fs::write(&media, b"0123456789").unwrap();
            Self { dir, exe, media }
        }
        fn start(&self, shares: &Arc<Mutex<Shares>>, recording: &str) -> Result<Value, String> {
            start(shares, self.exe.clone(), recording.into(), &self.media, 15)
        }
        fn pid(&self) -> Option<u32> {
            std::fs::read_to_string(self.dir.join("pid")).ok()?.trim().parse().ok()
        }
        fn alive(&self) -> bool {
            self.pid().is_some_and(|p| Command::new("kill").args(["-0", &p.to_string()]).stderr(Stdio::null()).status().is_ok_and(|s| s.success()))
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            if let Some(p) = self.pid() {
                let _ = Command::new("kill").arg(p.to_string()).stderr(Stdio::null()).status();
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
    const READY: &str = "echo 'https://fake-test.trycloudflare.com' >&2; echo 'Registered tunnel connection' >&2; exec sleep 30";
    fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(8);
        while !condition() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn status(shares: &Arc<Mutex<Shares>>) -> Vec<String> {
        list(shares).as_array().unwrap().iter().map(|v| v["status"].as_str().unwrap().to_owned()).collect()
    }
    #[test]
    fn start_returns_at_once_and_the_manager_stays_responsive_while_the_tunnel_starts() {
        let fake = Fake::new(&format!("sleep 2\n{READY}"));
        let shares = managed();
        let began = Instant::now();
        let item = fake.start(&shares, "take").unwrap();
        assert!(began.elapsed() < Duration::from_millis(500), "start blocked for {:?}", began.elapsed());
        assert_eq!(item["status"], "starting");
        assert!(item["url"].is_null() && item["accessCode"].is_null() && item["expiresAt"].is_null());
        assert_eq!(item["id"], "take");
        // Listing, revoking another recording and clearing are not queued behind the 30-second readiness wait.
        let probe = Instant::now();
        assert_eq!(status(&shares), ["starting"]);
        revoke(&shares, "other", None);
        assert!(probe.elapsed() < Duration::from_millis(200), "{:?}", probe.elapsed());
        wait_for("ready", || status(&shares) == ["ready"]);
        let ready = list(&shares)[0].clone();
        assert!(ready["url"].as_str().unwrap().starts_with("https://fake-test.trycloudflare.com/"));
        assert_eq!(ready["accessCode"].as_str().unwrap().len(), 9);
        assert!(ready["expiresAt"].as_u64().unwrap() > now());
        assert_eq!(ready["minutes"], 15);
        revoke(&shares, "take", None);
        assert!(list(&shares).as_array().unwrap().is_empty());
        wait_for("tunnel child reaped", || !fake.alive());
    }
    #[test]
    fn a_link_with_no_deadline_says_so_lasts_while_the_host_runs_and_ends_on_revoke_or_a_dropped_tunnel() {
        // Durations: Never (0) is valid beside the finite ones; anything else is refused before anything starts.
        let fake = Fake::new(READY);
        let shares = managed();
        for bad in [1u64, 5, 14, 61, 481, 1440, u64::MAX] {
            assert_eq!(start(&shares, fake.exe.clone(), "take".into(), &fake.media, bad).unwrap_err(), "Choose a valid link duration", "{bad}");
        }
        assert!(list(&shares).as_array().unwrap().is_empty() && fake.pid().is_none(), "a refused duration starts nothing");
        let item = start(&shares, fake.exe.clone(), "take".into(), &fake.media, 0).unwrap();
        assert_eq!((item["status"].as_str(), item["minutes"].as_u64()), (Some("starting"), Some(0)));
        wait_for("ready", || status(&shares) == ["ready"]);
        let ready = list(&shares)[0].clone();
        // Stated as "never", not as a far-off time; the code and link are as for any share.
        assert_eq!(ready["never"], true);
        assert!(ready.get("expiresAt").is_none(), "no deadline is invented: {ready}");
        assert_eq!(ready["accessCode"].as_str().unwrap().len(), 9);
        // A finite link keeps its exact end and no `never`.
        let finite = Fake::new(READY);
        let other = start(&shares, finite.exe.clone(), "second".into(), &finite.media, 60).unwrap();
        assert_eq!(other["minutes"], 60);
        wait_for("second ready", || status(&shares) == ["ready", "ready"]);
        let listed = list(&shares);
        let second = listed.as_array().unwrap().iter().find(|v| v["id"] == "second").unwrap();
        let end = second["expiresAt"].as_u64().unwrap();
        assert!(end > now() + 59 * 60_000 && end <= now() + 60 * 60_000 && second.get("never").is_none());
        // Still ready after a sweep; revoking ends it and reaps its tunnel.
        assert_eq!(list(&shares).as_array().unwrap().iter().filter(|v| v["status"] == "ready").count(), 2);
        revoke(&shares, "take", None);
        wait_for("never link gone", || list(&shares).as_array().unwrap().iter().all(|v| v["id"] != "take"));
        wait_for("its tunnel reaped", || !fake.alive());
        revoke(&shares, "second", None);
        // A link with no deadline is not permanent: when its tunnel drops (offline, sleep) it ends.
        let dropping = Fake::new("echo 'https://fake-test.trycloudflare.com' >&2; echo 'Registered tunnel connection' >&2; sleep 1; exit 0");
        start(&shares, dropping.exe.clone(), "dropped".into(), &dropping.media, 0).unwrap();
        wait_for("dropping link ready", || status(&shares) == ["ready"]);
        wait_for("link ended with its tunnel", || list(&shares).as_array().unwrap().is_empty());
    }
    #[test]
    fn revoke_while_starting_never_publishes_late_and_reaps_the_child() {
        let fake = Fake::new(&format!("sleep 1\n{READY}"));
        let shares = managed();
        fake.start(&shares, "take").unwrap();
        wait_for("child started", || fake.pid().is_some());
        revoke(&shares, "take", None);
        assert!(list(&shares).as_array().unwrap().is_empty());
        wait_for("child killed", || !fake.alive());
        // Past the moment the fake would have registered: still nothing published, capacity free.
        std::thread::sleep(Duration::from_millis(1500));
        assert!(list(&shares).as_array().unwrap().is_empty());
        assert_eq!(shares.lock().unwrap().active(), 0);
    }
    #[test]
    fn quit_and_archive_style_clear_cancel_pending_starts() {
        let fake = Fake::new(&format!("sleep 1\n{READY}"));
        let shares = managed();
        fake.start(&shares, "take").unwrap();
        wait_for("child started", || fake.pid().is_some());
        clear(&shares);
        wait_for("child killed", || !fake.alive());
        std::thread::sleep(Duration::from_millis(1300));
        assert!(list(&shares).as_array().unwrap().is_empty());
    }
    #[test]
    fn failed_start_is_reported_and_frees_capacity() {
        let fake = Fake::new("echo 'boom' >&2; exit 3");
        let shares = managed();
        fake.start(&shares, "take").unwrap();
        wait_for("failed", || status(&shares) == ["failed"]);
        let failed = list(&shares)[0].clone();
        assert!(failed["error"].as_str().unwrap().contains("tunnel"));
        assert!(failed["url"].is_null());
        assert_eq!(shares.lock().unwrap().active(), 0);
        // A failed entry never blocks new work.
        let again = Fake::new(&format!("sleep 20\n{READY}"));
        again.start(&shares, "other").unwrap();
        assert_eq!(shares.lock().unwrap().active(), 1);
        clear(&shares);
    }
    #[test]
    fn at_most_four_shares_are_active_or_starting() {
        let fake = Fake::new("exec sleep 20");
        let shares = managed();
        for _ in 0..MAX_ACTIVE {
            fake.start(&shares, "take").unwrap();
        }
        assert!(fake.start(&shares, "take").unwrap_err().contains("Stop an existing share"));
        assert!(start(&shares, fake.exe.clone(), "take".into(), &fake.media, 7).unwrap_err().contains("duration"));
        clear(&shares);
        assert_eq!(shares.lock().unwrap().active(), 0);
    }
    #[test]
    fn tunnel_is_isolated_from_user_config_and_tunnel_environment() {
        let var = format!("TUNNEL_TEST_{}", uuid::Uuid::new_v4().simple());
        std::env::set_var(&var, "x");
        let config = IsolatedConfig::new().unwrap();
        let command = tunnel_command(Path::new("/x/cloudflared"), 4321, &config.file());
        let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args[0], "tunnel");
        assert_eq!(args[1], "--config");
        assert_eq!(args[2], config.file().to_string_lossy());
        assert_eq!(std::fs::read(config.file()).unwrap(), b"");
        assert!(args.windows(2).any(|w| w == ["--url", "http://127.0.0.1:4321"]));
        assert!(args.contains(&"--no-autoupdate".to_owned()));
        assert!(command.get_envs().any(|(k, v)| k == var.as_str() && v.is_none()), "TUNNEL_* must be removed");
        let dir = config.0.clone();
        drop(config);
        assert!(!dir.exists());
        std::env::remove_var(&var);
    }
}
