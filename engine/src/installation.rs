//! Stable app location and diagnostics; installation never requests device access.
use std::{path::{Path, PathBuf}, process::{Command, Stdio}, time::{Duration, Instant}};

const BUNDLE_ID: &str = "online.openrecorder.desktop";
const RUNNING_HINT: &str = "OpenRecorder may be recording. Stop and quit every OpenRecorder window and host, then try again.";

/// One running process, as reported by `ps`. Paths may contain spaces, so `comm` and `args` are read separately.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Proc {
    pub pid: u32,
    pub exe: PathBuf,
    pub args: Option<String>,
}

fn parse_ps(output: &str) -> Vec<(u32, String)> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, rest) = line.split_once(char::is_whitespace)?;
            Some((pid.parse().ok()?, rest.trim().to_owned()))
        })
        .collect()
}

pub(crate) fn parse_processes(comm: &str, args: &str) -> Vec<Proc> {
    let args: std::collections::HashMap<u32, String> = parse_ps(args).into_iter().collect();
    parse_ps(comm)
        .into_iter()
        .map(|(pid, exe)| Proc { pid, exe: PathBuf::from(exe), args: args.get(&pid).cloned() })
        .collect()
}

fn ps(columns: &str) -> Result<String, String> {
    let out = Command::new("/bin/ps").args(["-axww", "-o", columns]).stdin(Stdio::null()).stderr(Stdio::null())
        .output().map_err(|e| e.to_string())?;
    if !out.status.success() { return Err("Could not list running processes".into()); }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn running_processes() -> Result<Vec<Proc>, String> {
    Ok(parse_processes(&ps("pid=,comm=")?, &ps("pid=,args=")?))
}

fn is_recorder(exe: &Path) -> bool {
    let in_bundle = exe.components().any(|c| {
        let c = c.as_os_str().to_string_lossy();
        c.starts_with("OpenRecorder") && c.ends_with(".app")
    });
    let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    in_bundle || name == "OpenRecorder" || name == "openrecorder-core"
}

/// Whether `proc` could be recording. Only MCP bridges and `--doctor` are known not to be: they hold no
/// capture state. Everything else - toolbar, library host, capture/synthetic modes, the bundled media
/// helper, or a process whose arguments cannot be read - is treated as possibly recording.
pub(crate) fn may_be_recording(proc: &Proc) -> bool {
    if !is_recorder(&proc.exe) { return false; }
    let passive = proc.args.as_deref().is_some_and(|args| {
        args.split_whitespace().any(|a| a == "--stdio" || a == "--doctor")
    });
    !passive
}

/// Decide whether a different build may replace the installed one. Pure so it can be tested without launching
/// anything. `processes` is the process listing, or why it could not be obtained (which blocks replacement).
pub(crate) fn replacement_allowed(host_recording: bool, processes: Result<&[Proc], &str>, self_pid: u32) -> Result<(), String> {
    if host_recording { return Err(RUNNING_HINT.into()); }
    let processes = processes.map_err(|e| format!("Could not confirm that nothing is recording ({e}). Update skipped."))?;
    if processes.iter().any(|p| p.pid != self_pid && may_be_recording(p)) { return Err(RUNNING_HINT.into()); }
    Ok(())
}

fn field(app: &Path, name: &str) -> Result<String, String> {
    let output = Command::new("/usr/libexec/PlistBuddy").args(["-c", &format!("Print :{name}")])
        .arg(app.join("Contents/Info.plist")).output().map_err(|e| e.to_string())?;
    if !output.status.success() { return Err("OpenRecorder bundle metadata is incomplete".into()); }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn verify_bundle(app: &Path, build: &str) -> Result<(), String> {
    if field(app, "CFBundleIdentifier")? != BUNDLE_ID { return Err("The recorder bundle has an unexpected identity".into()); }
    if field(app, "OpenRecorderBuild")? != build { return Err("The recorder bundle is not the expected build".into()); }
    let signature = Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"])
        .arg(app).stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| e.to_string())?;
    if !signature.success() { return Err("The recorder bundle failed its signature check".into()); }
    Ok(())
}

fn guard_now(host_recording: bool) -> Result<(), String> {
    let processes = running_processes();
    replacement_allowed(host_recording, processes.as_deref().map_err(String::as_str), std::process::id())
}

#[allow(dead_code)]
pub fn stable_mac_app(exe: &Path) -> Result<PathBuf, String> {
    stable_mac_app_checked(exe, false)
}

/// Locate or install `~/Applications/OpenRecorder.app`. A different build never replaces an installed one while
/// any OpenRecorder process may be recording (or `host_recording` says this host has an active take). The copy
/// is verified before the swap, and the previous app is kept until the installed copy verifies; no permission
/// reset or re-signing is performed.
pub fn stable_mac_app_checked(exe: &Path, host_recording: bool) -> Result<PathBuf, String> {
    let incoming = exe.parent().and_then(Path::parent).and_then(Path::parent)
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or("Open the packaged OpenRecorder app")?;
    let user = std::env::var("HOME").map_err(|_| "Cannot locate your Applications folder")?;
    let apps = PathBuf::from(user).join("Applications");
    let target = apps.join("OpenRecorder.app");
    if incoming == target { return Ok(target); }
    if field(incoming, "CFBundleIdentifier")? != BUNDLE_ID {
        return Err("The recorder bundle has an unexpected identity".into());
    }
    let signature = Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"])
        .arg(incoming).stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| e.to_string())?;
    if !signature.success() { return Err("The recorder bundle failed its signature check".into()); }
    let build = field(incoming, "OpenRecorderBuild")?;
    let old = target.exists();
    if old {
        if field(&target, "CFBundleIdentifier")? != BUNDLE_ID {
            return Err("Applications/OpenRecorder.app belongs to another app. Choose a different location first.".into());
        }
        if field(&target, "OpenRecorderBuild").ok().as_deref() == Some(&build) { return Ok(target); }
        guard_now(host_recording)?;
    }
    std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
    let staging = apps.join(format!(".OpenRecorder-{}.app", uuid::Uuid::new_v4()));
    let copied = Command::new("/usr/bin/ditto").arg(incoming).arg(&staging).status().map_err(|e| e.to_string())?;
    if !copied.success() { let _ = std::fs::remove_dir_all(&staging); return Err("Could not prepare the recorder update".into()); }
    // Nothing is touched until the copy is known good and, when replacing, nothing started recording meanwhile.
    if let Err(e) = verify_bundle(&staging, &build).and_then(|_| if old { guard_now(host_recording) } else { Ok(()) }) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    let backup = apps.join(format!(".OpenRecorder-previous-{}.app", uuid::Uuid::new_v4()));
    if old { std::fs::rename(&target, &backup).map_err(|e| { let _ = std::fs::remove_dir_all(&staging); e.to_string() })?; }
    if let Err(e) = std::fs::rename(&staging, &target) {
        if old { let _ = std::fs::rename(&backup, &target); }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("Could not install recorder: {e}"));
    }
    if let Err(e) = verify_bundle(&target, &build) {
        // Put the previous app back; the unverified copy is discarded, the backup is never deleted here.
        if old {
            let failed = apps.join(format!(".OpenRecorder-failed-{}.app", uuid::Uuid::new_v4()));
            if std::fs::rename(&target, &failed).is_ok() && std::fs::rename(&backup, &target).is_ok() {
                let _ = std::fs::remove_dir_all(failed);
            }
        }
        return Err(format!("The installed update could not be verified: {e}"));
    }
    if old { let _ = std::fs::remove_dir_all(backup); }
    Ok(target)
}

pub fn doctor() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut candidates = vec![];
    if let Ok(v) = std::env::var("OPENRECORDER_FFMPEG") { candidates.push(PathBuf::from(v)); }
    if let Some(parent) = exe.parent() {
        candidates.push(parent.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }));
        if let Some(contents) = parent.parent() { candidates.push(contents.join("Helpers/ffmpeg")); }
    }
    let helper = candidates.into_iter().find(|p| p.is_file()).ok_or("Packaged media helper is missing")?;
    let mut child = Command::new(&helper).arg("-version").stdin(Stdio::null()).stdout(Stdio::null())
        .stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() { return Err("Packaged media helper did not start".into()); }
            break;
        }
        if Instant::now() >= deadline { let _ = child.kill(); let _ = child.wait(); return Err("Media helper timed out".into()); }
        std::thread::sleep(Duration::from_millis(20));
    }
    println!("{}", serde_json::json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"nativeUi":true,"localLibrary":"Rust","mediaHelper":helper,"deviceAccessRequested":false}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn proc(pid: u32, exe: &str, args: Option<&str>) -> Proc {
        Proc { pid, exe: PathBuf::from(exe), args: args.map(str::to_owned) }
    }
    const APP: &str = "/Users/o/Applications/OpenRecorder.app/Contents/MacOS/OpenRecorder";
    #[test]
    fn parses_ps_output_with_spaces_in_paths() {
        let comm = "  101 /Users/o/Library/Application Support/x/OpenRecorder\n 7 /sbin/launchd\nbad line\n";
        let args = "  101 /Users/o/Library/Application Support/x/OpenRecorder --toolbar --config /c\n 7 /sbin/launchd\n";
        let all = parse_processes(comm, args);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].pid, 101);
        assert_eq!(all[0].exe, PathBuf::from("/Users/o/Library/Application Support/x/OpenRecorder"));
        assert!(all[0].args.as_deref().unwrap().contains("--toolbar"));
        assert!(!may_be_recording(&all[1]));
    }
    #[test]
    fn bridges_and_doctor_never_block_but_everything_else_does() {
        assert!(!may_be_recording(&proc(1, APP, Some("OpenRecorder --stdio"))));
        assert!(!may_be_recording(&proc(1, APP, Some("OpenRecorder --doctor"))));
        for args in ["OpenRecorder", "OpenRecorder --desktop --config /c", "OpenRecorder --toolbar --config /c", "OpenRecorder --library-host", "OpenRecorder --synthetic /o 5"] {
            assert!(may_be_recording(&proc(1, APP, Some(args))), "{args}");
        }
        // Unreadable arguments and the bundled encoder are conservatively recording.
        assert!(may_be_recording(&proc(1, APP, None)));
        assert!(may_be_recording(&proc(2, "/tmp/OpenRecorder-test.app/Contents/Helpers/ffmpeg", Some("ffmpeg -i x"))));
        assert!(may_be_recording(&proc(3, "/tmp/t/target/debug/openrecorder-core", Some("openrecorder-core --toolbar"))));
        assert!(!may_be_recording(&proc(4, "/usr/bin/ffmpeg", Some("ffmpeg -i x"))));
        assert!(!may_be_recording(&proc(5, "/Applications/Other.app/Contents/MacOS/Other", Some("Other"))));
    }
    #[test]
    fn different_build_is_refused_while_anything_may_be_recording() {
        let me = 100;
        let toolbar = proc(200, APP, Some("OpenRecorder --toolbar"));
        let bridge = proc(300, APP, Some("OpenRecorder --stdio"));
        let host = proc(400, APP, Some("OpenRecorder --library-host"));
        assert!(replacement_allowed(false, Ok(&[bridge.clone()]), me).is_ok());
        assert!(replacement_allowed(false, Ok(&[]), me).is_ok());
        assert!(replacement_allowed(false, Ok(&[bridge.clone(), toolbar]), me).is_err());
        assert!(replacement_allowed(false, Ok(&[host.clone()]), me).is_err());
        // This process itself is the host asking; its own take is reported separately.
        assert!(replacement_allowed(false, Ok(&[proc(me, APP, Some("OpenRecorder --library-host"))]), me).is_ok());
        assert!(replacement_allowed(true, Ok(&[proc(me, APP, Some("OpenRecorder --library-host"))]), me).is_err());
        // An unverifiable process list blocks rather than assuming idle.
        assert!(replacement_allowed(false, Err("ps failed"), me).is_err());
    }
}
