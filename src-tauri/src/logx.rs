//! A small log file next to the app data, so a problem on the user's computer can be looked at later.
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static PATH: OnceLock<PathBuf> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

pub fn init(dir: PathBuf) {
  let _ = std::fs::create_dir_all(&dir);
  let p = dir.join("rt-manager.log");
  // keep it small: start over when it grows past 1 MB
  if std::fs::metadata(&p).map(|m| m.len() > 1_000_000).unwrap_or(false) {
    let _ = std::fs::rename(&p, dir.join("rt-manager.old.log"));
  }
  let _ = PATH.set(p);
}

pub fn dir() -> Option<PathBuf> {
  PATH.get().and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

pub fn log(msg: &str) {
  let Some(p) = PATH.get() else { return };
  let _g = LOCK.lock();
  if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = writeln!(f, "{t} {msg}");
  }
}
