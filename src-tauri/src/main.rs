// Retocker Manager: every Retocker program in one window — install, update, remove.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apps;
mod firefox;
mod logx;
mod net;
mod setup;
mod sys;
mod upd;

use std::sync::OnceLock;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// started by an update: the version it updated to (the window says «Обновлено до …»)
static UPDATED: OnceLock<String> = OnceLock::new();

fn show_main(app: &AppHandle) {
  if let Some(w) = app.get_webview_window("main") {
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
  }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn app_info(app: AppHandle) -> Value {
  json!({
    "version": app.package_info().version.to_string(),
    "updated": UPDATED.get().cloned().unwrap_or_default(),
    "installed": setup::installed_dir().is_some(),
  })
}

/// what is installed on this computer (the registry and Firefox's profiles: quick, no network)
#[tauri::command]
async fn apps_local() -> Result<Vec<apps::Local>, String> {
  tauri::async_runtime::spawn_blocking(|| apps::IDS.iter().map(|id| apps::local(id)).collect()).await.map_err(|e| e.to_string())
}

/// which version of a program is out
#[tauri::command]
async fn app_remote(id: String) -> Result<apps::Remote, String> {
  tauri::async_runtime::spawn_blocking(move || apps::remote(&id)).await.map_err(|e| e.to_string())?
}

fn job_events(app: &AppHandle) -> impl Fn(Value) {
  let a = app.clone();
  move |v| {
    let _ = a.emit_to("main", "job", v);
  }
}

/// install or update (progress comes as «job» events)
#[tauri::command]
async fn app_install(app: AppHandle, id: String) -> Result<apps::Local, String> {
  tauri::async_runtime::spawn_blocking(move || apps::install(&id, &job_events(&app))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn app_uninstall(app: AppHandle, id: String, wipe: bool) -> Result<apps::Local, String> {
  tauri::async_runtime::spawn_blocking(move || apps::uninstall(&id, wipe, &job_events(&app))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn app_cancel(id: String) {
  apps::cancel(&id);
}

#[tauri::command]
fn app_open(id: String) -> Result<(), String> {
  apps::open(&id)
}

/// is there a newer manager
#[tauri::command]
async fn upd_check(app: AppHandle) -> Result<Value, String> {
  let cur = app.package_info().version.to_string();
  tauri::async_runtime::spawn_blocking(move || {
    upd::check().map(|m| json!({ "version": m.version, "notes": m.notes, "size": m.size, "newer": upd::newer(&m.version, &cur), "can": upd::usable(&m) }))
  })
  .await
  .map_err(|e| e.to_string())?
}

/// download, check, replace, restart (progress comes as «upd» events)
#[tauri::command]
fn upd_install(app: AppHandle) {
  std::thread::spawn(move || {
    let _ = upd::install(app);
  });
}

#[tauri::command]
fn open_logs() {
  if let Some(d) = logx::dir() {
    sys::open_url(&d.display().to_string());
  }
}

#[tauri::command]
fn log(msg: String) {
  logx::log(&format!("js: {}", msg.chars().take(2000).collect::<String>()));
}

/// the custom title bar buttons
#[tauri::command]
fn win_ctl(window: WebviewWindow, a: String) {
  match a.as_str() {
    "min" => {
      let _ = window.minimize();
    }
    _ => {
      let _ = window.close();
    }
  }
}

// ---------------------------------------------------------------- the build check

/// RM_DEBUG only: `--do status|remote|install|uninstall [--id <program>] [--wipe] --out <file>` does one thing
/// without a window and writes the answer to a file — the build machine checks real installs with it
fn cli(args: &[String]) -> Option<i32> {
  let what = setup::arg_after(args, "--do")?;
  if !net::debug() {
    return None;
  }
  let id = setup::arg_after(args, "--id").unwrap_or_default();
  let ev = |v: Value| logx::log(&format!("  job {v}"));
  let r: Result<Value, String> = match what.as_str() {
    "status" => Ok(json!(apps::IDS.iter().map(|i| apps::local(i)).collect::<Vec<_>>())),
    "remote" => apps::remote(&id).map(|r| json!(r)),
    "install" => apps::install(&id, &ev).map(|l| json!(l)),
    "uninstall" => apps::uninstall(&id, args.iter().any(|a| a == "--wipe"), &ev).map(|l| json!(l)),
    _ => Err("?".into()),
  };
  let ok = r.is_ok();
  let out = match r {
    Ok(v) => json!({ "ok": true, "result": v }),
    Err(e) => json!({ "ok": false, "error": e }),
  };
  if let Some(f) = setup::arg_after(args, "--out") {
    let _ = std::fs::write(f, out.to_string());
  }
  Some(if ok { 0 } else { 1 })
}

fn main() {
  let args: Vec<String> = std::env::args().collect();
  logx::init(sys::local_appdata().join(setup::IDENT).join("logs"));
  std::panic::set_hook(Box::new(|p| logx::log(&format!("panic: {p}"))));
  if let Some(code) = cli(&args) {
    std::process::exit(code);
  }
  // after an update the new copy starts while the old one is still quitting: wait for it
  if let Some(pid) = setup::arg_after(&args, "--wait-pid").and_then(|s| s.parse::<u32>().ok()) {
    let gone = sys::wait_pid(pid, 20000);
    logx::log(&format!("waited for the old copy ({pid}): {}", if gone { "gone" } else { "still there" }));
  }
  if let Some(v) = setup::arg_after(&args, "--updated") {
    let _ = UPDATED.set(v);
  }
  let ctx = tauri::generate_context!();
  let ver = ctx.package_info().version.to_string();
  let mode = setup::mode(&args);
  if mode != setup::Mode::App && args.iter().any(|a| a == "--silent" || a == "/S") {
    std::process::exit(setup::silent(mode, &ver, &args));
  }
  if !sys::webview2_ensure() {
    std::process::exit(1);
  }
  if mode != setup::Mode::App {
    logx::log(&format!("{mode:?} v{ver}"));
    setup::run_ui(mode, ctx);
    return;
  }
  run_app(ctx);
}

fn run_app(ctx: tauri::Context<tauri::Wry>) {
  tauri::Builder::default()
    .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
      // RM_DEBUG only (the build check): a second start with --test-update updates from RM_UPDATE_URL
      if args.iter().any(|a| a == "--test-update") && net::debug() {
        let a = app.clone();
        std::thread::spawn(move || {
          let r = upd::check();
          logx::log(&format!("test update: manifest {:?}", r.as_ref().map(|m| m.version.clone())));
          let _ = upd::install(a);
        });
        return;
      }
      show_main(app)
    }))
    .invoke_handler(tauri::generate_handler![
      app_info, apps_local, app_remote, app_install, app_uninstall, app_cancel, app_open, upd_check, upd_install, open_logs, log, win_ctl
    ])
    .setup(|app| {
      // here, not in main(): a second start (it only brings this window forward) must not delete a download in progress
      upd::cleanup_old();
      logx::log(&format!("start v{} updated={}", app.package_info().version, UPDATED.get().map(|s| s.as_str()).unwrap_or("")));
      let main = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Retocker Manager")
        .inner_size(880.0, 680.0)
        .min_inner_size(720.0, 520.0)
        .center()
        .decorations(false)
        .shadow(true)
        .background_color(tauri::window::Color(25, 25, 25, 255))
        .build()?;
      let _ = main.set_focus();
      Ok(())
    })
    .run(ctx)
    .expect("Retocker Manager could not start");
}
