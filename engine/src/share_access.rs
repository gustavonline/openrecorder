//! Bounded, connection-close HTTP/1 guest service. No filesystem lookup precedes authentication.
use super::{now, secret, session_secret};
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
const WORKERS: usize = 8;
const MEDIA_SLOTS: usize = 3;
const MAX_HEAD: usize = 8192;
const MAX_BODY: usize = 1024;
const MAX_WRONG_PER_MINUTE: usize = 5;
const MAX_SESSIONS: usize = 32;
// No script, no inline style: the one stylesheet is a same-origin route under the token, and the only image allowed is the
// data: brand disc. The page needs nothing else.
const CSP: &str = "default-src 'none'; style-src 'self'; img-src data:; media-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";
// `same-origin`, not `no-referrer`: with `no-referrer` a browser sends `Origin: null` on the unlock form's own POST (Fetch
// standard, "append a request Origin header": a non-cors request whose referrer policy is no-referrer serializes its origin
// as null), which the strict same-origin check on /unlock must refuse. `same-origin` sends the real origin to this origin
// and nothing at all anywhere else; the guest pages link nowhere else.
/// How long a viewer session lives on the server and in its cookie: the link's own end, or a week, whichever comes first. A
/// link with no deadline therefore still lets its sessions expire (and their capacity come back) while the link stays live.
const SESSION_MS: u64 = 7 * 24 * 3600 * 1000;
fn session_deadline(link_expires: u64, at: u64) -> u64 {
    link_expires.min(at.saturating_add(SESSION_MS))
}
const REFERRER_POLICY: &str = "same-origin";
// Test builds only: the browser fixture (`tests/share-browser-fixture.py`) can serve the guest pages with the preview.10
// policy so a real browser reproduces the failure it caused. Not compiled into the product.
#[cfg(test)]
static REFERRER_OVERRIDE: Mutex<Option<&'static str>> = Mutex::new(None);

// ---- The guest pages. Same micro/glass language as Home (neutral surfaces, system font, a black disc, restrained borders,
// a deliberate focus ring, no orange), no script, and nothing but the token's own routes.
const STYLE: &str = r#"*{box-sizing:border-box}
:root{color-scheme:light dark;--window:#f5f5f7;--surface:#fff;--control:#f0f0f2;--hairline:#d8d8de;--text:#1d1d1f;--text2:#6e6e73;--accent:#0a6cdf;--primary:#1d1d1f;--on-primary:#fff}
@media(prefers-color-scheme:dark){:root{--window:#161618;--surface:#1e1e21;--control:#2a2a2e;--hairline:#3a3a40;--text:#f2f2f4;--text2:#a1a1a8;--accent:#4c9aff;--primary:#f2f2f4;--on-primary:#111113}}
html{background:var(--window)}
body{margin:0;min-height:100vh;display:flex;flex-direction:column;align-items:center;padding:32px 16px;font:15px/1.4 Inter,ui-sans-serif,system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;color:var(--text);background:var(--window)}
.brand{display:flex;align-items:center;gap:8px;margin:0 0 20px;font-weight:600;font-size:14px;color:var(--text)}
.disc{width:16px;height:16px;border-radius:50%;background:#000;border:1px solid var(--hairline)}
main{width:100%;max-width:440px;padding:24px;border:1px solid var(--hairline);border-radius:12px;background:var(--surface)}
main.wide{max-width:960px;padding:16px}
h1{margin:0 0 8px;font-size:20px;line-height:26px;font-weight:700}
p{margin:0 0 16px;color:var(--text2)}
label{display:block;margin:0 0 6px;font-size:13px;font-weight:600;color:var(--text)}
input{width:100%;height:40px;padding:0 12px;border:1px solid var(--hairline);border-radius:8px;background:var(--surface);color:var(--text);font:inherit;font-size:18px;letter-spacing:.12em;text-transform:uppercase}
input::placeholder{color:var(--text2);opacity:.6;letter-spacing:.12em}
button,.button{display:inline-flex;align-items:center;justify-content:center;height:36px;padding:0 16px;border:1px solid transparent;border-radius:8px;font:inherit;font-weight:700;text-decoration:none;cursor:pointer}
button{width:100%;margin-top:12px;background:var(--primary);color:var(--on-primary)}
.button{background:var(--control);color:var(--text);border-color:var(--hairline);font-weight:600}
button:focus-visible,input:focus-visible,.button:focus-visible,video:focus-visible,a:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
.notice{display:flex;gap:8px;align-items:flex-start;margin:0 0 16px;padding:10px 12px;border:1px solid var(--hairline);border-radius:10px;background:var(--control);color:var(--text)}
.notice i{flex:none;width:16px;height:16px;border-radius:50%;background:var(--text);color:var(--surface);font:700 11px/16px Inter,system-ui,sans-serif;text-align:center}
.hint{margin:8px 0 0;font-size:13px}
video{display:block;width:100%;max-height:75vh;border-radius:10px;background:#000}
.actions{display:flex;justify-content:flex-end;margin-top:12px}
@media(max-width:480px){body{padding:16px 12px}main{padding:16px}.actions{justify-content:stretch}.actions .button{width:100%}}"#;
fn page(root: &str, title: &str, wide: bool, body: &str) -> String {
    format!("<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><meta name=color-scheme content='light dark'><title>{title}</title><link rel=icon href=\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Ccircle cx='8' cy='8' r='5.5'/%3E%3C/svg%3E\"><link rel=stylesheet href='{root}/style.css'></head><body><div class=brand><span class=disc></span>OpenRecorder</div><main{}>{body}</main></body></html>", if wide { " class=wide" } else { "" })
}
// The unlock form, with an optional message (role=alert, announced). The same form is shown again after a refusal: the page
// a browser lands on after a failed POST is never a dead end.
fn unlock_page(root: &str, problem: Option<&str>) -> String {
    let alert = problem.map(|m| format!("<div class=notice role=alert id=problem><i aria-hidden=true>!</i><span>{m}</span></div>")).unwrap_or_default();
    page(root, "Unlock recording", false, &format!("<h1>Shared recording</h1><p>Enter the access code you were given to watch this recording.</p>{alert}<form method=post action='{root}/unlock'><label for=code>Access code</label><input id=code name=code type=text inputmode=text autocomplete=off autocapitalize=characters spellcheck=false required maxlength=40 autofocus placeholder='ABCD-EFGH' aria-describedby='hint{}'><p class=hint id=hint>Letters and numbers; dashes and spaces are fine.</p><button type=submit>Unlock</button></form>", if problem.is_some() { " problem" } else { "" }))
}
fn player_page(root: &str) -> String {
    page(root, "Shared recording", true, &format!("<video controls playsinline preload=metadata src='{root}/video'></video><div class=actions><a class=button href='{root}/download' download>Download recording</a></div>"))
}
fn ended_page(root: &str) -> String {
    page(root, "Link ended", false, "<h1>This link has ended</h1><p>The person who shared this recording stopped sharing it, or the link expired. Ask them for a new link if you still need it.</p>")
}
fn session_ended_page(root: &str) -> String {
    page(root, "Access ended", false, &format!("<h1>Access has ended</h1><p>Your access to this recording ended. Open the link again and enter the access code to keep watching.</p><div class=actions><a class=button href='{root}'>Enter the code again</a></div>"))
}
#[derive(Clone)]
pub(super) struct Media {
    file: Arc<Mutex<File>>,
    webm: bool,
}
impl Media {
    // The authenticated owner selects this file once, before publishing a grant.
    // Keep its descriptor: replacing or moving the path cannot switch a live share
    // to an original or another recording. Guest handlers never open a path.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Not a recording",
            ));
        }
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            webm: path.extension().is_some_and(|e| e == "webm"),
        })
    }
}
struct Auth {
    sessions: HashMap<String, u64>,
    attempts: VecDeque<Instant>,
}
pub(super) struct Access {
    pub token: String,
    pub code: String,
    pub expires: u64,
    origin: String,
    stopped: AtomicBool,
    auth: Mutex<Auth>,
    media: AtomicUsize,
}
impl Access {
    pub fn new(token: String, code: String, origin: String, expires: u64) -> Self {
        Self {
            token,
            code,
            origin,
            expires,
            stopped: AtomicBool::new(false),
            auth: Mutex::new(Auth {
                sessions: HashMap::new(),
                attempts: VecDeque::with_capacity(5),
            }),
            media: AtomicUsize::new(0),
        }
    }
    pub fn live(&self) -> bool {
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        if now() >= self.expires {
            self.stop();
            return false;
        }
        true
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.auth.lock().unwrap().sessions.clear();
    }
    fn session(&self, cookie: Option<&str>) -> Option<String> {
        if !self.live() {
            return None;
        }
        let mut found = None;
        for part in cookie?.split(';') {
            let Some((name, value)) = part.trim().split_once('=') else {
                continue;
            };
            if name == "__Secure-share" {
                if found.is_some()
                    || value.len() != 64
                    || !value.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return None;
                }
                found = Some(value);
            }
        }
        let value = found?;
        let mut auth = self.auth.lock().unwrap();
        auth.sessions.retain(|_, expires| *expires > now());
        auth.sessions.contains_key(value).then(|| value.to_owned())
    }
    fn session_live(&self, session: &str) -> bool {
        self.live()
            && self
                .auth
                .lock()
                .unwrap()
                .sessions
                .get(session)
                .is_some_and(|e| *e > now())
    }
    // Wrong guesses are limited globally to MAX_WRONG_PER_MINUTE per rolling minute. A slot is reserved before the
    // comparison (so concurrent guesses cannot exceed the bound) and handed back when the code was right.
    fn prune(auth: &mut Auth) {
        let current = Instant::now();
        while auth.attempts.front().is_some_and(|t| current.duration_since(*t) >= Duration::from_secs(60)) {
            auth.attempts.pop_front();
        }
    }
    fn locked(&self) -> bool {
        let mut auth = self.auth.lock().unwrap();
        Self::prune(&mut auth);
        auth.attempts.len() >= MAX_WRONG_PER_MINUTE
    }
    fn reserve_attempt(&self) -> Option<Instant> {
        let mut auth = self.auth.lock().unwrap();
        Self::prune(&mut auth);
        if auth.attempts.len() >= MAX_WRONG_PER_MINUTE {
            return None;
        }
        let at = Instant::now();
        auth.attempts.push_back(at);
        Some(at)
    }
    /// On success: the session secret and the instant it ends (also the cookie's lifetime).
    fn unlock(&self, candidate: &str) -> Result<(String, u64), Unlock> {
        let slot = self.reserve_attempt().ok_or(Unlock::Limited)?;
        // Compared case-insensitively without separators; fixed work over every code byte, never early-exit equality.
        let candidate = normalize_code(candidate);
        let mut difference = candidate.len() ^ self.code.len();
        for (i, byte) in self.code.bytes().enumerate() {
            difference |= (std::hint::black_box(byte) ^ std::hint::black_box(candidate.get(i).copied().unwrap_or(0))) as usize;
        }
        if std::hint::black_box(difference) != 0 || !self.live() {
            return Err(Unlock::Refused);
        }
        let mut auth = self.auth.lock().unwrap();
        // A correct unlock is not a wrong attempt: give the slot back.
        if let Some(i) = auth.attempts.iter().position(|t| *t == slot) {
            auth.attempts.remove(i);
        }
        auth.sessions.retain(|_, expires| *expires > now());
        if self.stopped.load(Ordering::Acquire) || now() >= self.expires {
            return Err(Unlock::Refused);
        }
        if auth.sessions.len() >= MAX_SESSIONS {
            return Err(Unlock::Full);
        }
        // A session never outlives its link (nor a week), and is not cut short before that: long playback is not interrupted.
        let session = session_secret();
        let ends = session_deadline(self.expires, now());
        auth.sessions.insert(session.clone(), ends);
        Ok((session, ends))
    }
}
/// What a person typed or pasted, reduced to the code's own letters and digits: case, any Unicode whitespace (including the
/// no-break space and the newline a copied line can end with), the display dash and its look-alikes (hyphens, dashes, the
/// soft and the no-break hyphen, the minus sign), and zero-width characters are not part of the code. Nothing else is
/// forgiven: any other character stays and makes the comparison fail. Bounded: at most 64 characters are looked at.
fn normalize_code(input: &str) -> Vec<u8> {
    fn ignorable(c: char) -> bool {
        c.is_whitespace()
            || matches!(c, '-' | '\u{00AD}' | '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{FE58}' | '\u{FE63}' | '\u{FF0D}' | '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}')
    }
    let mut out = Vec::with_capacity(16);
    let mut buf = [0u8; 4];
    for c in input.chars().take(64).filter(|c| !ignorable(*c)) {
        out.extend_from_slice(c.to_ascii_uppercase().encode_utf8(&mut buf).as_bytes());
        if out.len() >= 16 {
            break;
        }
    }
    out
}
#[derive(Debug)]
pub(super) enum Unlock {
    Limited,
    Refused,
    Full,
}
pub(super) struct GuestServer {
    access: Arc<Access>,
    workers: Vec<JoinHandle<()>>,
}
impl GuestServer {
    pub fn start(listener: TcpListener, file: Media, access: Arc<Access>) -> io::Result<Self> {
        listener.set_nonblocking(true)?;
        let listener = Arc::new(listener);
        let mut server = Self {
            access,
            workers: Vec::new(),
        };
        for _ in 0..WORKERS {
            let listener = listener.clone();
            let access = server.access.clone();
            let file = file.clone();
            server.workers.push(
                std::thread::Builder::new()
                    .name("share-guest".into())
                    .spawn(move || {
                        while access.live() {
                            match listener.accept() {
                                Ok((mut stream, _)) => {
                                    if stream.set_nonblocking(false).is_err()
                                        || stream
                                            .set_read_timeout(Some(Duration::from_millis(500)))
                                            .is_err()
                                        || stream
                                            .set_write_timeout(Some(Duration::from_secs(1)))
                                            .is_err()
                                    {
                                        continue;
                                    }
                                    let _ = serve(&mut stream, &access, &file);
                                }
                                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                                    std::thread::sleep(Duration::from_millis(10))
                                }
                                Err(_) => {
                                    access.stop();
                                    break;
                                }
                            }
                        }
                    })?,
            );
        }
        Ok(server)
    }
}
impl Drop for GuestServer {
    fn drop(&mut self) {
        self.access.stop();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
}
impl Request {
    fn get(&self, key: &str) -> Option<&str> {
        self.headers.get(key).map(String::as_str)
    }
    fn head(&self) -> bool {
        self.method == "HEAD"
    }
}
fn request(reader: &mut impl Read) -> Result<Request, u16> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut data = Vec::with_capacity(MAX_HEAD);
    while !data.ends_with(b"\r\n\r\n") {
        if data.len() == MAX_HEAD {
            return Err(431);
        }
        if Instant::now() > deadline {
            return Err(408);
        }
        let mut byte = [0];
        reader.read_exact(&mut byte).map_err(|_| 400u16)?;
        data.push(byte[0]);
    }
    let text = std::str::from_utf8(&data).map_err(|_| 400u16)?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next().ok_or(400u16)?.split(' ');
    let method = first.next().ok_or(400u16)?.to_owned();
    let path = first.next().ok_or(400u16)?.to_owned();
    if !matches!(first.next(), Some("HTTP/1.1") | Some("HTTP/1.0")) || first.next().is_some() {
        return Err(400);
    }
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (key, value) = line.split_once(':').ok_or(400u16)?;
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || !value.bytes().all(|b| b == b'\t' || (32..127).contains(&b))
        {
            return Err(400);
        }
        if headers
            .insert(key.to_ascii_lowercase(), value.trim().to_owned())
            .is_some()
        {
            return Err(400);
        }
    }
    if headers.contains_key("transfer-encoding") || headers.contains_key("expect") {
        return Err(400);
    }
    Ok(Request {
        method,
        path,
        headers,
    })
}
fn response_head(
    w: &mut impl Write,
    status: u16,
    len: u64,
    kind: &str,
    extra: &[(&str, String)],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        303 => "See Other",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    };
    #[cfg(test)]
    let policy = REFERRER_OVERRIDE.lock().unwrap().unwrap_or(REFERRER_POLICY);
    #[cfg(not(test))]
    let policy = REFERRER_POLICY;
    write!(w, "HTTP/1.1 {status} {reason}\r\nContent-Length: {len}\r\nContent-Type: {kind}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: {policy}\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: {CSP}\r\n")?;
    for (name, value) in extra {
        write!(w, "{name}: {value}\r\n")?;
    }
    w.write_all(b"\r\n")
}
fn reply(
    w: &mut impl Write,
    req: Option<&Request>,
    status: u16,
    body: &str,
    extra: &[(&str, String)],
) -> io::Result<()> {
    response_head(
        w,
        status,
        body.len() as u64,
        "text/html; charset=utf-8",
        extra,
    )?;
    if !req.is_some_and(Request::head) {
        w.write_all(body.as_bytes())?;
    }
    Ok(())
}
fn form_value(value: &str) -> Option<String> {
    let mut out = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(b) = bytes.next() {
        out.push(match b {
            b'+' => b' ',
            b'%' => {
                let a = (bytes.next()? as char).to_digit(16)?;
                let b = (bytes.next()? as char).to_digit(16)?;
                (a * 16 + b) as u8
            }
            b => b,
        });
    }
    String::from_utf8(out).ok()
}
fn code_from_body(kind: Option<&str>, body: &[u8]) -> Result<String, u16> {
    match kind.unwrap_or("").split(';').next().unwrap_or("").trim() {
        "application/json" => {
            let value: serde_json::Value = serde_json::from_slice(body).map_err(|_| 400u16)?;
            let obj = value.as_object().ok_or(400u16)?;
            if obj.len() != 1 {
                return Err(400);
            }
            obj.get("code")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .ok_or(400)
        }
        "application/x-www-form-urlencoded" => {
            let text = std::str::from_utf8(body).map_err(|_| 400u16)?;
            let (key, value) = text.split_once('=').ok_or(400u16)?;
            if key != "code" || value.contains('&') {
                return Err(400);
            }
            form_value(value).ok_or(400)
        }
        _ => Err(415),
    }
}
fn range(value: Option<&str>, total: u64) -> Result<(u64, u64, u16), ()> {
    let Some(value) = value else {
        return Ok((0, total, 200));
    };
    let (a, b) = value
        .strip_prefix("bytes=")
        .and_then(|v| v.split_once('-'))
        .ok_or(())?;
    fn number(v: &str) -> Result<u64, ()> {
        if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
            return Err(());
        }
        v.parse().map_err(|_| ())
    }
    if total == 0 {
        return Err(());
    }
    let (start, end) = if a.is_empty() {
        let suffix = number(b)?;
        if suffix == 0 {
            return Err(());
        }
        (total.saturating_sub(suffix), total - 1)
    } else {
        let start = number(a)?;
        let end = if b.is_empty() {
            total - 1
        } else {
            number(b)?.min(total - 1)
        };
        if start >= total || end < start {
            return Err(());
        }
        (start, end)
    };
    Ok((start, end - start + 1, 206))
}
struct MediaPermit<'a>(&'a AtomicUsize);
impl Drop for MediaPermit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct CheckedReader {
    file: Arc<Mutex<File>>,
    position: u64,
    access: Arc<Access>,
    session: String,
    remaining: u64,
}
impl Read for CheckedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.access.session_live(&self.session) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Share ended",
            ));
        }
        let len = buf.len().min(self.remaining.min(65536) as usize);
        let mut file = self.file.lock().unwrap();
        if !self.access.session_live(&self.session) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Share ended",
            ));
        }
        file.seek(SeekFrom::Start(self.position))?;
        let count = file.read(&mut buf[..len])?;
        self.position += count as u64;
        self.remaining -= count as u64;
        Ok(count)
    }
}
fn serve(stream: &mut TcpStream, access: &Arc<Access>, file: &Media) -> io::Result<()> {
    let mut reader = BufReader::with_capacity(2048, stream.try_clone()?);
    serve_request(&mut reader, stream, access, file)
}
fn serve_request(
    reader: &mut impl Read,
    stream: &mut impl Write,
    access: &Arc<Access>,
    file: &Media,
) -> io::Result<()> {
    let req = match request(reader) {
        Ok(req) => req,
        Err(status) => return reply(stream, None, status, "", &[]),
    };
    if !access.live() {
        // The link's own page says it has ended; every other path, and every method but GET/HEAD, gets nothing.
        let root = format!("/{}", access.token);
        if (req.method == "GET" || req.method == "HEAD") && (req.path == root || req.path == format!("{root}/player")) {
            return reply(stream, Some(&req), 404, &ended_page(&root), &[]);
        }
        return reply(stream, Some(&req), 404, "", &[]);
    }
    // Exact raw-path matching rejects query credentials, escapes, traversal, owner paths,
    // absolute URLs, and original filenames without ever decoding a filesystem path.
    let root = format!("/{}", access.token);
    let route = if req.path == root {
        "root"
    } else if req.path == format!("{root}/unlock") {
        "unlock"
    } else if req.path == format!("{root}/player") {
        "player"
    } else if req.path == format!("{root}/video") {
        "video"
    } else if req.path == format!("{root}/download") {
        "download"
    } else if req.path == format!("{root}/style.css") {
        "style"
    } else {
        return reply(stream, Some(&req), 404, "", &[]);
    };
    let host = access.origin.strip_prefix("https://").unwrap_or("");
    if !req
        .get("host")
        .is_some_and(|h| h.eq_ignore_ascii_case(host))
        || req.get("origin").is_some_and(|o| o != access.origin)
        || (route != "root"
            && req
                .get("referer")
                .is_some_and(|r| !r.starts_with(&format!("{}/", access.origin))))
    {
        return reply(stream, Some(&req), 403, "", &[]);
    }
    if route == "unlock" {
        if req.method != "POST" {
            return reply(stream, Some(&req), 405, "", &[]);
        }
        if req.get("origin") != Some(access.origin.as_str())
            || req
                .get("sec-fetch-site")
                .is_some_and(|v| v != "same-origin")
        {
            return reply(stream, Some(&req), 403, "", &[]);
        }
        if access.locked() {
            return reply(
                stream,
                Some(&req),
                429,
                &unlock_page(&root, Some("Too many wrong codes. Try again in a minute.")),
                &[("Retry-After", "60".into())],
            );
        }
        let raw_len = match req.get("content-length") {
            Some(v) if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) => v,
            _ => return reply(stream, Some(&req), 400, "", &[]),
        };
        let len = match raw_len.parse::<usize>() {
            Ok(len) if len <= MAX_BODY => len,
            _ => return reply(stream, Some(&req), 413, "", &[]),
        };
        let mut body = vec![0; len];
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut offset = 0;
        while offset < len {
            if Instant::now() >= deadline {
                return reply(stream, Some(&req), 408, "", &[]);
            }
            match reader.read(&mut body[offset..]) {
                Ok(0) | Err(_) => return reply(stream, Some(&req), 400, "", &[]),
                Ok(n) => offset += n,
            }
        }
        let code = match code_from_body(req.get("content-type"), &body) {
            Ok(c) => c,
            Err(s) => return reply(stream, Some(&req), s, "", &[]),
        };
        let (session, session_ends) = match access.unlock(&code) {
            Ok(unlocked) => unlocked,
            Err(Unlock::Limited) => {
                return reply(stream, Some(&req), 429, &unlock_page(&root, Some("Too many wrong codes. Try again in a minute.")), &[("Retry-After", "60".into())])
            }
            Err(Unlock::Full) => return reply(stream, Some(&req), 503, &unlock_page(&root, Some("This link is busy right now. Try again in a moment.")), &[("Retry-After", "5".into())]),
            Err(Unlock::Refused) => return reply(stream, Some(&req), 403, &unlock_page(&root, Some("That code didn’t work. Check it and try again.")), &[]),
        };
        // The cookie lives exactly as long as the server's session.
        let max_age = (session_ends.saturating_sub(now()) / 1000).max(1);
        return reply(stream, Some(&req), 303, "", &[
            ("Set-Cookie", format!("__Secure-share={session}; Path={root}/; Max-Age={max_age}; HttpOnly; Secure; SameSite=Strict")),
            ("Location", format!("{root}/player")),
        ]);
    }
    if req.method != "GET" && req.method != "HEAD" {
        return reply(stream, Some(&req), 405, "", &[]);
    }
    if route == "root" {
        return reply(stream, Some(&req), 200, &unlock_page(&root, None), &[]);
    }
    if route == "style" {
        let css = STYLE.as_bytes();
        response_head(stream, 200, css.len() as u64, "text/css; charset=utf-8", &[])?;
        if !req.head() {
            stream.write_all(css)?;
        }
        return Ok(());
    }
    if req
        .get("sec-fetch-site")
        .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return reply(stream, Some(&req), 403, "", &[]);
    }
    let Some(session) = access.session(req.get("cookie")) else {
        // Only the page a person is looking at explains; media requests get nothing.
        let body = if route == "player" { session_ended_page(&root) } else { String::new() };
        return reply(stream, Some(&req), 403, &body, &[]);
    };
    if route == "player" {
        return reply(stream, Some(&req), 200, &player_page(&root), &[]);
    }
    if access
        .media
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < MEDIA_SLOTS).then_some(n + 1)
        })
        .is_err()
    {
        return reply(stream, Some(&req), 503, "", &[("Retry-After", "1".into())]);
    }
    let _permit = MediaPermit(&access.media);
    if !access.session_live(&session) {
        return reply(stream, Some(&req), 404, "", &[]);
    }
    // This is the first guest filesystem access; authorization already succeeded.
    let total = match file.file.lock().unwrap().metadata() {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        _ => return reply(stream, Some(&req), 404, "", &[]),
    };
    // Range is defined for GET only. HEAD describes the complete representation.
    let (start, len, status) = match range(if req.head() { None } else { req.get("range") }, total)
    {
        Ok(r) => r,
        Err(_) => {
            return reply(
                stream,
                Some(&req),
                416,
                "",
                &[
                    ("Content-Range", format!("bytes */{total}")),
                    ("Accept-Ranges", "bytes".into()),
                ],
            )
        }
    };
    if !access.session_live(&session) {
        return reply(stream, Some(&req), 404, "", &[]);
    }
    let mut extra = vec![("Accept-Ranges", "bytes".into())];
    if status == 206 {
        extra.push((
            "Content-Range",
            format!("bytes {start}-{}/{total}", start + len - 1),
        ));
    }
    if route == "download" {
        extra.push((
            "Content-Disposition",
            if file.webm {
                "attachment; filename=recording.webm".into()
            } else {
                "attachment; filename=recording.mp4".into()
            },
        ));
    }
    let kind = if file.webm { "video/webm" } else { "video/mp4" };
    response_head(stream, status, len, kind, &extra)?;
    if !req.head() {
        let mut checked = CheckedReader {
            file: file.file.clone(),
            position: start,
            access: access.clone(),
            session,
            remaining: len,
        };
        let mut chunk = [0; 65536];
        loop {
            let n = checked.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            stream.write_all(&chunk[..n])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct Fixture {
        access: Arc<Access>,
        file: PathBuf,
        media: Media,
    }
    struct Reply {
        status: u16,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            Self::with_expiry(now() + 60000)
        }
        fn with_expiry(expires: u64) -> Self {
            let file =
                std::env::temp_dir().join(format!("share-test-{}.mp4", uuid::Uuid::new_v4()));
            std::fs::write(&file, b"0123456789").unwrap();
            let media = Media::open(&file).unwrap();
            let access = Arc::new(Access::new(
                secret(),
                super::super::access_code(),
                "https://guest.example".into(),
                expires,
            ));
            Self {
                access,
                file,
                media,
            }
        }
        fn access(&self) -> &Arc<Access> {
            &self.access
        }
        fn path(&self, suffix: &str) -> String {
            format!("/{}{suffix}", self.access().token)
        }
        fn raw(&self, request: &str) -> Reply {
            let mut output = Vec::new();
            serve_request(
                &mut io::Cursor::new(request.as_bytes()),
                &mut output,
                self.access(),
                &self.media,
            )
            .unwrap();
            let boundary = output.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let text = std::str::from_utf8(&output[..boundary]).unwrap();
            let mut lines = text.split("\r\n");
            let status = lines
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .parse()
                .unwrap();
            let headers = lines
                .map(|l| {
                    let (k, v) = l.split_once(':').unwrap();
                    (k.to_lowercase(), v.trim().to_owned())
                })
                .collect();
            let reply = Reply {
                status,
                headers,
                body: output[boundary + 4..].to_vec(),
            };
            for (key, value) in [
                ("cache-control", "no-store"),
                ("referrer-policy", "same-origin"),
                ("x-content-type-options", "nosniff"),
                ("content-security-policy", CSP),
            ] {
                assert_eq!(reply.headers.get(key).map(String::as_str), Some(value));
            }
            assert!(!reply
                .headers
                .keys()
                .any(|k| k.starts_with("access-control-")));
            reply
        }
        fn get(&self, method: &str, path: &str, extra: &str) -> Reply {
            self.raw(&format!(
                "{method} {path} HTTP/1.1\r\nHost: guest.example\r\n{extra}\r\n"
            ))
        }
        fn post(&self, body: &str, kind: &str, extra: &str) -> Reply {
            self.raw(&format!("POST {} HTTP/1.1\r\nHost: guest.example\r\nOrigin: https://guest.example\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{extra}\r\n{body}", self.path("/unlock"), body.len()))
        }
        fn unlock(&self) -> String {
            let body = serde_json::json!({"code":self.access().code}).to_string();
            let reply = self.post(&body, "application/json", "");
            assert_eq!(reply.status, 303);
            let cookie = &reply.headers["set-cookie"];
            for flag in [
                "HttpOnly",
                "Secure",
                "SameSite=Strict",
                &format!("Path=/{}/", self.access().token),
            ] {
                assert!(cookie.contains(flag));
            }
            assert!(!cookie.to_lowercase().contains("domain="));
            assert_eq!(reply.headers["location"], self.path("/player"));
            cookie.split(';').next().unwrap().to_owned()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.access.stop();
            let _ = std::fs::remove_file(&self.file);
        }
    }
    #[test]
    fn shares_refuses_token_only_all_media_methods_and_original_paths() {
        let f = Fixture::new();
        for method in ["GET", "HEAD"] {
            let page = f.get(method, &f.path(""), "");
            assert_eq!(page.status, 200);
            if method == "HEAD" {
                assert!(page.body.is_empty());
            } else {
                assert!(String::from_utf8_lossy(&page.body).contains("name=code"));
            }
            assert!(!String::from_utf8_lossy(&page.body).contains(&f.access().code));
            for route in ["/player", "/video", "/download"] {
                for extra in ["", "Range: bytes=0-1\r\n"] {
                    assert_eq!(f.get(method, &f.path(route), extra).status, 403);
                }
            }
        }
        for path in [
            "/api/state".into(),
            "/api/shares".into(),
            "/admin".into(),
            "/".into(),
            f.file.to_string_lossy().into_owned(),
            f.path("/../video"),
            f.path("/%2e%2e/video"),
            f.path("/recording.mp4"),
            f.path("/originals/screen.mp4"),
            f.path("/..\\video"),
            f.path("/video?code=ignored"),
            f.path("/video/"),
            "/other/video".into(),
        ] {
            assert_eq!(f.get("GET", &path, "").status, 404, "{path}");
        }
        assert_eq!(f.get("GET", &f.path("/unlock"), "").status, 405);
    }
    #[test]
    fn shares_unlock_json_form_scoped_cookie_and_no_credentials_in_html() {
        let f = Fixture::new();
        let cookie = f.unlock();
        let player = f.get("GET", &f.path("/player"), &format!("Cookie: {cookie}\r\n"));
        assert_eq!(player.status, 200);
        let page = String::from_utf8(player.body).unwrap();
        assert!(!page.contains(&f.access().code));
        assert!(!page.contains(cookie.split('=').nth(1).unwrap()));
        let other = Fixture::new();
        for route in ["/player", "/video", "/download"] {
            assert_eq!(
                other
                    .get("GET", &other.path(route), &format!("Cookie: {cookie}\r\n"))
                    .status,
                403
            );
        }
        let form = f.post(
            &format!("code={}", f.access().code),
            "application/x-www-form-urlencoded; charset=UTF-8",
            "",
        );
        assert_eq!(form.status, 303);
        assert_eq!(
            f.get(
                "GET",
                &f.path("/video"),
                &format!("Cookie: {cookie}; {cookie}\r\n")
            )
            .status,
            403
        );
    }
    #[test]
    fn shares_wrong_code_global_rate_limit_and_reset() {
        let f = Fixture::new();
        for i in 0..5 {
            assert_eq!(
                f.post(
                    "{\"code\":\"wrong\"}",
                    "application/json",
                    &format!("X-Forwarded-For: 192.0.2.{i}\r\n")
                )
                .status,
                403
            );
        }
        let body = serde_json::json!({"code":f.access().code}).to_string();
        let reply = f.post(
            &body,
            "application/json",
            "X-Forwarded-For: 198.51.100.9\r\n",
        );
        assert_eq!(reply.status, 429);
        assert_eq!(reply.headers["retry-after"], "60");
        f.access()
            .auth
            .lock()
            .unwrap()
            .attempts
            .iter_mut()
            .for_each(|t| *t = Instant::now() - Duration::from_secs(61));
        f.unlock();
    }
    #[test]
    fn shares_bounded_body_and_strict_same_origin() {
        let f = Fixture::new();
        assert_eq!(
            f.get(
                "GET",
                &f.path(""),
                "Referer: https://owner.example/library\r\nSec-Fetch-Site: cross-site\r\n"
            )
            .status,
            200
        );
        let large = "x".repeat(MAX_BODY + 1);
        assert_eq!(f.post(&large, "application/json", "").status, 413);
        assert_eq!(f.post("{", "application/json", "").status, 400);
        assert_eq!(
            f.post("code=a&code=b", "application/x-www-form-urlencoded", "")
                .status,
            400
        );
        assert_eq!(
            f.post("code=%GG", "application/x-www-form-urlencoded", "")
                .status,
            400
        );
        assert_eq!(f.post("code=a", "text/plain", "").status, 415);
        assert_eq!(
            f.raw(&format!(
                "POST {} HTTP/1.1\r\nHost: guest.example\r\nContent-Length: 0\r\n\r\n",
                f.path("/unlock")
            ))
            .status,
            403
        );
        for origin in ["https://evil.example", "null", "https://guest.example.evil"] {
            assert_eq!(f.raw(&format!("POST {} HTTP/1.1\r\nHost: guest.example\r\nOrigin: {origin}\r\nContent-Length: 0\r\n\r\n", f.path("/unlock"))).status, 403);
        }
        assert_eq!(
            f.get("GET", &f.path(""), "Origin: https://evil.example\r\n")
                .status,
            403
        );
        assert_eq!(
            f.raw(&format!(
                "GET {} HTTP/1.1\r\nHost: evil.example\r\n\r\n",
                f.path("")
            ))
            .status,
            403
        );
        assert_eq!(
            f.post(
                "code=a",
                "application/x-www-form-urlencoded",
                "Sec-Fetch-Site: cross-site\r\n"
            )
            .status,
            403
        );
        assert_eq!(
            f.post(
                "code=a",
                "application/x-www-form-urlencoded",
                "Referer: https://evil.example/\r\n"
            )
            .status,
            403
        );
        assert_eq!(
            f.raw(&format!(
                "POST {} HTTP/1.1\r\nHost: guest.example\r\nTransfer-Encoding: chunked\r\n\r\n",
                f.path("/unlock")
            ))
            .status,
            400
        );
        assert_eq!(
            f.get(
                "GET",
                &f.path(""),
                &format!("X-Large: {}\r\n", "x".repeat(MAX_HEAD))
            )
            .status,
            431
        );
    }
    #[test]
    fn shares_head_single_suffix_open_ranges_and_download() {
        let f = Fixture::new();
        let cookie = f.unlock();
        for route in ["/video", "/download"] {
            for (range_header, status, expected, content_range) in [
                ("", 200, "0123456789", None),
                ("bytes=2-4", 206, "234", Some("bytes 2-4/10")),
                ("bytes=8-", 206, "89", Some("bytes 8-9/10")),
                ("bytes=-3", 206, "789", Some("bytes 7-9/10")),
                ("bytes=-30", 206, "0123456789", Some("bytes 0-9/10")),
                ("bytes=0-99", 206, "0123456789", Some("bytes 0-9/10")),
            ] {
                let extra = format!(
                    "Cookie: {cookie}\r\n{}",
                    if range_header.is_empty() {
                        String::new()
                    } else {
                        format!("Range: {range_header}\r\n")
                    }
                );
                for method in ["GET", "HEAD"] {
                    let reply = f.get(method, &f.path(route), &extra);
                    assert_eq!(reply.status, if method == "HEAD" { 200 } else { status });
                    assert_eq!(
                        reply.headers["content-length"],
                        if method == "HEAD" { 10 } else { expected.len() }.to_string()
                    );
                    assert_eq!(
                        reply.headers.get("content-range").map(String::as_str),
                        if method == "HEAD" {
                            None
                        } else {
                            content_range
                        }
                    );
                    assert_eq!(
                        reply.body,
                        if method == "GET" {
                            expected.as_bytes()
                        } else {
                            b""
                        }
                    );
                    if route == "/download" {
                        assert!(reply.headers["content-disposition"].starts_with("attachment;"));
                    }
                }
            }
        }
        for value in [
            "bytes=10-",
            "bytes=4-2",
            "bytes=-0",
            "bytes=",
            "bytes=1-2,4-5",
            "bytes=-",
            "bytes=+1-2",
            "bytes=18446744073709551616-",
        ] {
            for method in ["GET", "HEAD"] {
                let r = f.get(
                    method,
                    &f.path("/video"),
                    &format!("Cookie: {cookie}\r\nRange: {value}\r\n"),
                );
                assert_eq!(
                    r.status,
                    if method == "HEAD" { 200 } else { 416 },
                    "{value}"
                );
                if method == "HEAD" {
                    assert_eq!(r.headers["content-length"], "10");
                } else {
                    assert_eq!(r.headers["content-range"], "bytes */10");
                }
                assert!(r.body.is_empty());
            }
        }
        assert_eq!(range(None, 0), Ok((0, 0, 200)));
        assert_eq!(range(Some("bytes=0-"), 0), Err(()));
    }
    #[test]
    fn shares_auth_precedes_metadata_and_pins_selected_file_identity() {
        let f = Fixture::new();
        std::fs::remove_file(&f.file).unwrap();
        std::fs::write(&f.file, b"original must never be served").unwrap();
        for route in ["/video", "/download"] {
            assert_eq!(
                f.get("GET", &f.path(route), "Range: bytes=0-1\r\n").status,
                403
            );
        }
        let cookie = f.unlock();
        let response = f.get("GET", &f.path("/video"), &format!("Cookie: {cookie}\r\n"));
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"0123456789");
    }
    #[test]
    fn shares_active_reader_revocation_expiry_and_session_expiry() {
        for reason in ["revoke", "expiry", "session"] {
            let f = Fixture::new();
            let cookie = f.unlock();
            let session = cookie.split_once('=').unwrap().1.to_owned();
            let access = if reason == "expiry" {
                let a = Arc::new(Access::new(
                    secret(),
                    super::super::access_code(),
                    "https://guest.example".into(),
                    now() + 50,
                ));
                a.auth
                    .lock()
                    .unwrap()
                    .sessions
                    .insert(session.clone(), now() + 10000);
                a
            } else {
                f.access().clone()
            };
            let mut reader = CheckedReader {
                file: f.media.file.clone(),
                position: 0,
                access: access.clone(),
                session: session.clone(),
                remaining: 10,
            };
            let mut chunk = [0; 2];
            assert_eq!(reader.read(&mut chunk).unwrap(), 2);
            match reason {
                "revoke" => access.stop(),
                "expiry" => std::thread::sleep(Duration::from_millis(65)),
                _ => {
                    access
                        .auth
                        .lock()
                        .unwrap()
                        .sessions
                        .insert(session, now() - 1);
                }
            }
            assert_eq!(
                reader.read(&mut chunk).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            if reason != "session" {
                assert!(access.auth.lock().unwrap().sessions.is_empty());
            }
        }
    }
    #[test]
    fn shares_sessions_bounded_and_stop_clears() {
        let f = Fixture::new();
        for _ in 0..MAX_SESSIONS {
            assert!(f.access().unlock(&f.access().code).is_ok());
        }
        assert!(matches!(f.access().unlock(&f.access().code), Err(Unlock::Full)));
        assert_eq!(f.access().auth.lock().unwrap().sessions.len(), MAX_SESSIONS);
        assert!(f
            .access()
            .auth
            .lock()
            .unwrap()
            .sessions
            .values()
            .all(|e| *e == f.access().expires));
        f.access()
            .auth
            .lock()
            .unwrap()
            .sessions
            .values_mut()
            .for_each(|e| *e = now() - 1);
        assert!(f.access().unlock(&f.access().code).is_ok());
        f.access().stop();
        assert!(!f.access().live());
        assert!(f.access().auth.lock().unwrap().sessions.is_empty());
        assert!(f.access().unlock(&f.access().code).is_err());
    }
    #[test]
    fn shares_expired_session_revoked_and_expired_link_refusal_matrix() {
        for reason in ["session", "revoke", "expiry"] {
            let f = Fixture::new();
            let cookie = f.unlock();
            let expired;
            let target = match reason {
                "session" => {
                    f.access()
                        .auth
                        .lock()
                        .unwrap()
                        .sessions
                        .values_mut()
                        .for_each(|e| *e = now() - 1);
                    &f
                }
                "revoke" => {
                    f.access().stop();
                    &f
                }
                _ => {
                    expired = Fixture::with_expiry(now() - 1);
                    &expired
                }
            };
            for route in ["/player", "/video", "/download"] {
                for method in ["GET", "HEAD"] {
                    for conditional in [
                        "",
                        "Range: bytes=0-1\r\n",
                        "If-None-Match: *\r\n",
                        "If-Modified-Since: Wed, 01 Jan 2020 00:00:00 GMT\r\n",
                    ] {
                        let r = target.get(
                            method,
                            &target.path(route),
                            &format!("Cookie: {cookie}\r\n{conditional}"),
                        );
                        assert_eq!(r.status, if reason == "session" { 403 } else { 404 });
                        // Media requests get nothing; the player page says in words that access ended (and only that).
                        if route == "/player" && method == "GET" {
                            let page = String::from_utf8(r.body.clone()).unwrap();
                            assert!(page.contains(if reason == "session" { "Access has ended" } else { "This link has ended" }), "{reason}: {page}");
                            assert!(!page.contains("<video") && !page.contains(&target.access().code) && !page.contains(cookie.split('=').nth(1).unwrap()));
                        } else {
                            assert!(r.body.is_empty(), "{reason} {route} {method}");
                        }
                        assert!(!r.headers.contains_key("content-range"));
                        assert!(!r.headers.contains_key("accept-ranges"));
                        assert!(!r.headers.contains_key("content-disposition"));
                    }
                }
            }
        }
    }
    #[test]
    fn shares_media_capacity_does_not_block_unlock() {
        let f = Fixture::new();
        let cookie = f.unlock();
        // Park real handler streams inside their body writes, without opening sockets.
        struct PausedWriter {
            started: std::sync::mpsc::Sender<()>,
            release: std::sync::mpsc::Receiver<()>,
        }
        impl Write for PausedWriter {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                if data == b"0123456789" {
                    self.started.send(()).unwrap();
                    self.release
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|_| io::ErrorKind::TimedOut)?;
                }
                Ok(data.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let mut streams = Vec::new();
        for _ in 0..MEDIA_SLOTS {
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let mut writer = PausedWriter {
                started: started_tx.clone(),
                release: release_rx,
            };
            let access = f.access().clone();
            let file = f.media.clone();
            let request = format!(
                "GET {} HTTP/1.1\r\nHost: guest.example\r\nCookie: {cookie}\r\n\r\n",
                f.path("/video")
            );
            let handle = std::thread::spawn(move || {
                serve_request(&mut io::Cursor::new(request), &mut writer, &access, &file)
            });
            streams.push((release_tx, handle));
        }
        for _ in 0..MEDIA_SLOTS {
            started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        assert_eq!(f.access().media.load(Ordering::Acquire), MEDIA_SLOTS);
        assert_eq!(
            f.get("GET", &f.path("/video"), &format!("Cookie: {cookie}\r\n"))
                .status,
            503
        );
        f.unlock();
        for (release, handle) in streams {
            release.send(()).unwrap();
            handle.join().unwrap().unwrap();
        }
        assert_eq!(f.access().media.load(Ordering::Acquire), 0);
        assert_eq!(
            f.get("GET", &f.path("/video"), &format!("Cookie: {cookie}\r\n"))
                .status,
            200
        );
    }
    #[test]
    fn shares_code_is_short_unambiguous_and_compared_leniently_without_weakening() {
        let f = Fixture::new();
        let code = f.access().code.clone();
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|b| super::super::CODE_ALPHABET.contains(&b)));
        assert!(!code.contains(['0', 'O', '1', 'I']));
        let shown = super::super::display_code(&code);
        assert_eq!(shown.len(), 9);
        assert_eq!(shown.replace('-', ""), code);
        assert_ne!(code, f.access().token[..8]);
        // Case, the display dash and spaces are ignored; nothing else is.
        for accepted in [code.clone(), shown.clone(), shown.to_lowercase(), format!(" {} ", shown)] {
            assert!(f.access().unlock(&accepted).is_ok(), "{accepted}");
        }
        for wrong in [String::new(), code[..7].into(), format!("{code}A"), code.replacen(&code[..1], "!", 1), "AAAA-AAAA".into()] {
            assert!(matches!(f.access().unlock(&wrong), Err(Unlock::Refused)), "{wrong}");
        }
    }
    #[test]
    fn shares_only_wrong_codes_count_toward_the_global_limit() {
        let f = Fixture::new();
        for _ in 0..4 {
            assert!(matches!(f.access().unlock("WRONGONE"), Err(Unlock::Refused)));
        }
        // Correct unlocks neither consume the budget nor are blocked by a nearly spent one.
        for _ in 0..10 {
            assert!(f.access().unlock(&f.access().code).is_ok());
        }
        assert!(matches!(f.access().unlock("WRONGTWO"), Err(Unlock::Refused)));
        assert!(f.access().locked());
        assert!(matches!(f.access().unlock(&f.access().code), Err(Unlock::Limited)));
        // The window rolls: a minute later the correct code works again.
        f.access().auth.lock().unwrap().attempts.iter_mut().for_each(|t| *t = Instant::now() - Duration::from_secs(61));
        assert!(!f.access().locked());
        assert!(f.access().unlock(&f.access().code).is_ok());
    }
    #[test]
    fn shares_session_is_an_independent_256_bit_secret_and_the_url_token_stays_128_bit() {
        let f = Fixture::new();
        assert_eq!(f.access().token.len(), 32, "URL capability: 128 bits as 32 hex digits");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..16 {
            let session = f.access().unlock(&f.access().code).unwrap().0;
            assert_eq!(session.len(), 64, "session: 256 bits as 64 hex digits");
            assert!(session.bytes().all(|b| matches!(b, b'0'..=b'9' | b'A'..=b'F')));
            assert!(!session.contains(&f.access().token) && !f.access().token.contains(&session[..16]));
            assert!(seen.insert(session), "sessions must be unique");
        }
        // The cookie round-trips only at the full length: a 128-bit-looking value is not a session.
        let cookie = f.unlock();
        let value = cookie.split_once('=').unwrap().1;
        assert_eq!(value.len(), 64);
        assert_eq!(f.get("GET", &f.path("/player"), &format!("Cookie: {cookie}\r\n")).status, 200);
        let short = format!("__Secure-share={}", &value[..32]);
        assert_eq!(f.get("GET", &f.path("/player"), &format!("Cookie: {short}\r\n")).status, 403);
        // Independence: material of one generator never predicts the other.
        let a = super::super::session_secret();
        let b = super::super::secret();
        assert_ne!(&a[..32], b);
    }
    #[test]
    fn shares_session_lasts_exactly_as_long_as_the_link() {
        let f = Fixture::with_expiry(now() + 3 * 3600 * 1000);
        let cookie = f.unlock();
        let session = cookie.split_once('=').unwrap().1;
        assert_eq!(f.access().auth.lock().unwrap().sessions[session], f.access().expires);
        let reply = f.post(&serde_json::json!({"code":f.access().code}).to_string(), "application/json", "");
        let age: u64 = reply.headers["set-cookie"].split("Max-Age=").nth(1).unwrap().split(';').next().unwrap().parse().unwrap();
        assert!(age > 3 * 3600 - 5 && age <= 3 * 3600, "{age}");
        // No forced 15-minute cut: still live after the old limit would have passed.
        f.access().auth.lock().unwrap().sessions.insert(session.to_owned(), f.access().expires);
        assert!(f.access().session_live(session));
    }

    #[cfg(unix)]
    fn synthetic_link(id: &str, expires: u64) -> super::super::Link {
        let access = Arc::new(Access::new(
            secret(),
            super::super::access_code(),
            "https://guest.example".into(),
            expires,
        ));
        let child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        super::super::Link {
            id: id.into(),
            url: format!("https://guest.example/{}", access.token),
            child: Arc::new(Mutex::new(super::super::ChildGuard(child, None))),
            monitor: None,
            _server: GuestServer {
                access: access.clone(),
                workers: Vec::new(),
            },
            access,
        }
    }
    #[test]
    #[cfg(unix)]
    fn shares_owner_contract_revoke_clear_and_child_reaping() {
        let mut shares = super::super::Shares::default();
        let link = synthetic_link("one", now() + 60000);
        let access = link.access.clone();
        let child = link.child.clone();
        let session = access.unlock(&access.code).unwrap().0;
        let mut active = CheckedReader {
            file: {
                let f = Fixture::new();
                f.media.file.clone()
            },
            position: 0,
            access: access.clone(),
            session,
            remaining: 10,
        };
        let share = shares.insert_ready(link, 15);
        let list = shares.snapshot();
        assert_eq!(list[0]["id"], "one");
        assert_eq!(list[0]["shareId"], share.as_str());
        assert_eq!(list[0]["status"], "ready");
        assert_eq!(list[0]["protected"], true);
        assert_eq!(list[0]["accessCode"].as_str(), Some(super::super::display_code(&access.code).as_str()));
        assert_eq!(list[0]["expiresAt"], access.expires);
        let shares = Arc::new(Mutex::new(shares));
        super::super::revoke(&shares, "unrelated", None);
        assert_eq!(shares.lock().unwrap().entries.len(), 1);
        super::super::revoke(&shares, "one", Some("another-share"));
        assert_eq!(shares.lock().unwrap().entries.len(), 1);
        super::super::revoke(&shares, "one", None);
        assert!(shares.lock().unwrap().entries.is_empty());
        assert!(!access.live());
        assert!(access.auth.lock().unwrap().sessions.is_empty());
        assert!(child.lock().unwrap().0.try_wait().unwrap().is_some());
        assert!(active.read(&mut [0; 2]).is_err());
        let second = synthetic_link("two", now() + 60000);
        let second_access = second.access.clone();
        shares.lock().unwrap().insert_ready(second, 60);
        super::super::clear(&shares);
        assert!(!second_access.live());
        assert!(super::super::list(&shares).as_array().unwrap().is_empty());
    }
    #[test]
    #[cfg(unix)]
    fn shares_expired_cleanup_precedes_four_link_capacity_check() {
        let mut shares = super::super::Shares::default();
        for i in 0..4 {
            shares.insert_ready(synthetic_link(&i.to_string(), if i == 0 { now() - 1 } else { now() + 60000 }), 15);
        }
        drop(shares.sweep());
        assert_eq!(shares.entries.len(), 3);
        assert_eq!(shares.active(), 3);
        shares.insert_ready(synthetic_link("replacement", now() + 60000), 15);
        assert_eq!(shares.active(), 4);
    }
    #[test]
    #[cfg(unix)]
    fn shares_readiness_failure_reaps_synthetic_child_and_exit_invalidates_sessions() {
        let mut link = synthetic_link("timeout", now() + 60000);
        let (_tx, rx) = std::sync::mpsc::channel();
        assert!(super::super::wait_ready(
            &mut link.child.lock().unwrap(),
            &rx,
            Duration::from_millis(2),
            &AtomicBool::new(false)
        )
        .is_err());
        assert!(link.child.lock().unwrap().0.try_wait().unwrap().is_some());
        let access = link.access.clone();
        access.unlock(&access.code).unwrap();
        link.monitor =
            Some(super::super::monitor_tunnel(link.child.clone(), access.clone()).unwrap());
        link.monitor.take().unwrap().join().unwrap();
        assert!(!access.live());
        assert!(access.auth.lock().unwrap().sessions.is_empty());
        // Server destruction itself invalidates sessions, independently of a tunnel.
        let f = Fixture::new();
        f.unlock();
        drop(GuestServer {
            access: f.access().clone(),
            workers: Vec::new(),
        });
        assert!(!f.access().live());
        assert!(f.access().auth.lock().unwrap().sessions.is_empty());
    }
    // ---- Sharing repair (preview.11): the browser's own form, formatted codes, real Range, Never, the pages ----
    /// What a browser puts in `Origin` for the unlock form's own POST, derived from the Referrer-Policy the guest page was
    /// actually served with: Fetch, "append a request Origin header" — a same-origin navigation POST (not a cors request)
    /// from a document whose policy is `no-referrer` carries `null`; `same-origin` carries the document's origin.
    fn browser_form_origin(policy: &str, origin: &str) -> String {
        if policy == "no-referrer" { "null".into() } else { origin.into() }
    }
    fn browser_unlock(f: &Fixture, typed: &str) -> Reply {
        let page = f.get("GET", &f.path(""), "");
        let origin = browser_form_origin(&page.headers["referrer-policy"], "https://guest.example");
        let encoded: String = typed.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
        let body = format!("code={encoded}");
        f.raw(&format!("POST {} HTTP/1.1\r\nHost: guest.example\r\nOrigin: {origin}\r\nSec-Fetch-Site: same-origin\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\nReferer: https://guest.example{}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}", f.path("/unlock"), f.path(""), body.len()))
    }
    #[test]
    fn shares_the_browsers_own_form_post_unlocks_with_the_right_code() {
        let f = Fixture::new();
        let shown = super::super::display_code(&f.access().code);
        // The page is served so that a browser's same-origin form POST names its real origin, not null.
        let page = f.get("GET", &f.path(""), "");
        assert_eq!(page.headers["referrer-policy"], "same-origin");
        assert_eq!(browser_form_origin(&page.headers["referrer-policy"], "https://guest.example"), "https://guest.example");
        let ok = browser_unlock(&f, &shown);
        assert_eq!(ok.status, 303, "the form a browser sends with the right code unlocks");
        assert_eq!(ok.headers["location"], f.path("/player"));
        // The browser rule that broke it: under no-referrer the form says Origin: null, which the strict check refuses.
        assert_eq!(browser_form_origin("no-referrer", "https://guest.example"), "null");
        assert_eq!(f.raw(&format!("POST {} HTTP/1.1\r\nHost: guest.example\r\nOrigin: null\r\nSec-Fetch-Site: same-origin\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 14\r\n\r\ncode=ABCD-EFGH", f.path("/unlock"))).status, 403);
        // A wrong code from the same form is refused, shown as the form again with a message, and counts.
        let bad = browser_unlock(&f, "AAAA-AAAA");
        assert_eq!(bad.status, 403);
        let html = String::from_utf8(bad.body).unwrap();
        assert!(html.contains("role=alert") && html.contains("name=code") && html.contains("That code didn’t work"), "{html}");
        assert!(!html.contains(&f.access().code));
    }
    #[test]
    fn shares_pasted_and_formatted_codes_are_normalised_without_weakening() {
        // The same ways of writing one code, for any link's code.
        let variants = |code: &str| -> Vec<String> {
            let shown = super::super::display_code(code);
            let (a, b) = (&code[..4], &code[4..]);
            vec![
                shown.clone(), shown.to_lowercase(), code.to_owned(), format!("  {shown}\n"), format!("{shown}\r\n"), format!("{a} {b}"), format!("{a}\u{00A0}{b}"), format!("\u{00A0}{shown}\u{00A0}"),
                format!("{a}\u{2011}{b}"), format!("{a}\u{2013}{b}"), format!("{a}\u{2014}{b}"), format!("{a}\u{2212}{b}"), format!("{a}\u{00AD}{b}"), format!("{a}\u{200B}-{b}"), format!("\u{FEFF}{shown}"), format!("{a}\t{b}"), format!("{a} - {b}"),
            ]
        };
        // (One link per variant: a link allows 32 sessions.)
        let count = variants("ABCDEFGH").len();
        for n in 0..count {
            for through_form in [true, false] {
                let link = Fixture::new();
                let typed = variants(&link.access().code)[n].clone();
                let r = if through_form { browser_unlock(&link, &typed) } else { link.post(&serde_json::json!({"code":typed}).to_string(), "application/json", "") };
                assert_eq!(r.status, 303, "{typed:?} should unlock (form: {through_form})");
            }
        }
        let f = Fixture::new();
        let code = f.access().code.clone();
        let shown = super::super::display_code(&code);
        let (a, b) = (&code[..4], &code[4..]);
        // Nothing else is forgiven, and refusals still count and still lock.
        let wrong = [
            format!("{}{}", &code[..7], "A"), format!("{}", &code[..7]), format!("{shown}A"), format!("{a}{b}{a}"), format!("{}\u{0410}{b}", &a[..3]), // Cyrillic А
            format!("{a}.{b}"), format!("{a}_{b}"), format!("{a}/{b}"), format!("{a}\u{00DF}{b}"), "ABCD\u{0000}EFGH".into(), String::new(),
        ];
        let g = Fixture::new();
        let mut counted = 0;
        for typed in wrong.iter().take(5) {
            let r = browser_unlock(&g, typed);
            counted += 1;
            assert_eq!(r.status, 403, "{typed:?} must be refused");
        }
        assert_eq!(counted, 5);
        assert_eq!(browser_unlock(&g, &super::super::display_code(&g.access().code)).status, 429, "five wrong codes lock, even the right one, for the minute");
        // The scan is bounded: a long run of ignorable characters before the code is not a way to hide input from the limit.
        let h = Fixture::new();
        let long = format!("{}{}", " ".repeat(200), super::super::display_code(&h.access().code));
        assert!(matches!(h.access().unlock(&long), Err(Unlock::Refused)), "only the first 64 characters are read");
        for typed in &wrong[5..] {
            let k = Fixture::new();
            assert!(matches!(k.access().unlock(typed), Err(Unlock::Refused)), "{typed:?}");
        }
        assert_eq!(normalize_code("ab\u{200B}-c d\n"), b"ABCD");
    }
    #[test]
    fn shares_redirect_cookie_session_scope_and_the_pages_a_browser_then_loads() {
        let f = Fixture::new();
        let ok = browser_unlock(&f, &super::super::display_code(&f.access().code));
        assert_eq!(ok.status, 303);
        let cookie = &ok.headers["set-cookie"];
        let attrs: Vec<&str> = cookie.split("; ").collect();
        assert!(attrs[0].starts_with("__Secure-share=") && attrs[0].len() == "__Secure-share=".len() + 64);
        for want in [format!("Path=/{}/", f.access().token), "HttpOnly".into(), "Secure".into(), "SameSite=Strict".into()] {
            assert!(attrs.contains(&want.as_str()), "{want} in {cookie}");
        }
        let max_age: u64 = attrs.iter().find_map(|a| a.strip_prefix("Max-Age=")).unwrap().parse().unwrap();
        assert!((1..=60).contains(&max_age), "a session never outlives the 60 s link: {max_age}");
        assert!(!cookie.to_lowercase().contains("domain=") && !cookie.contains(&f.access().code));
        assert_eq!(ok.headers["location"], f.path("/player"));
        // The follow-up the browser makes: same-origin GET with the cookie, Referer from the form page.
        let jar = attrs[0];
        let nav = format!("Cookie: {jar}\r\nReferer: https://guest.example{}\r\nSec-Fetch-Site: same-origin\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\n", f.path(""));
        let player = f.get("GET", &f.path("/player"), &nav);
        assert_eq!(player.status, 200);
        let html = String::from_utf8(player.body).unwrap();
        assert!(html.contains("<video controls") && html.contains(&format!("src='{}'", f.path("/video"))) && html.contains(&format!("href='{}' download", f.path("/download"))), "{html}");
        assert!(!html.contains("<script") && !html.contains("<style") && !html.contains("style="), "no script and no inline style");
        assert!(!html.contains(&f.access().code) && !html.contains(&jar["__Secure-share=".len()..]));
        // Another link's cookie, no cookie, and a cross-site navigation are all refused.
        let other = Fixture::new();
        assert_eq!(other.get("GET", &other.path("/player"), &nav).status, 403);
        assert_eq!(f.get("GET", &f.path("/player"), "").status, 403);
        assert_eq!(f.get("GET", &f.path("/player"), &format!("Cookie: {jar}\r\nSec-Fetch-Site: cross-site\r\n")).status, 403);
        assert_eq!(f.get("GET", &f.path("/player"), &format!("Cookie: {jar}\r\nReferer: https://evil.example/x\r\n")).status, 403);
    }
    #[test]
    fn shares_real_range_playback_the_way_a_browser_asks_for_it() {
        // 200 000 bytes of known content: more than three read chunks, so ranges cross the 64 KiB read boundary.
        let f = Fixture::new();
        let content: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&f.file, &content).unwrap();
        let jar = browser_unlock(&f, &f.access().code.clone());
        let jar = jar.headers["set-cookie"].split("; ").next().unwrap().to_owned();
        let media = |method: &str, route: &str, range: &str| f.get(method, &f.path(route), &format!("Cookie: {jar}\r\nReferer: https://guest.example{}\r\nSec-Fetch-Site: same-origin\r\nSec-Fetch-Mode: no-cors\r\nSec-Fetch-Dest: video\r\nAccept-Encoding: identity\r\n{range}", f.path("/player")));
        // A first probe, then the open-ended ranges a media element issues as it plays and seeks.
        let first = media("GET", "/video", "Range: bytes=0-\r\n");
        assert_eq!((first.status, first.headers["content-range"].as_str(), first.headers["content-length"].as_str()), (206, "bytes 0-199999/200000", "200000"));
        assert_eq!((first.headers["accept-ranges"].as_str(), first.headers["content-type"].as_str()), ("bytes", "video/mp4"));
        assert!(first.body == content);
        let tail = media("GET", "/video", "Range: bytes=131072-\r\n");
        assert_eq!((tail.status, tail.headers["content-range"].as_str()), (206, "bytes 131072-199999/200000"));
        assert!(tail.body == content[131072..]);
        let across = media("GET", "/video", "Range: bytes=65530-65545\r\n");
        assert!(across.body == content[65530..65546] && across.headers["content-range"] == "bytes 65530-65545/200000");
        let suffix = media("GET", "/video", "Range: bytes=-1000\r\n");
        assert!(suffix.status == 206 && suffix.body == content[199000..]);
        // No Range: the whole file with 200 and Accept-Ranges so the browser knows it may seek. HEAD never has a body.
        let whole = media("GET", "/video", "");
        assert!(whole.status == 200 && whole.headers["accept-ranges"] == "bytes" && whole.body == content);
        let head = media("HEAD", "/video", "");
        assert!(head.status == 200 && head.headers["content-length"] == "200000" && head.body.is_empty());
        assert_eq!(media("GET", "/video", "Range: bytes=200000-\r\n").status, 416);
        let download = media("GET", "/download", "");
        assert!(download.status == 200 && download.headers["content-disposition"] == "attachment; filename=recording.mp4");
        // The same without the cookie is refused, with nothing about the file.
        let bare = f.get("GET", &f.path("/video"), "Range: bytes=0-\r\n");
        assert!(bare.status == 403 && bare.body.is_empty() && !bare.headers.contains_key("content-range"));
    }
    #[test]
    fn shares_pages_are_styled_script_free_and_the_policy_is_least_scope() {
        let f = Fixture::new();
        // The policy: no script, no inline style, no connect; one same-origin stylesheet and a data: brand disc.
        for directive in ["default-src 'none'", "style-src 'self'", "img-src data:", "media-src 'self'", "form-action 'self'", "frame-ancestors 'none'", "base-uri 'none'"] {
            assert!(CSP.contains(directive), "{directive}");
        }
        for forbidden in ["unsafe-inline", "unsafe-eval", "script-src", "connect-src", "*", "http:", "https:"] {
            assert!(!CSP.contains(forbidden), "{forbidden}");
        }
        let root = f.get("GET", &f.path(""), "");
        let html = String::from_utf8(root.body).unwrap();
        assert!(html.contains(&format!("href='{}'", f.path("/style.css"))) && html.contains("<form method=post") && html.contains("autocapitalize=characters") && html.contains("autocomplete=off"));
        assert!(html.contains("class=disc") && html.contains("OpenRecorder") && html.contains("lang=en") && html.contains("name=viewport"));
        // (The one http:// is the SVG namespace inside the data: icon.)
        assert!(!html.contains("<script") && !html.contains("<style") && !html.contains(" style=") && !html.contains("onclick") && !html.replace("http://www.w3.org/2000/svg", "").contains("http://") && !html.contains("https://"));
        assert!(!html.contains(&f.access().code) && !html.contains("role=alert"), "no message before anything went wrong");
        // The stylesheet: static, reachable without a session, no secrets, the design's neutral tokens, a deliberate focus ring,
        // dark and compact layouts, and no orange.
        let css = f.get("GET", &f.path("/style.css"), "Sec-Fetch-Site: same-origin\r\nSec-Fetch-Dest: style\r\n");
        assert_eq!((css.status, css.headers["content-type"].as_str()), (200, "text/css; charset=utf-8"));
        let text = String::from_utf8(css.body).unwrap();
        for want in ["#1d1d1f", "#f5f5f7", "var(--hairline)", ":focus-visible", "outline:2px solid var(--accent)", "prefers-color-scheme:dark", "max-width:480px", "system-ui", ".disc"] {
            assert!(text.contains(want), "{want}");
        }
        for orange in ["orange", "#ff9", "#f90", "#fa0", "#ffa500", "#e8590c", "#f97316"] {
            assert!(!text.to_lowercase().contains(orange), "{orange}");
        }
        assert!(!text.contains(&f.access().token) && !text.contains(&f.access().code) && !text.contains("url(") && !text.contains("@import"));
        assert_eq!(f.get("HEAD", &f.path("/style.css"), "").body.len(), 0);
        for bad in ["/style.css?x=1", "/style.css/", "/STYLE.CSS", "/styles.css"] {
            assert_eq!(f.get("GET", &f.path(bad), "").status, 404, "{bad}");
        }
        assert_eq!(f.raw(&format!("POST {} HTTP/1.1\r\nHost: guest.example\r\nContent-Length: 0\r\n\r\n", f.path("/style.css"))).status, 405);
        // Refusals show the form again with a plain message, not a dead end, and never the code.
        let limited = Fixture::new();
        for _ in 0..5 {
            browser_unlock(&limited, "AAAA-AAAA");
        }
        let locked = browser_unlock(&limited, "AAAA-AAAA");
        assert_eq!(locked.status, 429);
        assert_eq!(locked.headers["retry-after"], "60");
        let page = String::from_utf8(locked.body).unwrap();
        assert!(page.contains("role=alert") && page.contains("Too many wrong codes") && page.contains("name=code"));
        // Ended and revoked: the link's own page says so in words; other paths say nothing; expired and revoked are the same.
        for stopped in [{ let r = Fixture::new(); r.access().stop(); r }, Fixture::with_expiry(now() - 1)] {
            let ended = stopped.get("GET", &stopped.path(""), "");
            assert_eq!(ended.status, 404);
            let page = String::from_utf8(ended.body).unwrap();
            assert!(page.contains("This link has ended") && !page.contains("name=code") && !page.contains("<video"));
            assert!(stopped.get("GET", &stopped.path("/video"), "").body.is_empty());
            assert!(stopped.get("GET", "/anything-else", "").body.is_empty());
        }
        // Owner routes and files stay unreachable through the guest namespace.
        for path in ["/settings", "/api/state", "/api/action", "/mcp", "/file/x/recording.mp4", "/library", "/owner", "/openrecorder_home"] {
            let r = f.get("GET", path, "");
            assert!(r.status == 404 && r.body.is_empty(), "{path}");
        }
    }
    #[test]
    fn shares_a_link_with_no_deadline_stays_live_with_a_finite_cookie_and_ends_when_stopped() {
        let f = Fixture::with_expiry(super::super::NEVER);
        assert!(f.access().live());
        let ok = browser_unlock(&f, &super::super::display_code(&f.access().code));
        assert_eq!(ok.status, 303);
        let max_age: u64 = ok.headers["set-cookie"].split("; ").find_map(|a| a.strip_prefix("Max-Age=")).unwrap().parse().unwrap();
        assert_eq!(max_age, 7 * 24 * 3600, "a finite cookie even without a link deadline");
        let jar = ok.headers["set-cookie"].split("; ").next().unwrap().to_owned();
        assert_eq!(f.get("GET", &f.path("/video"), &format!("Cookie: {jar}\r\nRange: bytes=0-3\r\n")).status, 206);
        // The session lives as long as the link does, and not one moment past an explicit stop.
        assert!(f.access().session_live(jar.split('=').nth(1).unwrap()));
        f.access().stop();
        assert!(!f.access().live());
        assert_eq!(f.get("GET", &f.path("/video"), &format!("Cookie: {jar}\r\n")).status, 404);
        assert_eq!(browser_unlock(&f, &super::super::display_code(&f.access().code)).status, 404);
        // A finite link keeps its exact end.
        let finite = Fixture::with_expiry(now() + 5_000);
        assert!(finite.access().live() && finite.access().expires < super::super::NEVER);
        let gone = Fixture::with_expiry(now() - 1);
        assert!(!gone.access().live());
    }

    /// A browser fixture, not a check: serves the real guest handler (`GuestServer` over `Access`) on a loopback port for a
    /// TLS front (`tests/share-browser-fixture.py`) so a person can drive a real browser against it. Ignored unless started
    /// by that script, which supplies synthetic media, a fixed synthetic token and code, and an `Access.origin` naming the
    /// front's own https://127.0.0.1:<port>. Nothing here is production code or a production route; the handler is untouched.
    #[test]
    #[ignore = "browser fixture: started only by tests/share-browser-fixture.py"]
    fn share_browser_fixture_server() {
        let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is set by the fixture script"));
        let (tls_port, media, control, ready, stop) = (var("FIXTURE_TLS_PORT"), var("FIXTURE_MEDIA"), var("FIXTURE_CONTROL"), var("FIXTURE_READY"), var("FIXTURE_STOP"));
        let (token, code) = (var("FIXTURE_TOKEN"), var("FIXTURE_CODE"));
        assert!(token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()) && code.len() == 8, "synthetic, well-formed fixed credentials");
        let expires = match var("FIXTURE_EXPIRY").as_str() {
            "never" => super::super::NEVER,
            "expired" => now() - 1,
            finite => now() + finite.strip_prefix("finite:").and_then(|s| s.parse::<u64>().ok()).expect("never, expired or finite:SECONDS") * 1000,
        };
        let policy = |name: &str| -> Option<&'static str> { match name { "no-referrer" => Some("no-referrer"), "same-origin" => Some("same-origin"), _ => None } };
        *REFERRER_OVERRIDE.lock().unwrap() = policy(&var("FIXTURE_POLICY"));
        let access = Arc::new(Access::new(token, code, format!("https://127.0.0.1:{tls_port}"), expires));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = GuestServer::start(listener, Media::open(Path::new(&media)).unwrap(), access.clone()).unwrap();
        std::fs::write(&ready, port.to_string()).unwrap();
        // The only runtime controls, from a file the script writes: the referrer policy (to switch between the preview.10
        // behaviour and the fix) and an explicit revoke (the production `stop`). No credential is read from it.
        let mut revoked = false;
        while !Path::new(&stop).exists() {
            if let Ok(text) = std::fs::read_to_string(&control) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    *REFERRER_OVERRIDE.lock().unwrap() = v["referrerPolicy"].as_str().and_then(policy);
                    if v["revoke"] == true && !revoked {
                        access.stop();
                        revoked = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        drop(server);
    }

    #[test]
    fn shares_sessions_of_a_link_with_no_deadline_expire_and_give_their_capacity_back() {
        let f = Fixture::with_expiry(super::super::NEVER);
        let week = 7 * 24 * 3600 * 1000;
        let mut jars = Vec::new();
        for _ in 0..MAX_SESSIONS {
            let reply = browser_unlock(&f, &f.access().code.clone());
            assert_eq!(reply.status, 303);
            jars.push(reply.headers["set-cookie"].clone());
        }
        // Every server session ends in a finite time that is the cookie's own lifetime (a week at most).
        let stored: Vec<u64> = f.access().auth.lock().unwrap().sessions.values().copied().collect();
        assert_eq!(stored.len(), MAX_SESSIONS);
        for ends in &stored {
            assert!(*ends < super::super::NEVER && *ends <= now() + week && *ends + 5_000 > now() + week, "a finite server expiry: {ends}");
        }
        for jar in &jars {
            let max_age: u64 = jar.split("; ").find_map(|a| a.strip_prefix("Max-Age=")).unwrap().parse().unwrap();
            assert!((week / 1000 - 5..=week / 1000).contains(&max_age), "cookie lifetime {max_age}");
        }
        // The cap holds while they are alive...
        assert_eq!(browser_unlock(&f, &f.access().code.clone()).status, 503);
        // ...and a week later (their ends reached) the capacity is back, while the link itself is still live and unchanged.
        f.access().auth.lock().unwrap().sessions.values_mut().for_each(|e| *e = now() - 1);
        assert!(f.access().live());
        let again = browser_unlock(&f, &f.access().code.clone());
        assert_eq!(again.status, 303, "expired sessions released their capacity");
        assert_eq!(f.access().auth.lock().unwrap().sessions.len(), 1, "only the new session remains");
        let jar = again.headers["set-cookie"].split("; ").next().unwrap().to_owned();
        assert_eq!(f.get("GET", &f.path("/video"), &format!("Cookie: {jar}\r\nRange: bytes=0-3\r\n")).status, 206);
        // An expired session no longer plays, and the code is still required for a new one.
        let old = jars[0].split("; ").next().unwrap();
        assert_eq!(f.get("GET", &f.path("/video"), &format!("Cookie: {old}\r\n")).status, 403);
        assert_eq!(f.post("code=WRONGONE", "application/x-www-form-urlencoded", "").status, 403);
    }
    #[test]
    fn shares_a_session_ends_with_its_link_or_a_week_whichever_is_first() {
        let t = 1_000_000;
        assert_eq!(session_deadline(t + 60_000, t), t + 60_000, "a finite short link: the link's own end");
        assert_eq!(session_deadline(t + SESSION_MS + 1, t), t + SESSION_MS, "a longer link: a week");
        assert_eq!(session_deadline(super::super::NEVER, t), t + SESSION_MS, "no deadline: a week");
        assert_eq!(session_deadline(super::super::NEVER, u64::MAX - 5), u64::MAX, "no overflow");
        // A finite link's session ends exactly with the link, and its cookie says so.
        let f = Fixture::with_expiry(now() + 90_000);
        let reply = browser_unlock(&f, &f.access().code.clone());
        let ends = *f.access().auth.lock().unwrap().sessions.values().next().unwrap();
        assert_eq!(ends, f.access().expires);
        let max_age: u64 = reply.headers["set-cookie"].split("; ").find_map(|a| a.strip_prefix("Max-Age=")).unwrap().parse().unwrap();
        assert!((88..=90).contains(&max_age), "{max_age}");
    }

}
