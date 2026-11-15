//! Download a share into a local cache through moltd and verify it (size + sha256) before anyone may show it.
use crate::files::{verified, Check};
use crate::mcp::Mcp;
use crate::model::FileMeta;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn row(m: &Mcp, id: &str) -> Option<serde_json::Value> {
    let v = m.call("read_uploads", json!({})).ok()?;
    v.get("uploads")?.as_array()?.iter().find(|r| r.get("id").and_then(|x| x.as_str()) == Some(id)).cloned()
}

/// Make sure share `id` is on disk in `cache` (the daemon writes it, we verify) and return its path. `download_file` is async:
/// poll until the file verifies, a checksum mismatch persists after read_uploads says the download is done, an error shows up, or timeout.
pub fn ensure(m: &Mcp, cache: &Path, id: &str, meta: &FileMeta, max: u64, timeout: Duration) -> Result<PathBuf, String> {
    let name = crate::files::cache_name(id).ok_or("ungültige Datei-ID")?;
    if meta.size > max { return Err(format!("Datei zu groß ({} > {max} Bytes)", meta.size)) }
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    let dest = cache.join(name);
    if verified(&dest, meta.size, &meta.checksum) == Check::Ok { return Ok(dest) }
    let _ = std::fs::remove_file(&dest);
    m.call("download_file", json!({"id": id, "dest": dest.to_string_lossy()}))?;
    let t0 = Instant::now();
    let mut last = Check::Missing;
    while t0.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(300));
        last = verified(&dest, meta.size, &meta.checksum);
        let r = row(m, id);
        match last {
            Check::Ok => return Ok(dest),
            Check::Mismatch if r.as_ref().and_then(|r| r.get("local")).and_then(|l| l.as_str()).is_some_and(|l| matches!(l, "downloaded" | "own" | "mirrored")) => break,
            _ => {}
        }
        if let Some(e) = r.as_ref().and_then(|r| r.get("error")).and_then(|e| e.as_str()).filter(|e| !e.is_empty()) {
            let _ = std::fs::remove_file(&dest);
            return Err(crate::text::one(e));
        }
    }
    let _ = std::fs::remove_file(&dest);
    Err(if last == Check::Mismatch { "Prüfsumme stimmt nicht".into() } else { "Zeitüberschreitung beim Laden".into() })
}
