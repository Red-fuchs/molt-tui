//! Diagnostic tools: log/chain/mirror views, notices, wake test. Pure helpers + thin MCP wrappers; reply shapes are not documented, so parsing is tolerant.
use crate::mcp::Mcp;
use serde_json::{json, Value};

/// Keywords kept by the default log filter (case-insensitive).
pub const LOG_KEYS: [&str; 2] = ["propos", "refus"];
pub const LOG_MAX: usize = 500;

/// Flatten a log reply (`{"lines"|"log"|"entries"|"events":[..]}`, a bare array or a string) into one text line per entry.
pub fn log_lines(v: &Value) -> Vec<String> {
    let arr = match v {
        Value::Array(a) => Some(a),
        Value::Object(o) => ["lines", "log", "entries", "events"].iter().find_map(|k| o.get(*k).and_then(Value::as_array)),
        _ => None,
    };
    let one = |x: &Value| match x { Value::String(s) => s.clone(), Value::Object(o) if o.get("text").is_some_and(Value::is_string) => format!("{} {}", o.get("level").and_then(Value::as_str).unwrap_or(""), o["text"].as_str().unwrap_or("")), o => o.to_string() };
    match (arr, v) {
        (Some(a), _) => a.iter().map(one).collect(),
        (None, Value::String(s)) => s.lines().map(str::to_string).collect(),
        _ => vec![],
    }
}

/// Keep lines mentioning proposals/refusals, or (when `q` is set) containing `q` (an ID or word, case-insensitive). Newest last, capped to the last `LOG_MAX`.
pub fn filter_log(lines: &[String], q: &str) -> Vec<String> {
    let q = q.trim().to_lowercase();
    let keep = |l: &&String| { let l = l.to_lowercase(); if q.is_empty() { LOG_KEYS.iter().any(|k| l.contains(k)) } else { l.contains(&q) } };
    let v: Vec<String> = lines.iter().filter(keep).map(|l| crate::text::one(l)).collect();
    v[v.len().saturating_sub(LOG_MAX)..].to_vec()
}

/// Peer IPs (v4 literals) are replaced; use for any non-admin or exported view.
pub fn mask_ips(s: &str) -> String {
    s.split(' ').map(|w| {
        let core = w.trim_matches(|c: char| !(c.is_ascii_digit() || c == '.' || c == ':'));
        let isv4 = { let p: Vec<&str> = core.split(':').next().unwrap_or("").split('.').collect(); p.len() == 4 && p.iter().all(|x| !x.is_empty() && x.parse::<u8>().is_ok()) };
        if isv4 { w.replace(core.split(':').next().unwrap_or(""), "x.x.x.x") } else { w.to_string() }
    }).collect::<Vec<_>>().join(" ")
}

pub fn read_log(m: &Mcp, q: &str) -> Result<Vec<String>, String> { m.call("read_log", json!({"level":"all","lines":2000})).map(|v| filter_log(&log_lines(&v), q)) }

/// Pretty text of any JSON reply, capped (chain/mirror can be large). Strings stay raw.
pub fn pretty(v: &Value, max: usize) -> String {
    let t = match v { Value::String(s) => s.clone(), o => serde_json::to_string_pretty(o).unwrap_or_default() };
    if t.chars().count() > max { let mut c: String = t.chars().take(max).collect(); c.push_str("\n[... gekürzt]"); c } else { t }
}

pub fn read_chain(m: &Mcp) -> Result<String, String> { m.call("read_chain", json!({})).map(|v| pretty(&v, 20_000)) }
pub fn read_mirror(m: &Mcp) -> Result<String, String> { m.call("read_mirror", json!({})).map(|v| pretty(&v, 20_000)) }
pub fn wake_skill(m: &Mcp) -> Result<String, String> { m.call("wake_skill", json!({})).map(|v| pretty(&v, 20_000)) }
pub fn test_wake(m: &Mcp) -> Result<String, String> { m.call("test_wake", json!({})).map(|v| pretty(&v, 4_000)) }
/// `id` = a single notice, None = acknowledge all.
pub fn clear_notice(m: &Mcp, id: Option<&str>) -> Result<(), String> {
    m.call("clear_notice", id.map_or(json!({}), |i| json!({"id": i}))).map(|_| ())
}
/// sha256 hex prefix, 12..=64 digits (what `upload:<hex>` in wiki pages carries).
pub fn valid_checksum(c: &str) -> bool { (12..=64).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_hexdigit()) }
pub fn resolve_upload(m: &Mcp, id: &str) -> Result<Value, String> {
    if !valid_checksum(id) { return Err("ungültige Prüfsumme (sha256 hex, 12..64 Zeichen)".into()) }
    m.call("resolve_upload", json!({"checksum": id}))
}

/// Read-only diagnostic view by name (TUI `--diag`): chain | mirror | wake | log [q] | upload ID.
pub fn view(m: &Mcp, what: &str, arg: &str) -> Result<String, String> {
    match what {
        "chain" => read_chain(m), "mirror" => read_mirror(m), "wake" => wake_skill(m),
        "log" => read_log(m, arg).map(|l| if l.is_empty() { "(keine Treffer)".into() } else { l.join("\n") }),
        "upload" => resolve_upload(m, arg).map(|v| pretty(&v, 4000)),
        w => Err(format!("unbekannt: {w} (chain, mirror, wake, log [q], upload ID)")),
    }
}

/// Open vote on an upload row (`read_uploads` -> `vote`): (op, proposal id, mine_open).
pub fn upload_vote(row: &Value) -> Option<(String, u64, bool)> {
    let v = row.get("vote").filter(|v| v.is_object())?;
    Some((v.get("op").and_then(Value::as_str).unwrap_or("").to_string(), v.get("proposal").or_else(|| v.get("id")).and_then(Value::as_u64)?, v.get("mine_open").and_then(Value::as_bool).unwrap_or(false)))
}

#[cfg(test)]
mod t {
    use super::*;
    use crate::mock::{sim::Sim, start};
    #[test]
    fn shapes_and_filter() {
        let l = log_lines(&json!({"lines":["a propose #4 from 10.0.0.5:4040","b ping","c Refused vote 7", {"k":1}]}));
        assert_eq!(l.len(), 4);
        assert_eq!(filter_log(&l, "").len(), 2);
        assert_eq!(filter_log(&l, "#4").len(), 1);
        assert_eq!(log_lines(&json!("x\ny")), ["x", "y"]);
        assert!(log_lines(&json!(3)).is_empty());
        let many: Vec<String> = (0..600).map(|i| format!("propose {i}")).collect();
        let f = filter_log(&many, ""); assert_eq!((f.len(), f[0].as_str()), (500, "propose 100"));
    }
    #[test]
    fn ips_masked() {
        assert_eq!(mask_ips("peer 10.0.0.5:4040 ok (192.168.1.2)"), "peer x.x.x.x:4040 ok (x.x.x.x)");
        assert_eq!(mask_ips("version 1.2.3 id 12"), "version 1.2.3 id 12");
    }
    #[test]
    fn vote_parse() {
        assert_eq!(upload_vote(&json!({"vote":{"op":"persist","proposal":9,"mine_open":true}})), Some(("persist".into(), 9, true)));
        assert_eq!(upload_vote(&json!({"vote":null})), None);
        assert_eq!(upload_vote(&json!({"vote":{"op":"x"}})), None);
        assert_eq!(pretty(&json!({"a":1}), 3).lines().count() > 1, true);
    }
    #[test]
    fn view_dispatch() {
        let (h, st) = Sim::new("me").into_handler(None);
        let mk = start("T", h);
        let m = Mcp::with(mk.url.clone(), "T".into());
        st.lock().unwrap().log = vec!["propose 1".into()];
        assert_eq!(view(&m, "log", "").unwrap(), "propose 1");
        assert!(view(&m, "chain", "").unwrap().contains("head"));
        assert!(view(&m, "nope", "").is_err());
        assert!(view(&m, "upload", "x").is_err());
    }
    #[test]
    fn wrappers_against_sim() {
        let (h, st) = Sim::new("me").into_handler(None);
        let mk = start("T", h);
        let m = Mcp::with(mk.url.clone(), "T".into());
        st.lock().unwrap().log = vec!["sent proposal 4 to 10.1.2.3".into(), "chat noise".into(), "refused proposal 5".into()];
        assert_eq!(read_log(&m, "").unwrap().len(), 2);
        assert!(read_chain(&m).unwrap().contains("head"));
        assert!(read_mirror(&m).unwrap().contains("mirror"));
        assert!(wake_skill(&m).unwrap().contains("wake"));
        assert!(test_wake(&m).is_ok());
        st.lock().unwrap().notices = vec!["n1".into(), "n2".into()];
        clear_notice(&m, Some("n1")).unwrap();
        assert_eq!(st.lock().unwrap().notices, ["n2"]);
        clear_notice(&m, None).unwrap();
        assert!(st.lock().unwrap().notices.is_empty());
        assert!(resolve_upload(&m, "zz").is_err());
        let id = "a".repeat(32);
        assert!(resolve_upload(&m, &id).is_ok());
    }
}
