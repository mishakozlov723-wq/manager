//! Windows plumbing: processes, folders, the registry, shortcuts, one UAC prompt when something needs it,
//! the WebView2 runtime.
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::core::{w, Interface, GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{
  GetCurrentProcessId, GetExitCodeProcess, OpenProcess, TerminateProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows::Win32::UI::Shell::{
  FOLDERID_Desktop, FOLDERID_LocalAppData, FOLDERID_Programs, FOLDERID_RoamingAppData, IShellLinkW, SHGetKnownFolderPath, ShellExecuteExW, ShellExecuteW,
  ShellLink, KF_FLAG_DEFAULT, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
  MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_YESNO, SW_HIDE, SW_SHOWNORMAL,
};

pub const NO_WINDOW: u32 = 0x0800_0000;

pub fn wide(s: &str) -> Vec<u16> {
  s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ---------------------------------------------------------------- folders

fn known(id: &GUID) -> Option<PathBuf> {
  unsafe {
    let p: PWSTR = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as *const _));
    s.map(PathBuf::from)
  }
}
pub fn local_appdata() -> PathBuf {
  known(&FOLDERID_LocalAppData).or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from)).unwrap_or_else(std::env::temp_dir)
}
pub fn roaming_appdata() -> PathBuf {
  known(&FOLDERID_RoamingAppData).or_else(|| std::env::var_os("APPDATA").map(PathBuf::from)).unwrap_or_else(std::env::temp_dir)
}
pub fn programs_menu() -> Option<PathBuf> {
  known(&FOLDERID_Programs)
}
pub fn desktop() -> Option<PathBuf> {
  known(&FOLDERID_Desktop)
}

/// delete a folder with everything in it; a file that is just being closed gets a few tries
pub fn remove_dir(d: &Path) -> bool {
  for _ in 0..12 {
    if !d.exists() || std::fs::remove_dir_all(d).is_ok() {
      return true;
    }
    std::thread::sleep(Duration::from_millis(300));
  }
  false
}

// ---------------------------------------------------------------- processes

/// running processes with this exe name (this process is never in the list)
pub fn pids(exe: &str) -> Vec<u32> {
  let me = unsafe { GetCurrentProcessId() };
  let mut v = Vec::new();
  unsafe {
    let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return v };
    let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut ok = Process32FirstW(snap, &mut e).is_ok();
    while ok {
      let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
      let name = String::from_utf16_lossy(&e.szExeFile[..len]);
      if e.th32ProcessID != me && name.eq_ignore_ascii_case(exe) {
        v.push(e.th32ProcessID);
      }
      ok = Process32NextW(snap, &mut e).is_ok();
    }
    let _ = CloseHandle(snap);
  }
  v
}

/// end every running copy of a program; how many were ended
pub fn kill(exe: &str) -> usize {
  let mut n = 0;
  for pid in pids(exe) {
    unsafe {
      if let Ok(h) = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) {
        if TerminateProcess(h, 0).is_ok() {
          n += 1;
        }
        let _ = WaitForSingleObject(h, 3000);
        let _ = CloseHandle(h);
      }
    }
  }
  n
}

/// wait until a process has ended (the old copy after an update)
pub fn wait_pid(pid: u32, ms: u32) -> bool {
  unsafe {
    match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
      Ok(h) => {
        let r = WaitForSingleObject(h, ms);
        let _ = CloseHandle(h);
        r == WAIT_OBJECT_0
      }
      Err(_) => true, // already gone
    }
  }
}

/// start a program and wait for it, without a console window; the exit code (None if it could not start or
/// did not finish in time). `raw` is the command line after the exe, passed as it is.
pub fn run_wait(exe: &Path, raw: &str, timeout: Duration) -> Option<i32> {
  let mut c = std::process::Command::new(exe).raw_arg(raw).creation_flags(NO_WINDOW).spawn().ok()?;
  let t0 = Instant::now();
  loop {
    if let Ok(Some(s)) = c.try_wait() {
      return Some(s.code().unwrap_or(-1));
    }
    if t0.elapsed() > timeout {
      let _ = c.kill();
      return None;
    }
    std::thread::sleep(Duration::from_millis(100));
  }
}

/// start a program with administrator rights (the one UAC prompt) and wait for it.
/// Ok(exit code), Err(true) if the user said no, Err(false) on any other failure.
pub fn run_elevated(exe: &Path, params: &str) -> Result<u32, bool> {
  let f = wide(&exe.display().to_string());
  let p = wide(params);
  let mut sei = SHELLEXECUTEINFOW {
    cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
    fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
    lpVerb: w!("runas"),
    lpFile: PCWSTR(f.as_ptr()),
    lpParameters: PCWSTR(p.as_ptr()),
    nShow: SW_HIDE.0,
    ..Default::default()
  };
  unsafe {
    if let Err(e) = ShellExecuteExW(&mut sei) {
      // ERROR_CANCELLED: the user pressed «Нет» in the UAC window
      return Err(e.code().0 as u32 & 0xFFFF == 1223);
    }
    if sei.hProcess.is_invalid() {
      return Err(false);
    }
    let _ = WaitForSingleObject(sei.hProcess, INFINITE);
    let mut code = 1u32;
    let _ = GetExitCodeProcess(sei.hProcess, &mut code);
    let _ = CloseHandle(sei.hProcess);
    Ok(code)
  }
}

pub fn comspec() -> PathBuf {
  PathBuf::from(std::env::var("ComSpec").unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".into()))
}

/// a hidden cmd that waits a moment (until this process is gone), then runs `tail` — removes our own exe and folder
pub fn after_exit(tail: &str) {
  let line = format!("/c ping -n 3 127.0.0.1 >nul & {tail}");
  // not from the program folder: a folder that is some process's current folder can not be removed
  let _ = std::process::Command::new(comspec()).raw_arg(line).current_dir(std::env::temp_dir()).creation_flags(NO_WINDOW).spawn();
}

pub fn open_url(url: &str) {
  let u = wide(url);
  unsafe {
    ShellExecuteW(None, w!("open"), PCWSTR(u.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
  }
}

// ---------------------------------------------------------------- registry (writes go to HKCU)

pub fn reg_get(root: HKEY, key: &str, name: &str) -> Option<String> {
  let k = wide(key);
  let n = wide(name);
  let nm = if name.is_empty() { PCWSTR::null() } else { PCWSTR(n.as_ptr()) };
  unsafe {
    let mut size = 0u32;
    if RegGetValueW(root, PCWSTR(k.as_ptr()), nm, RRF_RT_REG_SZ, None, None, Some(&mut size)).is_err() || size == 0 {
      return None;
    }
    let mut buf = vec![0u16; size as usize / 2 + 1];
    let mut size2 = (buf.len() * 2) as u32;
    if RegGetValueW(root, PCWSTR(k.as_ptr()), nm, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size2)).is_err() {
      return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
  }
}

pub fn reg_has_key(root: HKEY, key: &str) -> bool {
  let k = wide(key);
  unsafe {
    let mut h = HKEY::default();
    if RegOpenKeyExW(root, PCWSTR(k.as_ptr()), None, KEY_READ, &mut h).is_err() {
      return false;
    }
    let _ = RegCloseKey(h);
    true
  }
}

fn open_w(key: &str) -> Option<HKEY> {
  let k = wide(key);
  let mut h = HKEY::default();
  if unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut h, None) }
    .is_err()
  {
    return None;
  }
  Some(h)
}

pub fn reg_set(key: &str, name: &str, v: &str) -> bool {
  let Some(h) = open_w(key) else { return false };
  let n = wide(name);
  let d = wide(v);
  unsafe {
    let b = std::slice::from_raw_parts(d.as_ptr() as *const u8, d.len() * 2);
    let r = RegSetValueExW(h, PCWSTR(n.as_ptr()), None, REG_SZ, Some(b)).is_ok();
    let _ = RegCloseKey(h);
    r
  }
}

pub fn reg_set_dword(key: &str, name: &str, v: u32) -> bool {
  let Some(h) = open_w(key) else { return false };
  let n = wide(name);
  unsafe {
    let r = RegSetValueExW(h, PCWSTR(n.as_ptr()), None, REG_DWORD, Some(&v.to_le_bytes())).is_ok();
    let _ = RegCloseKey(h);
    r
  }
}

pub fn reg_del_tree(key: &str) {
  let k = wide(key);
  unsafe {
    let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()));
    let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()));
  }
}

// ---------------------------------------------------------------- shortcuts

pub fn shortcut(lnk: &Path, target: &Path, desc: &str) -> bool {
  unsafe {
    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    let Ok(sl) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) else { return false };
    let t = wide(&target.display().to_string());
    let d = wide(desc);
    let wd = wide(&target.parent().map(|p| p.display().to_string()).unwrap_or_default());
    let _ = sl.SetPath(PCWSTR(t.as_ptr()));
    let _ = sl.SetDescription(PCWSTR(d.as_ptr()));
    let _ = sl.SetWorkingDirectory(PCWSTR(wd.as_ptr()));
    let _ = sl.SetIconLocation(PCWSTR(t.as_ptr()), 0);
    let Ok(pf) = sl.cast::<IPersistFile>() else { return false };
    if let Some(p) = lnk.parent() {
      let _ = std::fs::create_dir_all(p);
    }
    let l = wide(&lnk.display().to_string());
    pf.Save(PCWSTR(l.as_ptr()), true).is_ok()
  }
}

// ---------------------------------------------------------------- messages, WebView2

pub fn message(text: &str, warn: bool) {
  let t = wide(text);
  unsafe {
    MessageBoxW(None, PCWSTR(t.as_ptr()), w!("Retocker Manager"), MB_OK | MB_SETFOREGROUND | if warn { MB_ICONWARNING } else { MB_ICONINFORMATION });
  }
}

pub fn ask(text: &str) -> bool {
  let t = wide(text);
  unsafe { MessageBoxW(None, PCWSTR(t.as_ptr()), w!("Retocker Manager"), MB_YESNO | MB_ICONINFORMATION | MB_SETFOREGROUND) == IDYES }
}

const WV2: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// is the Microsoft Edge WebView2 runtime installed (the window of the app is drawn by it)
pub fn webview2_ok() -> bool {
  let good = |v: Option<String>| v.map(|s| !s.is_empty() && s != "0.0.0.0").unwrap_or(false);
  good(reg_get(HKEY_LOCAL_MACHINE, &format!("SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{WV2}"), "pv"))
    || good(reg_get(HKEY_LOCAL_MACHINE, &format!("SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{WV2}"), "pv"))
    || good(reg_get(HKEY_CURRENT_USER, &format!("Software\\Microsoft\\EdgeUpdate\\Clients\\{WV2}"), "pv"))
}

/// no WebView2 (an old Windows 10): ask, download Microsoft's small installer and run it quietly
pub fn webview2_ensure() -> bool {
  if webview2_ok() {
    return true;
  }
  if !ask("Для окна Retocker Manager нужен компонент Microsoft WebView2 — его сейчас нет на этом компьютере.\n\nСкачать и установить его? Это бесплатно и займёт минуту.") {
    return false;
  }
  let dst = std::env::temp_dir().join("MicrosoftEdgeWebview2Setup.exe");
  if crate::net::download("https://go.microsoft.com/fwlink/p/?LinkId=2124703", &dst, &mut |_, _| {}).is_err() {
    message("Не получилось скачать WebView2. Проверь интернет и запусти Retocker Manager ещё раз.", true);
    return false;
  }
  let _ = run_wait(&dst, "/silent /install", Duration::from_secs(600));
  let _ = std::fs::remove_file(&dst);
  if webview2_ok() {
    return true;
  }
  message("WebView2 не установился. Скачай его с сайта Microsoft (Evergreen Bootstrapper) и запусти Retocker Manager ещё раз.", true);
  false
}
