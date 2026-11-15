use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;

pub struct Mcp {
    url: String,
    tok: String,
    ag: ureq::Agent,
    sid: Mutex<Option<String>>,
    rt: Duration,
    wt: Duration,
}

/// Parse a JSON-RPC reply that is either plain JSON or SSE-framed (`data:` lines).
pub fn parse_sse(body: &str) -> Option<Value> {
    let b = body.trim();
    if b.starts_with('{') && let Ok(v) = serde_json::from_str(b) { return Some(v) }
    let mut best: Option<Value> = None;
    let mut data = String::new();
    let flush = |data: &mut String, best: &mut Option<Value>| {
        if let Ok(v) = serde_json::from_str::<Value>(data.trim()) && (v.get("result").is_some() || v.get("error").is_some() || best.is_none()) { *best = Some(v) }
        data.clear();
    };
    for l in body.lines() {
        let l = l.trim_end_matches('\r');
        if let Some(d) = l.strip_prefix("data:") {
            if !data.is_empty() { data.push('\n') }
            data.push_str(d.strip_prefix(' ').unwrap_or(d));
        } else if l.is_empty() {
            flush(&mut data, &mut best);
        }
    }
    flush(&mut data, &mut best);
    best
}

/// Extract the tool result (JSON text parsed, else string) or an error message.
pub fn tool_result(v: &Value) -> Result<Value, String> {
    if let Some(e) = v.get("error") {
        return Err(e.get("message").and_then(Value::as_str).unwrap_or("MCP-Fehler").to_string());
    }
    let r = v.get("result").ok_or("Antwort ohne result")?;
    let txt: Vec<&str> = r.get("content").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|c| c.get("text").and_then(Value::as_str)).collect();
    let txt = txt.join("\n");
    if r.get("isError").and_then(Value::as_bool) == Some(true) { return Err(txt) }
    Ok(serde_json::from_str(&txt).unwrap_or(Value::String(txt)))
}

fn emsg(e: ureq::Error, write: bool) -> String {
    match e {
        ureq::Error::Transport(t) if write && !matches!(t.kind(), ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Dns) =>
            format!("{UNKNOWN}: keine Antwort ({}). Die Aktion kann trotzdem angekommen sein; Liste prüfen, bevor du es erneut versuchst", t.message().map_or_else(|| t.kind().to_string(), str::to_string)),
        ureq::Error::Status(401, _) => "HTTP 401 (Token abgelehnt)".into(),
        ureq::Error::Status(c, _) => format!("HTTP {c}"),
        ureq::Error::Transport(t) => format!("transport: {}", t.message().map_or_else(|| t.kind().to_string(), str::to_string)),
    }
}

impl std::fmt::Debug for Mcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mcp").field("url", &self.url).field("tok", &"<redacted>").finish_non_exhaustive()
    }
}

/// Tools that only read (or are idempotent): safe to repeat after an ambiguous failure.
pub fn is_read_only(name: &str) -> bool {
    name.starts_with("read_") || name.starts_with("list_") || matches!(name, "status" | "download_file" | "mark_channel_read" | "quests_view" | "wiki_get" | "wiki_resolve" | "resolve_upload" | "wiki_list" | "wiki_search" | "wiki_links" | "wiki_neighbors" | "wiki_props" | "wiki_health" | "wiki_changes")
}

pub const READ_TIMEOUT: Duration = Duration::from_secs(20);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(120);

/// Marker in the error of a write whose outcome is unknown (sent, no answer): UIs must not show it as "rejected".
pub const UNKNOWN: &str = "Status unbekannt";

pub fn is_unknown(err: &str) -> bool { err.starts_with(UNKNOWN) }

/// Retry with a fresh session? Writes only for the exact expired-session case (`HTTP 404`: the daemon did not process the request,
/// re-init is safe); a tool error or HTTP 400 that merely mentions "session" never retries a write.
/// Read-only tools also retry on 400/"session"/5xx/transport/unreadable replies.
pub fn should_retry(name: &str, err: &str) -> bool {
    if err.starts_with("HTTP 404") { return true }
    if !is_read_only(name) { return false }
    err.starts_with("HTTP") || err.starts_with("transport:") || err.contains("Antwort") || err.to_lowercase().contains("session")
}

impl Mcp {
    pub fn new() -> Result<Self, String> { Self::resolve(None, None) }

    /// Explicit values win, then env MOLT_URL / MOLT_TOKEN, then ~/.config/molt/{url,token}, then the default URL.
    pub fn resolve(url: Option<String>, tok: Option<String>) -> Result<Self, String> {
        let home = std::env::var("HOME").unwrap_or_default();
        let rd = |env: &str, f: &str| std::env::var(env).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
            .or_else(|| std::fs::read_to_string(format!("{home}/.config/molt/{f}")).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()));
        let url = url.filter(|v| !v.is_empty()).or_else(|| rd("MOLT_URL", "url")).unwrap_or_else(|| "http://127.0.0.1:4040/mcp".into());
        let tok = tok.map(|t| t.trim().to_string()).filter(|v| !v.is_empty()).or_else(|| rd("MOLT_TOKEN", "token"))
            .ok_or_else(|| "Token fehlt: MOLT_TOKEN oder ~/.config/molt/token (oder --token-file)".to_string())?;
        Ok(Self::with(url, tok))
    }

    pub fn with(url: String, tok: String) -> Self {
        let ag = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .timeout(READ_TIMEOUT)
            .build();
        Mcp { url, tok, ag, sid: Mutex::new(None), rt: READ_TIMEOUT, wt: WRITE_TIMEOUT }
    }

    /// Override the per-request timeouts (reads, writes).
    pub fn timeouts(mut self, rt: Duration, wt: Duration) -> Self { self.rt = rt; self.wt = wt; self }

    fn post(&self, body: &Value, sid: Option<&str>, write: bool) -> Result<(Option<String>, String), String> {
        let mut r = self.ag.post(&self.url)
            .timeout(if write { self.wt } else { self.rt })
            .set("Authorization", &format!("Bearer {}", self.tok))
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream");
        if let Some(s) = sid { r = r.set("Mcp-Session-Id", s) }
        let resp = r.send_string(&body.to_string()).map_err(|e| emsg(e, write))?;
        let sid = resp.header("Mcp-Session-Id").map(str::to_string);
        let txt = resp.into_string().map_err(|e| if write { format!("{UNKNOWN}: Antwort abgebrochen ({e}). Liste prüfen, bevor du es erneut versuchst") } else { e.to_string() })?;
        Ok((sid, txt))
    }

    fn init(&self) -> Result<Option<String>, String> {
        let (sid, _) = self.post(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"molt-client","version":"1"}}}), None, false)?;
        self.post(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}), sid.as_deref(), false)?;
        Ok(sid)
    }

    fn once(&self, name: &str, args: &Value, fresh: bool) -> Result<Value, String> {
        let sid = {
            let mut g = self.sid.lock().unwrap();
            if fresh || g.is_none() { *g = Some(self.init()?.unwrap_or_default()) }
            g.clone().unwrap()
        };
        let sid = (!sid.is_empty()).then_some(sid);
        let (_, txt) = self.post(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":name,"arguments":args}}), sid.as_deref(), !is_read_only(name))?;
        let v = parse_sse(&txt).ok_or("Antwort nicht lesbar")?;
        tool_result(&v)
    }

    pub fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        match self.once(name, &args, false) {
            Err(e) if should_retry(name, &e) => {
                *self.sid.lock().unwrap() = None;
                self.once(name, &args, true)
            }
            r => r,
        }
    }
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn sse() {
        let b = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"{\\\"a\\\":1}\"}]}}\n\n";
        let v = parse_sse(b).unwrap();
        assert_eq!(tool_result(&v).unwrap()["a"], 1);
    }
    #[test]
    fn sse_multi_and_plain() {
        let b = "data: {\"jsonrpc\":\"2.0\",\"method\":\"ping\"}\n\ndata: {\"id\":1,\ndata: \"result\":{\"content\":[{\"text\":\"hi\"}]}}\r\n\r\n";
        let v = parse_sse(b).unwrap();
        assert_eq!(tool_result(&v).unwrap(), Value::String("hi".into()));
        assert!(parse_sse("{\"result\":{\"content\":[]}}").is_some());
        assert!(parse_sse("garbage").is_none());
    }
    #[test]
    fn retry_rules() {
        assert!(should_retry("chat_send", "HTTP 404"));
        assert!(!should_retry("chat_send", "Session expired") && !should_retry("propose", "HTTP 400") && !should_retry("wiki_edit", "session not found"));
        assert!(should_retry("read_state", "HTTP 400") && should_retry("wiki_get", "bad session"));
        for w in ["propose", "wiki_edit", "approve", "decline", "withdraw", "chat_send", "share_file"] { assert!(should_retry(w, "HTTP 404"), "{w}") }
        assert!(!should_retry("chat_send", "HTTP 502"));
        assert!(!should_retry("share_file", "transport: timed out"));
        assert!(!should_retry("chat_send", "Antwort nicht lesbar"));
        assert!(should_retry("read_state", "HTTP 502") && should_retry("status", "transport: reset"));
        assert!(!should_retry("read_state", "Token falsch"));
    }
    #[test]
    fn write_timeout_is_unknown_not_rejected() {
        use std::io::Read;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || for c in l.incoming().flatten() { std::thread::spawn(move || { let mut c = c; let mut b = [0u8; 8192]; let _ = c.read(&mut b); std::thread::sleep(Duration::from_secs(3)); }); });
        let m = Mcp::with(format!("http://127.0.0.1:{port}/mcp"), "T".into()).timeouts(Duration::from_millis(300), Duration::from_millis(300));
        // init itself hangs (read timeout) -> plain transport error, nothing was sent yet
        let e = m.call("propose", json!({})).unwrap_err();
        assert!(!is_unknown(&e) && !should_retry("propose", &e), "{e}");
        // an established session whose write call hangs: outcome unknown
        *m.sid.lock().unwrap() = Some("S".into());
        let e = m.call("propose", json!({})).unwrap_err();
        assert!(is_unknown(&e) && e.contains("Liste prüfen") && !e.to_lowercase().contains("abgelehnt"), "{e}");
        assert!(!should_retry("propose", &e));
    }
    #[test]
    fn debug_hides_token() {
        let m = Mcp::with("http://x/mcp".into(), "SECRET".into());
        assert!(!format!("{m:?}").contains("SECRET"));
    }
    #[test]
    fn errors() {
        assert_eq!(tool_result(&json!({"error":{"message":"boom"}})).unwrap_err(), "boom");
        let e = tool_result(&json!({"result":{"isError":true,"content":[{"text":"nope"}]}})).unwrap_err();
        assert_eq!(e, "nope");
    }

    #[test]
    fn mock_server_session_flow() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let mut seen = vec![];
            for _ in 0..3 {
                let (mut c, _) = l.accept().unwrap();
                let mut buf = vec![];
                let mut tmp = [0u8; 4096];
                let (head, body) = loop {
                    let n = c.read(&mut tmp).unwrap();
                    buf.extend_from_slice(&tmp[..n]);
                    let t = String::from_utf8_lossy(&buf).to_string();
                    if let Some(p) = t.find("\r\n\r\n") {
                        let cl = t.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                        if t.len() >= p + 4 + cl { break (t[..p].to_string(), t[p + 4..].to_string()) }
                    }
                };
                let m: Value = serde_json::from_str(&body).unwrap();
                let (extra, resp) = match m["method"].as_str().unwrap() {
                    "initialize" => ("Mcp-Session-Id: S1\r\n", String::new()),
                    "notifications/initialized" => ("", String::new()),
                    _ => ("", format!("event: message\ndata: {}\n\n", json!({"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"ok\":true}"}]}}))),
                };
                let st = if resp.is_empty() { "202 Accepted" } else { "200 OK" };
                write!(c, "HTTP/1.1 {st}\r\n{extra}Content-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp}", resp.len()).unwrap();
                seen.push((head, m));
            }
            seen
        });
        let m = Mcp { url: format!("http://127.0.0.1:{port}/mcp"), tok: "T0K".into(), ag: ureq::agent(), sid: Mutex::new(None), rt: READ_TIMEOUT, wt: WRITE_TIMEOUT };
        let r = m.call("chat_send", json!({"body":"hi","channel":{"kind":"topic","name":"x"}})).unwrap();
        assert_eq!(r["ok"], true);
        let seen = h.join().unwrap();
        assert!(seen[2].0.contains("Mcp-Session-Id: S1") || seen[2].0.to_lowercase().contains("mcp-session-id: s1"));
        assert!(seen[0].0.contains("Bearer T0K"));
        assert_eq!(seen[2].1["params"]["name"], "chat_send");
        assert_eq!(seen[2].1["params"]["arguments"]["channel"]["name"], "x");
    }
}
