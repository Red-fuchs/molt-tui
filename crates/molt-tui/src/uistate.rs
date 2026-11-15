//! Persisted UI state (~/.config/molt-tui/ui.json): collapsed sidebar sections + sidebar visibility.
use serde_json::{json, Value};
use std::{collections::HashSet, path::{Path, PathBuf}};

pub const SECS: [(&str, &str); 4] = [("general", "Allgemein"), ("topics", "Themen"), ("open", "Offene Vorschläge"), ("done", "Erledigte Vorschläge")];

/// Config dir: MOLT_TUI_CONFIG_DIR, else ~/.config/molt-tui.
pub fn default_path() -> Option<PathBuf> {
    let d = std::env::var_os("MOLT_TUI_CONFIG_DIR").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config/molt-tui")))?;
    Some(d.join("ui.json"))
}

/// Missing/corrupt file -> defaults (nothing collapsed, sidebar visible).
pub fn load(p: &Path) -> (HashSet<String>, bool) {
    let v: Value = std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
    let col = v["collapsed"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    (col, v["sidebar"].as_bool().unwrap_or(true))
}

pub fn save(p: &Path, col: &HashSet<String>, sidebar: bool) -> std::io::Result<()> {
    if let Some(d) = p.parent() {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(d)?;
    }
    let mut c: Vec<&String> = col.iter().collect();
    c.sort();
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, json!({"collapsed": c, "sidebar": sidebar}).to_string())?;
    std::fs::rename(tmp, p)
}
