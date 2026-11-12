//! Firefox from the outside: where it is, which version of an extension its profiles have, opening a page in it.
//! An extension can not be installed or removed behind Firefox's back — it always asks the user itself.
use std::path::PathBuf;

use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use crate::sys;

pub fn exe() -> Option<PathBuf> {
  const KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\firefox.exe";
  let reg = [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE].into_iter().filter_map(|r| sys::reg_get(r, KEY, "")).map(|s| PathBuf::from(s.trim_matches('"')));
  let std_dirs = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
    .into_iter()
    .filter_map(std::env::var_os)
    .map(|d| PathBuf::from(d).join("Mozilla Firefox").join("firefox.exe"));
  reg.chain(std_dirs).find(|p| p.is_file())
}

/// every profile folder: the usual place, and the one of Firefox from the Microsoft Store
fn profiles() -> Vec<PathBuf> {
  let mut roots = vec![sys::roaming_appdata().join("Mozilla").join("Firefox").join("Profiles")];
  if let Ok(rd) = std::fs::read_dir(sys::local_appdata().join("Packages")) {
    for e in rd.flatten() {
      if e.file_name().to_string_lossy().starts_with("Mozilla.Firefox") {
        roots.push(e.path().join("LocalCache").join("Roaming").join("Mozilla").join("Firefox").join("Profiles"));
      }
    }
  }
  roots.into_iter().filter_map(|r| std::fs::read_dir(r).ok()).flat_map(|rd| rd.flatten().map(|e| e.path())).collect()
}

/// the version of an installed extension (the newest one, if several profiles have it)
pub fn ext_version(id: &str) -> Option<String> {
  let mut best: Option<String> = None;
  for p in profiles() {
    // Firefox's own list of what is installed in this profile
    let Ok(b) = std::fs::read(p.join("extensions.json")) else { continue };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) else { continue };
    let Some(list) = v.get("addons").and_then(|a| a.as_array()) else { continue };
    for a in list {
      if a.get("id").and_then(|x| x.as_str()) != Some(id) {
        continue;
      }
      let ver = a.get("version").and_then(|x| x.as_str()).unwrap_or("").to_string();
      if best.as_deref().map(|b| crate::upd::newer(&ver, b)).unwrap_or(true) {
        best = Some(ver);
      }
    }
  }
  best
}

/// open an address (or just the browser) in Firefox
pub fn open(url: &str) -> Result<(), String> {
  let fx = exe().ok_or("Firefox не найден на этом компьютере. Установи Firefox и попробуй ещё раз")?;
  std::process::Command::new(fx).arg(url).spawn().map(|_| ()).map_err(|e| format!("Firefox не запустился: {e}"))
}
