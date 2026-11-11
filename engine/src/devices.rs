//! Read-only device discovery. Selection persists stable identity where available,
//! not list order. Linux /dev/videoN fallback paths can change across reconnects.
use serde::Serialize;
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CameraDevice { pub id: String, pub name: String }
pub fn parse_avfoundation_devices(text: &str) -> Vec<CameraDevice> {
    let mut video=false;let mut out=vec![];
    for line in text.lines() {
        if line.contains("AVFoundation video devices:") {video=true;continue;}
        if line.contains("AVFoundation audio devices:") {video=false;continue;}
        if !video {continue;}
        let Some((left,tail))=line.rsplit_once(" (uid=")else{continue};
        let Some(id)=tail.strip_suffix(')')else{continue};
        let Some((_,name))=left.rsplit_once("] ")else{continue};
        if id.is_empty()||id.len()>512||id.chars().any(char::is_control)||name.is_empty()||out.iter().any(|d:&CameraDevice|d.id==id){continue;}
        out.push(CameraDevice{id:id.to_owned(),name:name.to_owned()});
    }
    out
}
/// Parse both current typed entries and older section-based DirectShow listings.
/// Only an alternative name immediately associated with a video entry is usable.
pub fn parse_dshow_devices(text: &str) -> Vec<CameraDevice> {
    let mut video_section = false;
    let mut pending_name = None;
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("[dshow @ ") else { continue };
        let Some((_, entry)) = rest.split_once(']') else { continue };
        let entry = entry.trim();
        if entry.starts_with("DirectShow video devices") {
            video_section = true;
            pending_name = None;
        } else if entry.starts_with("DirectShow audio devices") {
            video_section = false;
            pending_name = None;
        } else if let Some(alternative) = entry.strip_prefix("Alternative name ") {
            let Some(name) = pending_name.take() else { continue };
            let Some(id) = alternative.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else { continue };
            if id.is_empty() || id.len() > 512 || id.chars().any(|c| c.is_control() || c == '"') {
                continue;
            }
            if !out.iter().any(|d: &CameraDevice| d.id == id) {
                out.push(CameraDevice { id: id.to_owned(), name });
            }
        } else if let Some(quoted) = entry.strip_prefix('"') {
            pending_name = None;
            let Some((name, kind)) = quoted.rsplit_once('"') else { continue };
            let kind = kind.trim();
            if (kind == "(video)" || (kind.is_empty() && video_section))
                && !name.is_empty() && !name.chars().any(char::is_control)
            {
                pending_name = Some(name.to_owned());
            }
        } else {
            pending_name = None;
        }
    }
    out
}
#[cfg(target_os="windows")]
pub fn cameras(helper: &str) -> Result<Vec<CameraDevice>, String> {
    use std::{io::Read, process::{Command, Stdio}, sync::mpsc, time::{Duration, Instant}};
    const OUTPUT_LIMIT: u64 = 65536;
    let until = Instant::now() + Duration::from_secs(3);
    let mut child = Command::new(helper)
        .args(["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped())
        .spawn().map_err(|_| "Camera discovery could not start")?;
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Camera discovery unavailable".into());
    };
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes);
        let _ = send.send(result.map(|_| bytes));
    });
    // FFmpeg normally exits unsuccessfully for the dummy input, so its exit code
    // is not a discovery result. Never join a reader that could hold an open pipe.
    let result = (|| {
        let mut output = None;
        loop {
            if output.is_none() {
                match receive.try_recv() {
                    Ok(Ok(bytes)) if bytes.len() <= OUTPUT_LIMIT as usize => output = Some(bytes),
                    Ok(_) => return Err("Camera discovery output could not be read"),
                    Err(mpsc::TryRecvError::Disconnected) => return Err("Camera discovery interrupted"),
                    Err(mpsc::TryRecvError::Empty) => {},
                }
            }
            let exited = child.try_wait().map_err(|_| "Camera discovery interrupted")?.is_some();
            if exited && output.is_some() { return Ok(output.unwrap()); }
            if Instant::now() >= until { return Err("Camera discovery timed out"); }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    let bytes = result?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "Camera discovery output was not UTF-8")?;
    let devices = parse_dshow_devices(text);
    if devices.is_empty() {
        Err("No camera was found. Reconnect it and try again.".into())
    } else {
        Ok(devices)
    }
}
#[cfg(target_os="macos")]
pub fn cameras(helper:&str)->Result<Vec<CameraDevice>,String>{
    use std::{process::{Command,Stdio},io::Read,time::{Instant,Duration}};
    let mut child=Command::new(helper).args(["-hide_banner","-list_devices","true","-f","avfoundation","-i",""])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|_|"Camera discovery could not start")?;
    let stderr=child.stderr.take().ok_or("Camera discovery unavailable")?;
    let read=std::thread::spawn(move||{let mut text=String::new();let _=stderr.take(65536).read_to_string(&mut text);text});
    let until=Instant::now()+Duration::from_secs(3);
    loop{match child.try_wait(){Ok(Some(_))=>break,Ok(None)if Instant::now()<until=>std::thread::sleep(Duration::from_millis(20)),_=>{let _=child.kill();let _=child.wait();break;}}}
    let text=read.join().map_err(|_|"Camera discovery interrupted")?;
    let devices=parse_avfoundation_devices(&text);
    if devices.is_empty(){Err("No camera was found. Reconnect it and try again.".into())}else{Ok(devices)}
}
#[cfg(target_os="linux")]
pub fn cameras(_helper:&str)->Result<Vec<CameraDevice>,String> {
    use std::path::Path;
    let mut devices=vec![];
    if let Ok(entries)=std::fs::read_dir("/dev/v4l/by-id") {
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().ends_with("video-index0"){continue;}
            let path=entry.path();let real=path.canonicalize().ok();
            let name=real.as_ref().and_then(|p|p.file_name()).and_then(|n|std::fs::read_to_string(Path::new("/sys/class/video4linux").join(n).join("name")).ok()).map(|s|s.trim().to_owned()).unwrap_or_else(||entry.file_name().to_string_lossy().to_string());
            devices.push(CameraDevice{id:path.display().to_string(),name});
        }
    }
    if devices.is_empty() {
        for n in 0..32 {let name=format!("video{n}");let path=format!("/dev/{name}");if Path::new(&path).exists(){let title=std::fs::read_to_string(Path::new("/sys/class/video4linux").join(&name).join("name")).unwrap_or(name);devices.push(CameraDevice{id:path,name:title.trim().to_owned()});}}
    }
    devices.sort_by(|a,b|a.id.cmp(&b.id));Ok(devices)
}
/// One microphone as CPAL reports it. CPAL exposes only a display name, never a
/// stable identifier, so the name is the whole identity and several devices that
/// share it cannot be told apart. They stay listed but cannot be chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct MicrophoneChoice { pub name: String, pub duplicate: bool }
pub fn classify_microphones(mut names: Vec<String>) -> Vec<MicrophoneChoice> {
    names.sort();
    let mut out: Vec<MicrophoneChoice> = Vec::new();
    for name in names {
        match out.last_mut() {
            Some(last) if last.name == name => last.duplicate = true,
            _ => out.push(MicrophoneChoice { name, duplicate: false }),
        }
    }
    out
}
/// Enumerates input devices without opening a stream or asking for access.
pub fn microphones() -> Result<Vec<MicrophoneChoice>, String> {
    use cpal::traits::{HostTrait, DeviceTrait};
    let names = cpal::default_host().input_devices().map_err(|e| e.to_string())?
        .filter_map(|d| d.name().ok()).collect::<Vec<_>>();
    Ok(classify_microphones(names))
}
/// Short tail of a stable camera identifier, shown only to tell same-named cameras apart.
pub fn id_suffix(id: &str) -> String {
    let tail: Vec<char> = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    tail[tail.len().saturating_sub(4)..].iter().collect::<String>().to_uppercase()
}
pub fn shares_name(devices: &[CameraDevice], device: &CameraDevice) -> bool {
    devices.iter().filter(|d| d.name == device.name).count() > 1
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]
 fn dshow_typed_entries_keep_duplicate_names_and_ignore_intermixed_audio() {
     let text = r#"[dshow @ 000001ab] "USB Camera" (video)
[dshow @ 000001ab]   Alternative name "@device_pnp_\\?\usb#vid_1234&pid_5678#a#{video}\global"
[dshow @ 000001ab] "USB Camera" (audio)
[dshow @ 000001ab]   Alternative name "@device_cm_{audio}\\wave_0"
[dshow @ 000001ab] "USB Camera" (video)
[dshow @ 000001ab]   Alternative name "@device_pnp_\\?\usb#vid_1234&pid_5678#b#{video}\global"
[in#0 @ 000002ab] Error opening input: Immediate exit requested"#;
     let devices = parse_dshow_devices(text);
     assert_eq!(devices.len(), 2);
     assert_eq!(devices[0].name, "USB Camera");
     assert_eq!(devices[1].name, "USB Camera");
     assert_eq!(devices[0].id, r"@device_pnp_\\?\usb#vid_1234&pid_5678#a#{video}\global");
     assert_eq!(devices[1].id, r"@device_pnp_\\?\usb#vid_1234&pid_5678#b#{video}\global");
 }
 #[test]
 fn dshow_identity_survives_reordering() {
     let a = "[dshow @ x] \"USB Camera\" (video)\n[dshow @ x] Alternative name \"@device_pnp_A\"\n";
     let b = "[dshow @ y] \"USB Camera\" (video)\n[dshow @ y] Alternative name \"@device_pnp_B\"\n";
     let mut first = parse_dshow_devices(&format!("{a}{b}"));
     let mut reordered = parse_dshow_devices(&format!("{b}{a}"));
     first.sort_by(|a, b| a.id.cmp(&b.id));
     reordered.sort_by(|a, b| a.id.cmp(&b.id));
     assert_eq!(first, reordered);
     assert_eq!(first.len(), 2);
 }
 #[test]
 fn dshow_legacy_sections_do_not_associate_missing_ids_with_audio() {
     let text = r#"[dshow @ x] DirectShow video devices (some may be both video and audio devices)
[dshow @ x] "Missing ID"
[dshow @ x] "Camera"
[dshow @ x]   Alternative name "@device_pnp_CAMERA"
[dshow @ x] "Missing ID before audio"
[dshow @ x] DirectShow audio devices
[dshow @ x]   Alternative name "@device_cm_ORPHAN"
[dshow @ x] "Microphone"
[dshow @ x]   Alternative name "@device_cm_MIC""#;
     assert_eq!(parse_dshow_devices(text), vec![CameraDevice {
         id: "@device_pnp_CAMERA".into(), name: "Camera".into(),
     }]);
 }
 #[test]
 fn dshow_rejects_invalid_ids_and_deduplicates_only_by_id() {
     let mut text = String::from("[dshow @ x] Alternative name \"orphan\"\n");
     for id in [String::new(), "x".repeat(513), "bad\0id".into(), "bad\tid".into(), "bad\rid".into(), "bad\"id".into()] {
         text.push_str(&format!("[dshow @ x] \"Invalid\" (video)\n[dshow @ x] Alternative name \"{id}\"\n"));
         // A rejected alternative must consume its pending display name.
         text.push_str("[dshow @ x] Alternative name \"unassociated\"\n");
     }
     text.push_str("[dshow @ x] \"Missing\" (video)\n[dshow @ x] \"Audio\" (audio)\n[dshow @ x] Alternative name \"audio\"\n");
     text.push_str("[dshow @ x] \"Valid\" (video)\n[dshow @ x] Alternative name \"@device_pnp_OK\"\n");
     text.push_str("[dshow @ x] \"Duplicate\" (video)\n[dshow @ x] Alternative name \"@device_pnp_OK\"\n");
     assert_eq!(parse_dshow_devices(&text), vec![CameraDevice {
         id: "@device_pnp_OK".into(), name: "Valid".into(),
     }]);
 }
 #[test]fn stable_identity_survives_reordering_and_duplicate_names(){let a="AVFoundation video devices:\n[avfoundation @ x] [0] USB Camera (uid=A)\n[avfoundation @ x] [1] USB Camera (uid=B)\nAVFoundation audio devices:\n[avfoundation @ x] [0] Mic (uid=MIC)";let b=a.replace("[0] USB Camera (uid=A)","[8] USB Camera (uid=A)");assert_eq!(parse_avfoundation_devices(a),parse_avfoundation_devices(&b));assert_eq!(parse_avfoundation_devices(a).len(),2);assert_eq!(parse_avfoundation_devices(a)[1].id,"B");}
 #[test]fn rejects_missing_identity_and_ignores_screen_sources(){let text="AVFoundation video devices:\n[x] [0] Legacy camera\n[x] [1] Screen Capture\n[x] [2] Camera (uid=)\n[x] [3] Valid (uid=ABC)\n[x] [4] Duplicate (uid=ABC)";assert_eq!(parse_avfoundation_devices(text),vec![CameraDevice{id:"ABC".into(),name:"Valid".into()}]);}
 #[test]
 fn microphone_duplicates_are_flagged_not_merged_or_chosen() {
     let found = classify_microphones(vec!["USB Mic".into(), "Built-in".into(), "USB Mic".into()]);
     assert_eq!(found, vec![
         MicrophoneChoice { name: "Built-in".into(), duplicate: false },
         MicrophoneChoice { name: "USB Mic".into(), duplicate: true },
     ]);
     assert!(classify_microphones(vec![]).is_empty());
 }
 #[test]
 fn suffix_uses_the_identifier_tail_and_only_same_names_share() {
     assert_eq!(id_suffix("0x1424000046d085c"), "085C");
     assert_eq!(id_suffix("a"), "A");
     assert_eq!(id_suffix(""), "");
     let a = CameraDevice { id: "A1".into(), name: "USB Camera".into() };
     let b = CameraDevice { id: "B2".into(), name: "USB Camera".into() };
     let c = CameraDevice { id: "C3".into(), name: "Other".into() };
     let all = vec![a.clone(), b, c.clone()];
     assert!(shares_name(&all, &a) && !shares_name(&all, &c));
 }
}
