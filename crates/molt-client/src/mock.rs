//! Minimal in-process MCP-over-HTTP mock for tests (never touches a real moltd).
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub type Calls = Arc<Mutex<Vec<(String, Value)>>>;
pub type Handler = Arc<dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync>;

pub struct Mock { pub url: String, pub token: String, pub calls: Calls }

impl Mock {
    /// Tool names (with args) received so far.
    pub fn called(&self, name: &str) -> Vec<Value> {
        self.calls.lock().unwrap().iter().filter(|(n, _)| n == name).map(|(_, a)| a.clone()).collect()
    }
}

/// Start a mock; `h(tool, args)` returns the tool's JSON result or an error text. Requires `Authorization: Bearer <token>`.
pub fn start(token: &str, h: Handler) -> Mock {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let calls: Calls = Default::default();
    let (c2, tok) = (calls.clone(), token.to_string());
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            let (h, c2, tok) = (h.clone(), c2.clone(), tok.clone());
            std::thread::spawn(move || { let _ = serve(c, &tok, h, c2); });
        }
    });
    Mock { url: format!("http://127.0.0.1:{port}/mcp"), token: token.into(), calls }
}

fn serve(mut c: std::net::TcpStream, tok: &str, h: Handler, calls: Calls) -> std::io::Result<()> {
    let mut buf = vec![];
    let mut tmp = [0u8; 8192];
    let (head, body) = loop {
        let n = c.read(&mut tmp)?;
        if n == 0 { return Ok(()) }
        buf.extend_from_slice(&tmp[..n]);
        let t = String::from_utf8_lossy(&buf).to_string();
        if let Some(p) = t.find("\r\n\r\n") {
            let cl = t.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
            if buf.len() >= p + 4 + cl { break (t[..p].to_string(), String::from_utf8_lossy(&buf[p + 4..p + 4 + cl]).to_string()) }
        }
    };
    let send = |c: &mut std::net::TcpStream, st: &str, extra: &str, b: &str| {
        write!(c, "HTTP/1.1 {st}\r\n{extra}Content-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len())
    };
    if !head.contains(&format!("Bearer {tok}")) { return send(&mut c, "401 Unauthorized", "", "") }
    let m: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    match m["method"].as_str().unwrap_or("") {
        "initialize" => send(&mut c, "200 OK", "Mcp-Session-Id: MOCK\r\n", &format!("data: {}\n\n", json!({"jsonrpc":"2.0","id":0,"result":{}}))),
        "tools/call" => {
            let (n, a) = (m["params"]["name"].as_str().unwrap_or("").to_string(), m["params"]["arguments"].clone());
            calls.lock().unwrap().push((n.clone(), a.clone()));
            let res = match h(&n, &a) {
                Ok(v) => json!({"content":[{"type":"text","text":v.to_string()}]}),
                Err(e) => json!({"isError":true,"content":[{"type":"text","text":e}]}),
            };
            send(&mut c, "200 OK", "", &format!("event: message\ndata: {}\n\n", json!({"jsonrpc":"2.0","id":1,"result":res})))
        }
        _ => send(&mut c, "202 Accepted", "", ""),
    }
}
