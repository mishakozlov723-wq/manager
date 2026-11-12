//! The manager updates itself: a small manifest (latest.json) on the releases page says which version is out;
//! the new exe is downloaded, checked against the manifest (size, SHA-256), put in place of the running one
//! (a running exe can be renamed, not overwritten) and started; the old copy quits.
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;
use tauri::{AppHandle, Emitter};

use crate::{logx, net, setup};

const MANIFEST: &str = "https://github.com/mishakozlov723-wq/manager/releases/latest/download/latest.json";
/// the new exe may come only from this repository's releases
const DOWNLOADS: &str = "https://github.com/mishakozlov723-wq/manager/releases/download/";

#[derive(serde::Deserialize, serde::Serialize, Clone, Default, Debug)]
pub struct Manifest {
  pub version: String,
  #[serde(default)]
  pub notes: String,
  #[serde(default)]
  pub size: String,
  #[serde(default)]
  pub bytes: u64,
  #[serde(default)]
  pub url: String,
  #[serde(default)]
  pub sha256: String,
}

static MAN: Mutex<Option<Manifest>> = Mutex::new(None);
static BUSY: Mutex<bool> = Mutex::new(false);

/// the build check may point the app at a test manifest
fn test_url() -> Option<String> {
  if !net::debug() {
    return None;
  }
  std::env::var("RM_UPDATE_URL").ok().filter(|u| !u.is_empty())
}

/// a.b.c newer than x.y.z?
pub fn newer(a: &str, b: &str) -> bool {
  let p = |s: &str| s.trim().trim_start_matches('v').split('.').map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
  let (x, y) = (p(a), p(b));
  for i in 0..3 {
    let (m, n) = (x.get(i).copied().unwrap_or(0), y.get(i).copied().unwrap_or(0));
    if m != n {
      return m > n;
    }
  }
  false
}

pub fn check() -> Result<Manifest, String> {
  let url = test_url().unwrap_or_else(|| MANIFEST.to_string());
  let m = net::get(&url).and_then(|b| serde_json::from_slice::<Manifest>(&b).map_err(|_| "непонятный ответ сервера".to_string()))?;
  if m.version.is_empty() {
    return Err("пустой ответ".into());
  }
  *MAN.lock().unwrap_or_else(|e| e.into_inner()) = Some(m.clone());
  Ok(m)
}

/// can this manifest be installed (it names a file of ours and its checksum)
pub fn usable(m: &Manifest) -> bool {
  m.sha256.len() == 64 && (m.url.starts_with(DOWNLOADS) || test_url().is_some())
}

/// put `src` where `target` is: the running target is renamed to rt-manager.old.exe, the new one is copied in
fn swap(src: &Path, target: &Path) -> std::io::Result<PathBuf> {
  let mut old = target.with_file_name("rt-manager.old.exe");
  if old.exists() && std::fs::remove_file(&old).is_err() {
    old = target.with_file_name(format!("rt-manager.old{}.exe", std::process::id()));
  }
  std::fs::rename(target, &old)?;
  if let Err(e) = std::fs::copy(src, target) {
    let _ = std::fs::rename(&old, target);
    return Err(e);
  }
  Ok(old)
}

/// leftovers of the previous update next to the exe, and of downloads in the temp folder
pub fn cleanup_old() {
  let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) else { return };
  if let Ok(rd) = std::fs::read_dir(&dir) {
    for e in rd.flatten() {
      let n = e.file_name().to_string_lossy().to_lowercase();
      if n.starts_with("rt-manager.old") && n.ends_with(".exe") {
        for _ in 0..10 {
          if std::fs::remove_file(e.path()).is_ok() {
            break;
          }
          std::thread::sleep(Duration::from_millis(300));
        }
      }
    }
  }
  let _ = std::fs::remove_dir_all(crate::apps::temp_dir());
}

fn emit(app: &AppHandle, v: serde_json::Value) {
  let _ = app.emit_to("main", "upd", v);
}

/// download → check → swap → start the new one → quit. Progress goes to the window as «upd» events.
pub fn install(app: AppHandle) -> Result<(), String> {
  {
    let mut b = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    if *b {
      return Ok(());
    }
    *b = true;
  }
  let r = install_inner(&app);
  *BUSY.lock().unwrap_or_else(|e| e.into_inner()) = false;
  if let Err(e) = &r {
    logx::log(&format!("update failed: {e}"));
    emit(&app, json!({ "stage": "error", "msg": e }));
  }
  r
}

fn install_inner(app: &AppHandle) -> Result<(), String> {
  let cur = app.package_info().version.to_string();
  let m = match MAN.lock().unwrap_or_else(|e| e.into_inner()).clone() {
    Some(m) => m,
    None => check()?,
  };
  if !newer(&m.version, &cur) {
    return Err("у тебя уже последняя версия".into());
  }
  if !usable(&m) {
    return Err("нет адреса для скачивания".into());
  }
  logx::log(&format!("update: {cur} -> {} from {}", m.version, m.url));
  emit(app, json!({ "stage": "down", "p": 0.0 }));
  let tmp = std::env::temp_dir().join(format!("rt-manager-update-{}.exe", std::process::id()));
  let mut last = Instant::now() - Duration::from_secs(1);
  let hint = m.bytes;
  let (sum, got) = net::download(&m.url, &tmp, &mut |got, total| {
    let total = if total > 0 { total } else { hint };
    if last.elapsed() > Duration::from_millis(80) || (total > 0 && got >= total) {
      last = Instant::now();
      emit(app, json!({ "stage": "down", "p": if total > 0 { got as f64 / total as f64 } else { 0.0 }, "got": got, "total": total }));
    }
  })?;
  emit(app, json!({ "stage": "check" }));
  let good = sum.eq_ignore_ascii_case(m.sha256.trim()) && (m.bytes == 0 || m.bytes == got) && net::is_exe(&tmp);
  if !good {
    let _ = std::fs::remove_file(&tmp);
    return Err("файл скачался с ошибкой (контрольная сумма не совпала)".into());
  }
  let target = std::env::current_exe().map_err(|e| e.to_string())?;
  emit(app, json!({ "stage": "apply" }));
  let r = swap(&tmp, &target);
  let _ = std::fs::remove_file(&tmp);
  r.map_err(|e| format!("не получилось заменить файл: {e}"))?;
  setup::note_version(&target, m.version.trim());
  logx::log(&format!("update: applied {}, restarting", m.version));
  emit(app, json!({ "stage": "restart" }));
  restart(app, &target, m.version.trim())
}

/// start the new exe (it waits until this process is gone) and quit
fn restart(app: &AppHandle, exe: &Path, ver: &str) -> Result<(), String> {
  let pid = std::process::id().to_string();
  std::process::Command::new(exe)
    .args(["--app", "--wait-pid", &pid, "--updated", ver])
    .spawn()
    .map_err(|e| format!("новая версия не запустилась: {e}. Перезапусти менеджер сам"))?;
  std::thread::spawn(|| {
    std::thread::sleep(Duration::from_secs(6));
    std::process::exit(0);
  });
  app.exit(0);
  Ok(())
}
