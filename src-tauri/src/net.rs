//! Downloads: small JSON manifests into memory, installers straight into a file with their SHA-256 on the way.
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

/// the build check only: local files instead of addresses, test update manifests
pub fn debug() -> bool {
  std::env::var("RM_DEBUG").map(|v| v == "1").unwrap_or(false)
}

fn agent() -> ureq::Agent {
  let mut b = ureq::AgentBuilder::new()
    .timeout_connect(Duration::from_secs(15))
    .timeout_read(Duration::from_secs(30))
    .user_agent("retocker-manager")
    .try_proxy_from_env(true);
  if let Ok(t) = native_tls::TlsConnector::new() {
    b = b.tls_connector(std::sync::Arc::new(t));
  }
  b.build()
}

pub fn hex(b: &[u8]) -> String {
  b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
  let s = s.trim();
  if s.len() % 2 != 0 || !s.is_ascii() {
    return None;
  }
  (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// a local file given instead of an address (the build check)
fn local(url: &str) -> Option<String> {
  if !debug() {
    return None;
  }
  url.strip_prefix("file:///").or_else(|| url.strip_prefix("file://")).map(|s| s.to_string()).or_else(|| {
    let b = url.as_bytes();
    (b.len() > 2 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')).then(|| url.to_string())
  })
}

fn open(url: &str) -> Result<(Box<dyn Read>, u64), String> {
  if let Some(p) = local(url) {
    let f = std::fs::File::open(p.replace('/', "\\")).map_err(|e| e.to_string())?;
    let n = f.metadata().map(|m| m.len()).unwrap_or(0);
    return Ok((Box::new(f), n));
  }
  if !url.starts_with("https://") {
    return Err("адрес должен начинаться с https://".into());
  }
  let r = agent().get(url).call().map_err(|e| match e {
    ureq::Error::Status(c, _) => format!("сервер ответил {c}"),
    ureq::Error::Transport(_) => "нет связи с сервером".to_string(),
  })?;
  let total: u64 = r.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
  Ok((Box::new(r.into_reader()), total))
}

/// a small file (a manifest) into memory; never more than 2 MB
pub fn get(url: &str) -> Result<Vec<u8>, String> {
  // manifests are cached by GitHub's CDN for a few minutes: a changing query gets the fresh one
  let url = if url.starts_with("https://") {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}t={}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
  } else {
    url.to_string()
  };
  let (rd, _) = open(&url)?;
  let mut out = Vec::new();
  rd.take(2 * 1024 * 1024).read_to_end(&mut out).map_err(|e| e.to_string())?;
  Ok(out)
}

/// a file to disk with progress (bytes so far, total or 0); returns its SHA-256 and size. Never more than 400 MB.
pub fn download(url: &str, dst: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<(String, u64), String> {
  let (rd, total) = open(url)?;
  if let Some(d) = dst.parent() {
    std::fs::create_dir_all(d).map_err(|e| format!("не получилось создать папку: {e}"))?;
  }
  let mut f = std::fs::File::create(dst).map_err(|e| format!("не получилось сохранить файл: {e}"))?;
  let mut rd = rd.take(400 * 1024 * 1024);
  let mut sum = Sha256::new();
  let mut buf = vec![0u8; 64 * 1024];
  let mut got = 0u64;
  loop {
    let n = rd.read(&mut buf).map_err(|_| "связь оборвалась во время скачивания".to_string())?;
    if n == 0 {
      break;
    }
    sum.update(&buf[..n]);
    f.write_all(&buf[..n]).map_err(|e| format!("не получилось сохранить файл: {e}"))?;
    got += n as u64;
    progress(got, total);
  }
  f.flush().map_err(|e| e.to_string())?;
  Ok((hex(&sum.finalize()), got))
}

/// a Windows program starts with «MZ»
pub fn is_exe(p: &Path) -> bool {
  let mut b = [0u8; 2];
  std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut b)).is_ok() && &b == b"MZ"
}

/// «7,2 МБ»
pub fn size_text(bytes: u64) -> String {
  if bytes == 0 {
    return String::new();
  }
  let mb = bytes as f64 / 1_048_576.0;
  if mb < 1.0 {
    format!("{} КБ", (bytes as f64 / 1024.0).round() as u64)
  } else {
    format!("{mb:.1} МБ").replace('.', ",")
  }
}
