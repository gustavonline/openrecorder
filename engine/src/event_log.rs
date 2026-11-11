//! Small local lifecycle journal. No frames, audio, access keys or owner content.
use std::{fs,io::Write,path::Path,sync::Mutex};
static LOCK:Mutex<()>=Mutex::new(());
fn append_at(home:&Path,event:&str,data:serde_json::Value)->std::io::Result<()> {
    let _lock=LOCK.lock().map_err(|_|std::io::Error::other("Journal lock unavailable"))?;
    fs::create_dir_all(home)?;let path=home.join("native-events.jsonl");
    if fs::metadata(&path).is_ok_and(|m|m.len()>262144){let _=fs::remove_file(home.join("native-events.previous.jsonl"));fs::rename(&path,home.join("native-events.previous.jsonl"))?;}
    let mut options=fs::OpenOptions::new();options.create(true).append(true);
    #[cfg(unix)]{use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
    let mut file=options.open(path)?;
    let ms=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    let value=serde_json::json!({"at":ms,"pid":std::process::id(),"event":event,"data":data});
    writeln!(file,"{value}")
}
pub fn event(name:&str,data:serde_json::Value){let _=append_at(&super::library_host::home(),name,data);}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn journal_preserves_prior_run_with_bounded_rotation(){let home=std::env::temp_dir().join(format!("recorder-journal-{}",uuid::Uuid::new_v4()));fs::create_dir_all(&home).unwrap();let old=vec![b'x';262145];fs::write(home.join("native-events.jsonl"),&old).unwrap();append_at(&home,"ui_exit",serde_json::json!({"recording":false})).unwrap();assert_eq!(fs::read(home.join("native-events.previous.jsonl")).unwrap(),old);let text=fs::read_to_string(home.join("native-events.jsonl")).unwrap();let value:serde_json::Value=serde_json::from_str(text.trim()).unwrap();assert_eq!(value["event"],"ui_exit");assert!(text.len()<500);fs::remove_dir_all(home).unwrap();}
}
