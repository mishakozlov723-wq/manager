//! The installer and the uninstaller of the manager itself — the same exe as the app. Named «…Setup….exe» it
//! installs itself; started with --uninstall it removes itself. Both show a small window in the app's style
//! (ui/setup.html); with --silent they work without one (the build check uses that).
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::System::Registry::HKEY_CURRENT_USER;

use crate::{logx, sys};

pub const UNINST_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\RetockerManager";
// not «retocker-…»: the installers of Voice and Snap close every running retocker*.exe
pub const EXE: &str = "rt-manager.exe";
pub const NAME: &str = "Retocker Manager";
pub const IDENT: &str = "com.retocker.manager";
const ABOUT: &str = "Все программы Retocker в одном месте";

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Mode {
  App,
  Setup,
  Uninstall,
}

pub fn mode(args: &[String]) -> Mode {
  if args.iter().any(|a| a == "--uninstall") {
    return Mode::Uninstall;
  }
  if args.iter().any(|a| a == "--app") {
    return Mode::App;
  }
  let name = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase())).unwrap_or_default();
  if name.contains("setup") {
    Mode::Setup
  } else {
    Mode::App
  }
}

pub fn arg_after(args: &[String], k: &str) -> Option<String> {
  args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned()
}

/// where the installer put the manager (None if it is not installed)
pub fn installed_dir() -> Option<PathBuf> {
  sys::reg_get(HKEY_CURRENT_USER, UNINST_KEY, "InstallLocation").map(PathBuf::from).filter(|d| d.join(EXE).exists())
}

/// always a folder of the user's own, so no administrator rights are ever needed
pub fn default_dir() -> PathBuf {
  installed_dir().unwrap_or_else(|| sys::local_appdata().join("Programs").join(NAME))
}

fn links() -> (Option<PathBuf>, Option<PathBuf>) {
  (sys::programs_menu().map(|p| p.join(format!("{NAME}.lnk"))), sys::desktop().map(|p| p.join(format!("{NAME}.lnk"))))
}

/// after an update inside the app: the version in «Apps & features»
pub fn note_version(exe: &Path, ver: &str) {
  if installed_dir().as_deref() == exe.parent() {
    sys::reg_set(UNINST_KEY, "DisplayVersion", ver);
  }
}

// ---------------------------------------------------------------- install

fn copy_in(src: &Path, exe: &Path) -> Result<(), String> {
  if let (Ok(a), Ok(b)) = (src.canonicalize(), exe.canonicalize()) {
    if a == b {
      return Ok(());
    }
  }
  if let Some(d) = exe.parent() {
    std::fs::create_dir_all(d).map_err(|e| format!("Не получилось создать папку: {e}"))?;
  }
  let mut last = String::new();
  for i in 0..20 {
    match std::fs::copy(src, exe) {
      Ok(_) => {
        // the copy keeps the «downloaded from the internet» mark of the setup file; without it Windows does not
        // ask «are you sure you want to run this?» every time the installed program starts
        let _ = std::fs::remove_file(format!("{}:Zone.Identifier", exe.display()));
        return Ok(());
      }
      Err(e) => {
        last = e.to_string();
        // still in use (a copy that is just closing): move it aside, it goes away on the next start
        if i == 10 && exe.exists() {
          let _ = std::fs::rename(exe, exe.with_file_name("rt-manager.old.exe"));
        }
        std::thread::sleep(Duration::from_millis(250));
      }
    }
  }
  Err(format!("Не получилось скопировать программу: {last}"))
}

fn ymd() -> String {
  // days since 1970 -> civil date (Howard Hinnant's algorithm)
  let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|x| x.as_secs() / 86400).unwrap_or(0) as i64;
  let z = d + 719468;
  let era = z.div_euclid(146097);
  let doe = z - era * 146097;
  let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let day = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };
  let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
  format!("{y:04}{m:02}{day:02}")
}

pub fn install(ver: &str, desktop: bool, step: &dyn Fn(u32, &str)) -> Result<PathBuf, String> {
  let dir = default_dir();
  let me = std::env::current_exe().map_err(|e| e.to_string())?;
  let exe = dir.join(EXE);
  logx::log(&format!("setup v{ver}: into {} desktop={desktop}", dir.display()));

  step(10, "Закрываю Retocker Manager…");
  if sys::kill(EXE) > 0 {
    std::thread::sleep(Duration::from_millis(400));
  }
  step(40, "Копирую программу…");
  copy_in(&me, &exe)?;

  step(70, "Создаю ярлыки…");
  let (menu, desk) = links();
  if let Some(m) = &menu {
    sys::shortcut(m, &exe, ABOUT);
  }
  if let Some(d) = &desk {
    if desktop {
      sys::shortcut(d, &exe, ABOUT);
    } else {
      let _ = std::fs::remove_file(d);
    }
  }

  step(88, "Записываю в Windows…");
  let size_kb = std::fs::metadata(&exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
  let ex = exe.display().to_string();
  sys::reg_set(UNINST_KEY, "DisplayName", NAME);
  sys::reg_set(UNINST_KEY, "DisplayIcon", &format!("{ex},0"));
  sys::reg_set(UNINST_KEY, "DisplayVersion", ver);
  sys::reg_set(UNINST_KEY, "Publisher", "Retocker");
  sys::reg_set(UNINST_KEY, "InstallLocation", &dir.display().to_string());
  sys::reg_set(UNINST_KEY, "InstallDate", &ymd());
  sys::reg_set(UNINST_KEY, "UninstallString", &format!("\"{ex}\" --uninstall"));
  sys::reg_set(UNINST_KEY, "QuietUninstallString", &format!("\"{ex}\" --uninstall --silent"));
  sys::reg_set_dword(UNINST_KEY, "NoModify", 1);
  sys::reg_set_dword(UNINST_KEY, "NoRepair", 1);
  sys::reg_set_dword(UNINST_KEY, "EstimatedSize", size_kb);
  step(100, "Готово");
  logx::log(&format!("setup: installed {ex}"));
  Ok(exe)
}

// ---------------------------------------------------------------- uninstall

/// what to delete once this process is gone (our own exe can not be deleted while it runs)
static TAIL: Mutex<Option<String>> = Mutex::new(None);

/// removes the manager only: the programs installed through it stay
pub fn uninstall(step: &dyn Fn(u32, &str)) -> Result<(), String> {
  let me = std::env::current_exe().ok();
  let dir = installed_dir().or_else(|| {
    me.as_ref().filter(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(EXE)).unwrap_or(false)).and_then(|p| p.parent().map(|d| d.to_path_buf()))
  });
  logx::log(&format!("uninstall: {dir:?}"));
  step(15, "Закрываю Retocker Manager…");
  sys::kill(EXE);
  step(40, "Убираю ярлыки…");
  let (menu, desk) = links();
  for l in [menu, desk].into_iter().flatten() {
    let _ = std::fs::remove_file(l);
  }
  step(60, "Убираю из списка программ…");
  sys::reg_del_tree(UNINST_KEY);
  step(80, "Удаляю журнал и настройки окна…");
  sys::remove_dir(&sys::local_appdata().join(IDENT));
  sys::remove_dir(&sys::roaming_appdata().join(IDENT));
  if let Some(d) = dir {
    let ds = d.display().to_string();
    *TAIL.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("del /f /q \"{ds}\\{EXE}\" & del /f /q \"{ds}\\rt-manager.old*.exe\" & rmdir \"{ds}\""));
  }
  step(100, "Готово");
  Ok(())
}

fn finish(webdata: Option<&Path>) {
  let mut tail = TAIL.lock().unwrap_or_else(|e| e.into_inner()).take();
  if let Some(w) = webdata {
    let t = format!("rmdir /s /q \"{}\"", w.display());
    tail = Some(match tail {
      Some(a) => format!("{a} & {t}"),
      None => t,
    });
  }
  if let Some(t) = tail {
    sys::after_exit(&t);
  }
}

// ---------------------------------------------------------------- without a window

pub fn silent(m: Mode, ver: &str, args: &[String]) -> i32 {
  let log = |p: u32, t: &str| logx::log(&format!("  {p}% {t}"));
  match m {
    Mode::Setup => match install(ver, !args.iter().any(|a| a == "--no-desktop"), &log) {
      Ok(_) => 0,
      Err(e) => {
        logx::log(&format!("setup failed: {e}"));
        1
      }
    },
    Mode::Uninstall => {
      let r = uninstall(&log);
      finish(None);
      if r.is_ok() {
        0
      } else {
        1
      }
    }
    Mode::App => 0,
  }
}

// ---------------------------------------------------------------- the window

static DONE_EXE: Mutex<Option<PathBuf>> = Mutex::new(None);

#[tauri::command]
fn setup_info(app: AppHandle) -> serde_json::Value {
  let inst = installed_dir();
  let desk = links().1.map(|d| d.exists()).unwrap_or(false);
  json!({
    "version": app.package_info().version.to_string(),
    "was": inst.as_ref().and_then(|_| sys::reg_get(HKEY_CURRENT_USER, UNINST_KEY, "DisplayVersion")).unwrap_or_default(),
    "installed": inst.is_some(),
    "desktop": if inst.is_some() { desk } else { true },
    "auto": std::env::var("RM_SETUP_AUTO").map(|v| !v.is_empty()).unwrap_or(false),
  })
}

#[tauri::command]
async fn setup_run(app: AppHandle, desktop: bool) -> Result<String, String> {
  let ver = app.package_info().version.to_string();
  let a = app.clone();
  tauri::async_runtime::spawn_blocking(move || {
    let step = |p: u32, t: &str| {
      let _ = a.emit_to("setup", "setup", json!({ "p": p, "t": t }));
    };
    let r = install(&ver, desktop, &step);
    if let Err(e) = &r {
      logx::log(&format!("setup failed: {e}"));
    }
    r.map(|exe| {
      *DONE_EXE.lock().unwrap_or_else(|e| e.into_inner()) = Some(exe.clone());
      exe.display().to_string()
    })
  })
  .await
  .map_err(|e| e.to_string())?
}

#[tauri::command]
fn setup_launch(app: AppHandle) {
  if let Some(exe) = DONE_EXE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
    let _ = std::process::Command::new(&exe).spawn();
  }
  app.exit(0);
}

#[tauri::command]
async fn uninst_run(app: AppHandle) -> Result<(), String> {
  let a = app.clone();
  tauri::async_runtime::spawn_blocking(move || {
    let step = |p: u32, t: &str| {
      let _ = a.emit_to("setup", "setup", json!({ "p": p, "t": t }));
    };
    uninstall(&step)
  })
  .await
  .map_err(|e| e.to_string())?
}

#[tauri::command]
fn setup_close(app: AppHandle) {
  app.exit(0);
}

#[tauri::command]
fn setup_min(window: WebviewWindow) {
  let _ = window.minimize();
}

pub fn run_ui(m: Mode, ctx: tauri::Context<tauri::Wry>) {
  // its own browser data in the temp folder: the installed app's data may be deleted while this window is open
  let data = std::env::temp_dir().join(format!("rt-manager-setup-{}", std::process::id()));
  let d2 = data.clone();
  let un = m == Mode::Uninstall;
  let app = tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![setup_info, setup_run, setup_launch, uninst_run, setup_close, setup_min])
    .setup(move |app| {
      let init = format!("window.RM_SETUP = {};", json!({ "mode": if un { "uninstall" } else { "setup" } }));
      WebviewWindowBuilder::new(app, "setup", WebviewUrl::App("setup.html".into()))
        .title(if un { "Удаление Retocker Manager" } else { "Установка Retocker Manager" })
        .inner_size(600.0, 440.0)
        .resizable(false)
        .maximizable(false)
        .center()
        .decorations(false)
        .shadow(true)
        .background_color(tauri::window::Color(25, 25, 25, 255))
        .data_directory(d2.clone())
        .initialization_script(&init)
        .build()?;
      if let Some(w) = app.get_webview_window("setup") {
        let _ = w.set_focus();
      }
      Ok(())
    })
    .build(ctx);
  match app {
    Ok(a) => a.run(move |_, e| {
      if let RunEvent::Exit = e {
        finish(Some(&data));
      }
    }),
    Err(e) => {
      logx::log(&format!("setup window failed: {e}"));
      sys::message(&format!("Не получилось открыть окно установки: {e}"), true);
    }
  }
}
