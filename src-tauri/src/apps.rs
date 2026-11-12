//! The four programs the manager looks after: what is installed here, what is out, and how each one is
//! installed, updated and removed.
//!   voice, snap — one exe that installs itself: «…Setup.exe --silent», removed by «<exe> --uninstall --silent»
//!   zapret      — its own setup window (it has no quiet mode); removed by its NSIS uninstaller
//!   translate   — a Firefox extension: Firefox itself asks the user to add or remove it
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::{json, Value};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use crate::{firefox, logx, net, sys, upd};

pub const IDS: [&str; 4] = ["voice", "snap", "zapret", "translate"];

const GH: &str = "https://github.com/mishakozlov723-wq/";
const UNINSTALL: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\";
const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

/// a program whose exe is its own installer (Voice and Snap are built the same way)
struct Own {
  id: &'static str,
  name: &'static str,
  exe: &'static str,
  /// its key under «Uninstall» in the registry
  key: &'static str,
  /// the public repository with its releases
  repo: &'static str,
  /// its updates are signed: «<sign>:<version>:<sha256>» with the private half of this key
  sign: &'static str,
  pubkey: [u8; 32],
}

const OWN: [Own; 2] = [
  Own {
    id: "voice",
    name: "Retocker Voice",
    exe: "retocker-voice.exe",
    key: "RetockerVoice",
    repo: "retocker-absolute-download",
    sign: "retocker-voice",
    pubkey: [
      0x86, 0xc4, 0xf5, 0xa0, 0xd1, 0xcc, 0x2d, 0x6b, 0xa3, 0xa8, 0x19, 0xf8, 0xc4, 0x17, 0x88, 0x00, 0xb4, 0xdb, 0x34, 0x2b, 0x69, 0x93, 0x88, 0x3f,
      0xdb, 0xe4, 0x02, 0x84, 0x98, 0xff, 0x04, 0xc3,
    ],
  },
  Own {
    id: "snap",
    name: "Retocker Snap",
    exe: "retocker-snap.exe",
    key: "RetockerSnap",
    repo: "retocker-snap-download",
    sign: "retocker-snap",
    pubkey: [
      0x82, 0x41, 0xab, 0xdf, 0x61, 0x8f, 0xca, 0x90, 0xaf, 0x60, 0xd4, 0xb3, 0xbc, 0xc7, 0xff, 0xcc, 0xa9, 0xbb, 0xb3, 0xf2, 0xeb, 0xa3, 0xd4, 0xf6,
      0x0b, 0x91, 0x21, 0x74, 0x62, 0x18, 0xf9, 0x84,
    ],
  },
];

const ZAP_KEY: &str = "Zapret Manager";
const ZAP_EXE: &str = "zapret-manager.exe";
const ZAP_REPO: &str = "zapret-manager2";
const ZAP_SETUP: &str = "Zapret-Manager-Setup.exe";
const ZAP_IDENT: &str = "com.retocker.zapretmanager";
/// the bypass service Zapret Manager sets up, and the mark it leaves on the one it made
const ZAP_SERVICE: &str = "ZapretManagerWinws";
const ZAP_OWNER: &str = "RetockerZapretManager";

const EXT_ID: &str = "translate@retocker.app";
const EXT_HOST: &str = "https://rt-update.retocker.workers.dev/";

fn own(id: &str) -> Option<&'static Own> {
  OWN.iter().find(|a| a.id == id)
}

pub fn temp_dir() -> PathBuf {
  std::env::temp_dir().join("rt-manager")
}

// ---------------------------------------------------------------- what is installed here

#[derive(serde::Serialize, Clone, Default, Debug)]
pub struct Local {
  pub id: String,
  pub installed: bool,
  pub version: String,
  pub running: bool,
  /// the extension only: is there a Firefox to put it into
  pub firefox: bool,
}

/// the folder and the version a program's installer wrote down (None if the program is not there)
fn reg_install(key: &str, exe: &str) -> Option<(PathBuf, String)> {
  let k = format!("{UNINSTALL}{key}");
  let dir = PathBuf::from(sys::reg_get(HKEY_CURRENT_USER, &k, "InstallLocation")?.trim().trim_matches('"'));
  if !dir.join(exe).is_file() {
    return None;
  }
  Some((dir, sys::reg_get(HKEY_CURRENT_USER, &k, "DisplayVersion").unwrap_or_default()))
}

fn place(id: &str) -> Option<(PathBuf, &'static str)> {
  match own(id) {
    Some(a) => reg_install(a.key, a.exe).map(|(d, _)| (d, a.exe)),
    None if id == "zapret" => reg_install(ZAP_KEY, ZAP_EXE).map(|(d, _)| (d, ZAP_EXE)),
    None => None,
  }
}

pub fn local(id: &str) -> Local {
  let mut l = Local { id: id.to_string(), ..Default::default() };
  let (found, exe) = match own(id) {
    Some(a) => (reg_install(a.key, a.exe), a.exe),
    None if id == "zapret" => (reg_install(ZAP_KEY, ZAP_EXE), ZAP_EXE),
    None => {
      l.firefox = firefox::exe().is_some();
      if let Some(v) = firefox::ext_version(EXT_ID) {
        l.installed = true;
        l.version = v;
      }
      return l;
    }
  };
  if let Some((_, ver)) = found {
    l.installed = true;
    l.version = ver;
    l.running = !sys::pids(exe).is_empty();
  }
  l
}

// ---------------------------------------------------------------- what is out

#[derive(serde::Serialize, Clone, Default, Debug)]
pub struct Remote {
  pub id: String,
  pub version: String,
  pub notes: String,
  pub size: String,
  #[serde(skip)]
  bytes: u64,
  #[serde(skip)]
  url: String,
  #[serde(skip)]
  sha256: String,
  #[serde(skip)]
  sig: String,
}

#[derive(serde::Deserialize)]
struct OwnManifest {
  version: String,
  #[serde(default)]
  notes: String,
  #[serde(default)]
  bytes: u64,
  #[serde(default)]
  url: String,
  #[serde(default)]
  sha256: String,
  #[serde(default)]
  sig: String,
}

#[derive(serde::Deserialize)]
struct ZapManifest {
  tag: String,
  #[serde(default)]
  size: u64,
  #[serde(default)]
  sha256: String,
  #[serde(default)]
  notes: String,
}

/// release notes are written in Markdown; the card shows plain lines
fn plain(notes: &str) -> String {
  let lines: Vec<String> = notes
    .lines()
    .map(|l| l.trim().replace("**", "").replace("__", "").replace('`', ""))
    .filter(|l| !l.is_empty() && !l.starts_with('#'))
    .map(|l| l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).map(|s| s.to_string()).unwrap_or(l))
    .collect();
  let many = lines.len() > 1;
  let s = lines.iter().map(|l| if many { format!("• {l}") } else { l.clone() }).collect::<Vec<_>>().join("\n");
  if s.chars().count() > 900 {
    s.chars().take(900).collect::<String>() + "…"
  } else {
    s
  }
}

/// only digits and dots: the version ends up in file names, addresses and on the screen
fn sane(v: &str) -> bool {
  !v.is_empty() && v.len() <= 24 && v.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn json_at<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
  net::get(url).and_then(|b| serde_json::from_slice::<T>(&b).map_err(|_| "непонятный ответ сервера".to_string()))
}

/// ask the releases page which version is out (every program publishes a small manifest next to its files)
pub fn remote(id: &str) -> Result<Remote, String> {
  let mut r = Remote { id: id.to_string(), ..Default::default() };
  if let Some(a) = own(id) {
    let m: OwnManifest = json_at(&format!("{GH}{}/releases/latest/download/latest.json", a.repo))?;
    if !m.url.starts_with(&format!("{GH}{}/releases/download/", a.repo)) {
      return Err("в описании версии чужой адрес".into());
    }
    r.version = m.version.trim().to_string();
    r.notes = plain(&m.notes);
    r.bytes = m.bytes;
    r.url = m.url;
    r.sha256 = m.sha256.trim().to_lowercase();
    r.sig = m.sig;
  } else if id == "zapret" {
    let m: ZapManifest = json_at(&format!("{GH}{ZAP_REPO}/releases/latest/download/latest.json"))?;
    r.version = m.tag.trim().trim_start_matches('v').to_string();
    r.notes = plain(&m.notes);
    r.bytes = m.size;
    r.url = format!("{GH}{ZAP_REPO}/releases/download/v{}/{ZAP_SETUP}", r.version);
    r.sha256 = m.sha256.trim().to_lowercase();
  } else if id == "translate" {
    // the same list Firefox reads to update the extension
    let v: Value = json_at(&format!("{EXT_HOST}updates.json"))?;
    let ups = v["addons"][EXT_ID]["updates"].as_array().cloned().unwrap_or_default();
    for u in ups {
      let (ver, link) = (u["version"].as_str().unwrap_or(""), u["update_link"].as_str().unwrap_or(""));
      if sane(ver) && link.starts_with(EXT_HOST) && (r.version.is_empty() || upd::newer(ver, &r.version)) {
        r.version = ver.to_string();
        r.url = link.to_string();
      }
    }
  } else {
    return Err("нет такой программы".into());
  }
  if !sane(&r.version) {
    return Err("непонятный ответ сервера".into());
  }
  r.size = net::size_text(r.bytes);
  Ok(r)
}

// ---------------------------------------------------------------- one job per program at a time

static BUSY: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CANCEL: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Job(String);

impl Drop for Job {
  fn drop(&mut self) {
    BUSY.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| x != &self.0);
  }
}

fn begin(id: &str) -> Result<Job, String> {
  let mut b = BUSY.lock().unwrap_or_else(|e| e.into_inner());
  if b.iter().any(|x| x == id) {
    return Err("уже занимаюсь этой программой".into());
  }
  b.push(id.to_string());
  CANCEL.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| x != id);
  Ok(Job(id.to_string()))
}

/// stop waiting for the user (the setup window of Zapret Manager, the question in Firefox)
pub fn cancel(id: &str) {
  CANCEL.lock().unwrap_or_else(|e| e.into_inner()).push(id.to_string());
}

fn cancelled(id: &str) -> bool {
  CANCEL.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|x| x == id)
}

/// «the user changed their mind»: an error with nothing to say
const QUIET: &str = "";

// ---------------------------------------------------------------- download and check

fn fetch_setup(id: &str, r: &Remote, file: &str, ev: &dyn Fn(Value)) -> Result<PathBuf, String> {
  if r.sha256.len() != 64 {
    return Err("в описании версии нет контрольной суммы".into());
  }
  let dst = temp_dir().join(file);
  ev(json!({ "id": id, "stage": "down", "got": 0, "total": r.bytes }));
  let mut last = Instant::now();
  let (sum, got) = net::download(&r.url, &dst, &mut |got, total| {
    let total = if total > 0 { total } else { r.bytes };
    if last.elapsed() > Duration::from_millis(80) || (total > 0 && got >= total) {
      last = Instant::now();
      ev(json!({ "id": id, "stage": "down", "got": got, "total": total }));
    }
  })?;
  ev(json!({ "id": id, "stage": "check" }));
  if sum != r.sha256 || (r.bytes > 0 && r.bytes != got) || !net::is_exe(&dst) {
    let _ = std::fs::remove_file(&dst);
    return Err("файл скачался с ошибкой (контрольная сумма не совпала)".into());
  }
  Ok(dst)
}

/// the manifest is exactly what the program's build machine signed
fn signed(a: &Own, r: &Remote) -> bool {
  let Ok(key) = VerifyingKey::from_bytes(&a.pubkey) else { return false };
  let Some(sig) = net::unhex(&r.sig).and_then(|b| <[u8; 64]>::try_from(b.as_slice()).ok()) else { return false };
  let msg = format!("{}:{}:{}", a.sign, r.version, r.sha256);
  key.verify_strict(msg.as_bytes(), &Signature::from_bytes(&sig)).is_ok()
}

fn code_ok(what: &str, code: Option<i32>) -> Result<(), String> {
  match code {
    Some(0) => Ok(()),
    Some(c) => Err(format!("{what} завершился с ошибкой (код {c})")),
    None => Err(format!("{what} не ответил")),
  }
}

// ---------------------------------------------------------------- Voice, Snap

/// The installers of Voice and Snap close every running retocker*.exe — each other's too.
/// So: who else is running now…
fn others_running(id: &str) -> Vec<&'static Own> {
  OWN.iter().filter(|o| o.id != id && !sys::pids(o.exe).is_empty()).collect()
}

/// …and whoever of them was closed is started again, quietly, the way Windows starts it
fn revive(list: &[&'static Own]) {
  for o in list {
    if sys::pids(o.exe).is_empty() {
      start_quiet(o);
    }
  }
}

fn start_quiet(a: &Own) {
  if let Some((dir, _)) = reg_install(a.key, a.exe) {
    let _ = std::process::Command::new(dir.join(a.exe)).arg("--autostart").current_dir(&dir).spawn();
  }
}

fn install_own(a: &'static Own, ev: &dyn Fn(Value)) -> Result<(), String> {
  let r = remote(a.id)?;
  if !signed(a, &r) {
    return Err("подпись не совпала — файл не от Retocker".into());
  }
  let setup = fetch_setup(a.id, &r, &format!("{}-Setup.exe", a.name.replace(' ', "-")), ev)?;
  let mut flags = String::from("--silent");
  if reg_install(a.key, a.exe).is_some() {
    // an update keeps what the user chose before: starting with Windows and the desktop shortcut
    if sys::reg_get(HKEY_CURRENT_USER, RUN, a.name).is_none() {
      flags += " --no-autostart";
    }
    if !sys::desktop().map(|d| d.join(format!("{}.lnk", a.name)).exists()).unwrap_or(false) {
      flags += " --no-desktop";
    }
  }
  let was_running = !sys::pids(a.exe).is_empty();
  let others = others_running(a.id);
  ev(json!({ "id": a.id, "stage": "install" }));
  let code = sys::run_wait(&setup, &flags, Duration::from_secs(180));
  let _ = std::fs::remove_file(&setup);
  revive(&others);
  code_ok("установщик", code)?;
  if reg_install(a.key, a.exe).is_none() {
    return Err("программа не появилась после установки".into());
  }
  if was_running {
    start_quiet(a);
  }
  Ok(())
}

fn uninstall_own(a: &'static Own, wipe: bool, ev: &dyn Fn(Value)) -> Result<(), String> {
  let (dir, _) = reg_install(a.key, a.exe).ok_or("программа уже удалена")?;
  let others = others_running(a.id);
  ev(json!({ "id": a.id, "stage": "remove" }));
  let code = sys::run_wait(&dir.join(a.exe), if wipe { "--uninstall --silent --wipe" } else { "--uninstall --silent" }, Duration::from_secs(120));
  revive(&others);
  code_ok("удаление", code)?;
  // its exe and folder go a moment after it has quit
  for _ in 0..40 {
    if !dir.join(a.exe).exists() {
      break;
    }
    std::thread::sleep(Duration::from_millis(200));
  }
  if reg_install(a.key, a.exe).is_some() {
    return Err("программа не удалилась".into());
  }
  Ok(())
}

// ---------------------------------------------------------------- Zapret Manager

fn install_zapret(ev: &dyn Fn(Value)) -> Result<(), String> {
  let id = "zapret";
  let r = remote(id)?;
  let setup = fetch_setup(id, &r, ZAP_SETUP, ev)?;
  // its setup refuses to work while the program's window is open
  if !sys::pids(ZAP_EXE).is_empty() {
    ev(json!({ "id": id, "stage": "close" }));
    sys::kill(ZAP_EXE);
    std::thread::sleep(Duration::from_millis(400));
  }
  ev(json!({ "id": id, "stage": "wait", "text": "Открыл установщик — заверши установку в его окне" }));
  let mut child = std::process::Command::new(&setup).current_dir(temp_dir()).spawn().map_err(|e| format!("установщик не запустился: {e}"))?;
  loop {
    if let Ok(Some(_)) = child.try_wait() {
      let _ = std::fs::remove_file(&setup);
      break;
    }
    if cancelled(id) {
      return Err(QUIET.into()); // the setup window stays open; its file goes on the manager's next start
    }
    std::thread::sleep(Duration::from_millis(300));
  }
  match reg_install(ZAP_KEY, ZAP_EXE) {
    Some((_, v)) if !upd::newer(&r.version, &v) => Ok(()),
    _ => Err(QUIET.into()), // the window was closed without installing
  }
}

fn uninstall_zapret(wipe: bool, ev: &dyn Fn(Value)) -> Result<(), String> {
  let id = "zapret";
  let (dir, _) = reg_install(ZAP_KEY, ZAP_EXE).ok_or("программа уже удалена")?;
  let un = dir.join("uninstall.exe");
  if !un.is_file() {
    return Err("не нашёл uninstall.exe в папке программы".into());
  }
  ev(json!({ "id": id, "stage": "close" }));
  sys::kill(ZAP_EXE);
  // The bypass service keeps running without the program, and nothing would be left to turn it off with.
  // Only the service Zapret Manager made itself is touched, never the user's own «zapret».
  let svc = format!("SYSTEM\\CurrentControlSet\\Services\\{ZAP_SERVICE}");
  if sys::reg_get(HKEY_LOCAL_MACHINE, &svc, "ZapretManagerOwner").as_deref() == Some(ZAP_OWNER) {
    ev(json!({ "id": id, "stage": "admin", "text": "Убираю службу обхода — Windows спросит разрешение" }));
    match sys::run_elevated(&sys::comspec(), &format!("/c sc stop {ZAP_SERVICE} & sc delete {ZAP_SERVICE}")) {
      Ok(c) => logx::log(&format!("zapret: service removed (code {c})")),
      Err(true) => return Err("Без разрешения Windows службу обхода не убрать — удаление отменено".into()),
      Err(false) => return Err("Не получилось получить права администратора".into()),
    }
    for _ in 0..25 {
      if !sys::reg_has_key(HKEY_LOCAL_MACHINE, &svc) {
        break;
      }
      std::thread::sleep(Duration::from_millis(200));
    }
  }
  ev(json!({ "id": id, "stage": "remove" }));
  // NSIS wants «_?=» last and without quotes, even with spaces in the path; with it the uninstaller works
  // in place and can be waited for — and then can not delete itself
  let code = sys::run_wait(&un, &format!("/S _?={}", dir.display()), Duration::from_secs(120));
  let _ = std::fs::remove_file(&un);
  let _ = std::fs::remove_dir(&dir);
  code_ok("удаление", code)?;
  if wipe {
    for d in [sys::local_appdata().join("ZapretManager"), sys::local_appdata().join(ZAP_IDENT), sys::roaming_appdata().join(ZAP_IDENT)] {
      if !sys::remove_dir(&d) {
        logx::log(&format!("zapret: could not delete {}", d.display()));
      }
    }
  }
  if reg_install(ZAP_KEY, ZAP_EXE).is_some() {
    return Err("программа не удалилась".into());
  }
  Ok(())
}

// ---------------------------------------------------------------- Retocker Translate (Firefox)

/// look into Firefox's profiles until `done` likes the installed version, the user gives up, or 3 minutes pass
fn wait_ext(done: &dyn Fn(Option<String>) -> bool, late: &str) -> Result<(), String> {
  let t0 = Instant::now();
  loop {
    if done(firefox::ext_version(EXT_ID)) {
      return Ok(());
    }
    if cancelled("translate") {
      return Err(QUIET.into());
    }
    if t0.elapsed() > Duration::from_secs(180) {
      return Err(late.into());
    }
    std::thread::sleep(Duration::from_millis(1200));
  }
}

fn install_ext(ev: &dyn Fn(Value)) -> Result<(), String> {
  let r = remote("translate")?;
  // Firefox opens the signed file and asks «Добавить Retocker Translate?» itself
  firefox::open(&r.url)?;
  ev(json!({ "id": "translate", "stage": "wait", "text": "Открыл Firefox — нажми там «Добавить»" }));
  wait_ext(
    &|v| v.map(|v| !upd::newer(&r.version, &v)).unwrap_or(false),
    "Не дождался ответа в Firefox. Если расширение уже добавлено — нажми «Проверить»",
  )
}

fn uninstall_ext(ev: &dyn Fn(Value)) -> Result<(), String> {
  firefox::open("about:addons")?;
  ev(json!({ "id": "translate", "stage": "wait", "text": "Открыл дополнения Firefox — удали там Retocker Translate" }));
  wait_ext(&|v| v.is_none(), "Не увидел, что расширение удалено. Закрой вкладку дополнений в Firefox и нажми «Проверить»")
}

// ---------------------------------------------------------------- for the window

/// install, or update to the version that is out (the same thing for every program)
pub fn install(id: &str, ev: &dyn Fn(Value)) -> Result<Local, String> {
  let _job = begin(id)?;
  logx::log(&format!("{id}: install"));
  let r = match own(id) {
    Some(a) => install_own(a, ev),
    None if id == "zapret" => install_zapret(ev),
    None if id == "translate" => install_ext(ev),
    None => Err("нет такой программы".into()),
  };
  logx::log(&format!("{id}: install -> {r:?}"));
  r.map(|_| local(id))
}

/// remove a program; `wipe` also deletes its settings and data
pub fn uninstall(id: &str, wipe: bool, ev: &dyn Fn(Value)) -> Result<Local, String> {
  let _job = begin(id)?;
  logx::log(&format!("{id}: uninstall wipe={wipe}"));
  let r = match own(id) {
    Some(a) => uninstall_own(a, wipe, ev),
    None if id == "zapret" => uninstall_zapret(wipe, ev),
    None if id == "translate" => uninstall_ext(ev),
    None => Err("нет такой программы".into()),
  };
  logx::log(&format!("{id}: uninstall -> {r:?}"));
  r.map(|_| local(id))
}

/// start an installed program (for the extension: its page in Firefox)
pub fn open(id: &str) -> Result<(), String> {
  if id == "translate" {
    return firefox::open("about:addons");
  }
  let (dir, exe) = place(id).ok_or("программа не установлена")?;
  start(&dir, exe)
}

fn start(dir: &Path, exe: &str) -> Result<(), String> {
  std::process::Command::new(dir.join(exe)).current_dir(dir).spawn().map(|_| ()).map_err(|e| format!("программа не запустилась: {e}"))
}
