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

/// Stateful in-memory republic behind a `Handler`: chat, topics, proposals, votes, withdraw, wiki_edit.
/// Mirrors the daemon's refusals closely enough to test write paths end to end; seeded with neutral data.
pub mod sim {
    use super::Handler;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    pub struct Sim {
        pub seat: String, pub members: Vec<String>, pub threshold: u64, pub msgs: Vec<Value>, pub props: Vec<Value>,
        pub uploads: Vec<Value>, pub pages: Vec<String>, pub blobs: Vec<(String, Vec<u8>)>, n: u64, next_prop: u64,
    }

    fn hex_id(n: u64) -> String { format!("{n:032x}") }
    fn s<'a>(v: &'a Value, k: &str) -> &'a str { v.get(k).and_then(Value::as_str).unwrap_or("") }

    impl Sim {
        /// Seat `seat`, members bob + carol, threshold 2; proposal 4 (open, by bob) and 1 (applied) exist.
        pub fn new(seat: &str) -> Sim {
            let mut m = Sim { seat: seat.into(), members: vec!["bob".into(), "carol".into(), seat.into()], threshold: 2, msgs: vec![], props: vec![],
                uploads: vec![], pages: vec!["home.md".into()], blobs: vec![], n: 100, next_prop: 5 };
            m.props.push(json!({"id":1,"state":"applied","by":"bob","surface":"memory","payload":{"op":"add_note","title":"Alt"},"approvals":2,"threshold":2,
                "votes":[{"member":"bob","vote":"approved"},{"member":"carol","vote":"approved"}],"approved_by_me":false,"declined_by_me":false}));
            m.props.push(json!({"id":4,"state":"proposed","by":"bob","surface":"memory","payload":{"op":"wiki_patch","summary":"+1","value":"diff --git a/neu.md b/neu.md\nnew file mode 100644\n--- /dev/null\n+++ b/neu.md\n@@ -0,0 +1 @@\n+Hallo\n"},
                "approvals":1,"threshold":2,"current":"","proposed":"Hallo\n","votes":[{"member":"bob","vote":"approved"},{"member":"carol","vote":"open"},{"member":seat,"vote":"open"}],
                "approved_by_me":false,"declined_by_me":false}));
            m.say("bob", "Hallo zusammen", None, None);
            m
        }
        pub fn into_handler(self, inner: Option<Handler>) -> (Handler, Arc<Mutex<Sim>>) {
            let st = Arc::new(Mutex::new(self));
            let st2 = st.clone();
            let h: Handler = Arc::new(move |name, a| match st2.lock().unwrap().call(name, a) {
                Some(r) => r,
                None => match &inner { Some(i) => i(name, a), None => Err(format!("unexpected tool {name}")) },
            });
            (h, st)
        }

        fn say(&mut self, from: &str, body: &str, chan: Option<Value>, quote: Option<&str>) -> String {
            self.n += 1;
            let id = hex_id(self.n);
            let mut m = json!({"id":id,"from":from,"ts":1_794_000_000 + self.n as i64,"kind":"user","body":body});
            if let Some(c) = chan { m["channel"] = c }
            if let Some(q) = quote { m["quote"] = json!(q) }
            self.msgs.push(m);
            id
        }
        /// A file share in the group chat: bytes served by `download_file`. `checksum` None = real sha256 of `bytes`.
        pub fn say_file(&mut self, from: &str, name: &str, kind: &str, bytes: &[u8], checksum: Option<&str>) -> String {
            let sum = checksum.map(str::to_string).unwrap_or_else(|| { use sha2::Digest; sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect() });
            let id = self.say(from, "", None, None);
            if let Some(m) = self.msgs.last_mut() {
                m["kind"] = json!("file");
                m["file"] = json!({"name":name,"kind":kind,"size":bytes.len(),"checksum":sum});
            }
            self.uploads.push(json!({"id":id,"local":"none","available":true,"online":true}));
            self.blobs.push((id.clone(), bytes.to_vec()));
            id
        }
        fn prop_mut(&mut self, id: u64) -> Result<&mut Value, String> {
            self.props.iter_mut().find(|p| p["id"] == id).ok_or_else(|| format!("unknown proposal {id}"))
        }
        fn open(&mut self, id: u64) -> Result<&mut Value, String> {
            let p = self.prop_mut(id)?;
            if p["state"] != "proposed" { return Err(format!("proposal {id} is not pending")) }
            Ok(p)
        }
        fn note(&mut self, id: u64, a: &Value) {
            if let Some(n) = a.get("note").and_then(Value::as_str).filter(|n| !n.trim().is_empty()) {
                let seat = self.seat.clone();
                self.say(&seat, n, Some(json!({"kind":"patch","id":id})), None);
            }
        }
        fn mint(&mut self, surface: &str, payload: Value, extra: Value) -> Value {
            let id = self.next_prop;
            self.next_prop += 1;
            let seat = self.seat.clone();
            let mut votes: Vec<Value> = self.members.iter().map(|m| json!({"member":m,"vote":if *m == seat {"approved"} else {"open"}})).collect();
            votes.sort_by_key(|v| s(v, "member").to_string());
            let mut p = json!({"id":id,"state":"proposed","by":seat,"surface":surface,"payload":payload,"approvals":1,"threshold":self.threshold,
                "votes":votes,"approved_by_me":true,"declined_by_me":false,"mine":true});
            if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) { for (k, v) in e { o.insert(k.clone(), v.clone()); } }
            self.props.push(p);
            json!({"id":id,"state":"proposed","approvals":1,"threshold":self.threshold,"channel":{"kind":"patch","id":id}})
        }
        fn vote(&mut self, id: u64, ok: bool, a: &Value) -> Result<Value, String> {
            let (seat, thr, n) = (self.seat.clone(), self.threshold, self.members.len() as u64);
            { let p = self.open(id)?;
              if p["approved_by_me"] == true || p["declined_by_me"] == true { return Err("this seat already voted".into()) } }
            self.note(id, a);
            let p = self.open(id)?;
            for v in p["votes"].as_array_mut().into_iter().flatten() { if s(v, "member") == seat { v["vote"] = json!(if ok { "approved" } else { "declined" }) } }
            p[if ok { "approved_by_me" } else { "declined_by_me" }] = json!(true);
            let cnt = |p: &Value, w: &str| p["votes"].as_array().map_or(0, |v| v.iter().filter(|x| s(x, "vote") == w).count() as u64);
            let (ap, de) = (cnt(p, "approved"), cnt(p, "declined"));
            p["approvals"] = json!(ap);
            if ap >= thr { p["state"] = json!("applied") } else if de > n - thr { p["state"] = json!("rejected") }
            Ok(json!({"id":id,"state":p["state"],"approvals":ap,"threshold":thr,"channel":{"kind":"patch","id":id}}))
        }

        /// None = not a tool this sim knows.
        pub fn call(&mut self, name: &str, a: &Value) -> Option<Result<Value, String>> {
            Some(match name {
                "status" => Ok(json!({"name":"Demo Republic","member":self.seat,"members":self.members,"chat_retention_days":7,"relays":["ws://a"],"features":["memory"]})),
                "read_members" => Ok(json!({"members":self.members.iter().map(|m| json!({"member":m,"presence":0})).collect::<Vec<_>>()})),
                "read_uploads" => Ok(json!({"uploads":self.uploads})),
                "download_file" => (|| {
                    let id = s(a, "id");
                    let dest = s(a, "dest");
                    let bytes = &self.blobs.iter().find(|(i, _)| i == id).ok_or("unknown share")?.1;
                    std::fs::write(dest, bytes).map_err(|e| e.to_string())?;
                    if let Some(u) = self.uploads.iter_mut().find(|u| u["id"] == id) { u["local"] = json!("downloaded") }
                    Ok(json!({"ok":true}))
                })(),
                "read_actions" => Ok(json!({"actions":self.props.iter().filter(|p| p["state"] == "proposed" && p["approved_by_me"] != true && p["declined_by_me"] != true)
                    .map(|p| json!({"kind":"vote","proposal":p["id"]})).collect::<Vec<_>>()})),
                "list_proposals" => Ok(json!({"proposals":self.props})),
                "read_proposal" => a["id"].as_u64().ok_or("id fehlt".to_string()).and_then(|i| self.prop_mut(i).map(|p| p.clone())),
                "mark_channel_read" => Ok(json!({"ok":true})),
                "read_state" => {
                    let mut ch: Vec<Value> = vec![]; let mut seen: Vec<Value> = vec![];
                    for m in &self.msgs { let c = m.get("channel").cloned().unwrap_or(json!({"kind":"group"})); if !seen.contains(&c) { seen.push(c) } }
                    if !seen.contains(&json!({"kind":"group"})) { seen.insert(0, json!({"kind":"group"})) }
                    for p in &self.props { let c = json!({"kind":"patch","id":p["id"]}); if !seen.contains(&c) { seen.push(c) } }
                    for c in seen {
                        let n = self.msgs.iter().filter(|m| m.get("channel").cloned().unwrap_or(json!({"kind":"group"})) == c).count();
                        let last = self.msgs.iter().filter(|m| m.get("channel").cloned().unwrap_or(json!({"kind":"group"})) == c).map(|m| m["ts"].as_i64().unwrap_or(0)).max().unwrap_or(0);
                        ch.push(json!({"channel":c,"count":n,"unread":0,"last_ts":last}));
                    }
                    Ok(json!({"wiki_rev":1,"channels":ch,"applied":self.msgs}))
                }
                "chat_send" => {
                    let body = s(a, "body");
                    if body.trim().is_empty() { return Some(Err("empty body".into())) }
                    let c = a.get("channel").cloned();
                    if let Some(c) = &c { match s(c, "kind") {
                        "topic" => { let n = s(c, "name"); if n.trim().is_empty() || n.chars().count() > 64 || n != n.trim() { return Some(Err("invalid topic name".into())) } }
                        "patch" => { if !c["id"].as_u64().is_some_and(|i| self.props.iter().any(|p| p["id"] == i)) { return Some(Err("unknown proposal channel".into())) } }
                        "group" => {}
                        k => return Some(Err(format!("unknown channel kind {k}"))),
                    } }
                    let seat = self.seat.clone();
                    let id = self.say(&seat, body, c, a.get("quote").and_then(Value::as_str));
                    Ok(json!({"id":id}))
                }
                "approve" | "decline" => match a["proposal_id"].as_u64() { Some(i) => self.vote(i, name == "approve", a), None => Err("proposal_id fehlt".into()) },
                "withdraw" => match a["proposal_id"].as_u64() {
                    None => Err("proposal_id fehlt".into()),
                    Some(i) => (|| {
                        let seat = self.seat.clone();
                        if s(self.open(i)?, "by") != seat { return Err("only the proposer may withdraw".to_string()) }
                        self.note(i, a);
                        let p = self.open(i)?;
                        p["state"] = json!("withdrawn");
                        Ok(json!({"id":i,"state":"withdrawn","channel":{"kind":"patch","id":i}}))
                    })(),
                },
                "propose" => {
                    let (sf, p) = (s(a, "surface"), &a["payload"]);
                    let op = s(p, "op");
                    let val = s(p, "value");
                    let ok = match (sf, op) {
                        ("organization", "set_name" | "set_charter") => !val.trim().is_empty(),
                        ("organization", "set_chat_retention") => val.parse::<u32>().is_ok_and(|d| d > 0),
                        ("organization", "set_relays") => val.split_whitespace().next().is_some_and(|u| u.starts_with("ws")),
                        ("organization", "set_features") => !val.trim().is_empty() && val.split_whitespace().all(|k| ["memory", "quests", "vault", "wallet"].contains(&k)),
                        ("memory", "add_note") => !s(p, "title").trim().is_empty(),
                        ("files", "persist") => s(p, "id").len() == 32,
                        _ => return Some(Err(format!("unknown op {sf}/{op}"))),
                    };
                    if !ok { return Some(Err(format!("invalid payload for {sf}/{op}"))) }
                    Ok(self.mint(sf, p.clone(), json!({})))
                }
                "wiki_edit" => (|| {
                    let edits = a["edits"].as_array().filter(|e| !e.is_empty()).ok_or("edits fehlen".to_string())?;
                    let (mut add, mut lines) = (0, 0);
                    let mut patch = String::new();
                    let mut pages = self.pages.clone();
                    for e in edits {
                        let (op, path, content) = (s(e, "op"), s(e, "path"), s(e, "content"));
                        if path.is_empty() || path.starts_with('/') || path.contains("..") { return Err(format!("bad path {path}")) }
                        match op {
                            "create" => { if pages.iter().any(|p| p == path) { return Err(format!("create refused: {path} already exists")) } pages.push(path.into()); add += 1 }
                            "content" => { if !pages.iter().any(|p| p == path) { pages.push(path.into()); add += 1 } }
                            o => return Err(format!("unsupported op {o}")),
                        }
                        lines += content.lines().count();
                        patch += &format!("diff --git a/{path} b/{path}\n--- /dev/null\n+++ b/{path}\n{}", content.lines().map(|l| format!("+{l}\n")).collect::<String>());
                    }
                    let summary = format!("+{add} ~{lines}");
                    if a["dry_run"] == true { return Ok(json!({"summary":summary,"warnings":[],"paths":edits.iter().map(|e| e["path"].clone()).collect::<Vec<_>>()})) }
                    self.pages = pages;
                    Ok(self.mint("memory", json!({"op":"wiki_patch","summary":summary,"value":patch}), json!({"paths":edits.iter().map(|e| e["path"].clone()).collect::<Vec<_>>()})))
                })(),
                _ => return None,
            })
        }
    }
}

#[cfg(test)]
mod sim_t {
    use super::sim::Sim;
    use serde_json::json;
    #[test]
    fn propose_vote_withdraw_topics() {
        let mut m = Sim::new("me");
        let c = |m: &mut Sim, n: &str, a: serde_json::Value| m.call(n, &a).unwrap();
        let r = c(&mut m, "propose", json!({"surface":"memory","payload":{"op":"add_note","title":"T"}})).unwrap();
        assert_eq!(r["id"], 5);
        assert!(c(&mut m, "propose", json!({"surface":"organization","payload":{"op":"set_name","value":""}})).is_err());
        assert!(c(&mut m, "propose", json!({"surface":"organization","payload":{"op":"bogus"}})).is_err());
        assert!(c(&mut m, "withdraw", json!({"proposal_id":4})).unwrap_err().contains("proposer"));
        assert_eq!(c(&mut m, "withdraw", json!({"proposal_id":5,"note":"[x] weg"})).unwrap()["state"], "withdrawn");
        assert!(c(&mut m, "approve", json!({"proposal_id":5})).is_err());
        assert_eq!(c(&mut m, "approve", json!({"proposal_id":4,"note":"ok"})).unwrap()["state"], "applied");
        assert!(c(&mut m, "decline", json!({"proposal_id":4})).is_err());
        let st = c(&mut m, "read_state", json!({"surface":"chat"})).unwrap();
        assert!(st["applied"].as_array().unwrap().iter().any(|x| x["channel"]["id"] == 4 && x["body"] == "ok"));
        c(&mut m, "chat_send", json!({"body":"hi","channel":{"kind":"topic","name":"Neu"}})).unwrap();
        assert!(c(&mut m, "chat_send", json!({"body":"hi","channel":{"kind":"topic","name":" "}})).is_err());
        let st = c(&mut m, "read_state", json!({"surface":"chat"})).unwrap();
        assert!(st["channels"].as_array().unwrap().iter().any(|x| x["channel"]["name"] == "Neu"));
        assert!(c(&mut m, "wiki_edit", json!({"edits":[{"op":"create","path":"home.md","content":"x"}]})).unwrap_err().contains("already exists"));
        assert_eq!(c(&mut m, "wiki_edit", json!({"edits":[{"op":"create","path":"a.md","content":"x\ny"}],"dry_run":true})).unwrap()["summary"], "+1 ~2");
        let r = c(&mut m, "wiki_edit", json!({"edits":[{"op":"create","path":"a.md","content":"x"}]})).unwrap();
        assert_eq!(r["channel"]["kind"], "patch");
    }

}
