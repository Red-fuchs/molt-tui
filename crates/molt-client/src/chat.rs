//! Chat write actions beyond chat_send: reactions, delete, topic lifecycle, withdraw a share, read receipts.
//! Every function validates locally first; callers own the confirmation step (these are real writes).
use crate::mcp::Mcp;
use crate::model::Msg;
use serde_json::{json, Value};

/// Short list offered by the UIs (any short emoji is accepted by the daemon).
pub const REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "🎉", "🙏", "👀"];

/// Message id as moltd hands it out: 32 lowercase hex chars.
pub fn valid_id(id: &str) -> bool { id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }

/// A single short emoji-ish token: non-empty, <= 16 bytes, no whitespace/control chars, not plain ASCII text.
pub fn valid_emoji(e: &str) -> bool { !e.is_empty() && e.len() <= 16 && !e.chars().any(|c| c.is_whitespace() || c.is_control()) && !e.is_ascii() }

fn id(i: &str) -> Result<(), String> { if valid_id(i) { Ok(()) } else { Err("ungültige Nachrichten-ID".into()) } }

pub fn react(m: &Mcp, i: &str, emoji: &str) -> Result<Value, String> {
    id(i)?;
    if !valid_emoji(emoji) { return Err("ungültiges Emoji".into()) }
    m.call("react_chat", json!({"id":i,"emoji":emoji}))
}

pub fn delete(m: &Mcp, i: &str) -> Result<Value, String> { id(i)?; m.call("delete_chat", json!({"id":i})) }
pub fn remove_file(m: &Mcp, i: &str) -> Result<Value, String> { id(i)?; m.call("remove_file", json!({"id":i})) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thread { Close, Reopen, Forget }
impl Thread {
    pub fn tool(self) -> &'static str { match self { Thread::Close => "thread_close", Thread::Reopen => "thread_reopen", Thread::Forget => "thread_forget" } }
}

pub fn thread(m: &Mcp, op: Thread, name: &str) -> Result<Value, String> {
    if name.trim().is_empty() || name.trim() != name || name.chars().count() > 64 { return Err("ungültiger Themenname".into()) }
    m.call(op.tool(), json!({"name":name}))
}

/// Read receipts for exactly these messages (own/unknown/already-read ids are ignored by the daemon). Invalid ids are dropped; empty = no call.
pub fn mark_read(m: &Mcp, ids: &[&str]) -> Result<Value, String> {
    let v: Vec<&str> = ids.iter().copied().filter(|i| valid_id(i)).take(200).collect();
    if v.is_empty() { return Ok(json!({"ok":true,"skipped":true})) }
    m.call("mark_read", json!({"ids":v}))
}

/// Own message by author name (the seat is shared: UIs add their own tag check on top).
pub fn is_own(msg: &Msg, seat: &str) -> bool { !seat.is_empty() && msg.from == seat && !msg.deleted && !matches!(msg.kind.as_str(), "system" | "wake") }

#[cfg(test)]
mod t {
    use super::*;
    use crate::mock::{self, sim::Sim};
    use serde_json::json;

    fn up() -> (mock::Mock, std::sync::Arc<std::sync::Mutex<Sim>>, Mcp) {
        let (h, st) = Sim::new("me").into_handler(None);
        let mk = mock::start("t", h);
        let m = Mcp::with(mk.url.clone(), "t".into());
        (mk, st, m)
    }

    #[test]
    fn validators() {
        assert!(valid_id(&"a".repeat(32)) && !valid_id("A".repeat(32).as_str()) && !valid_id("abc"));
        assert!(valid_emoji("👍") && valid_emoji("❤️") && !valid_emoji("") && !valid_emoji("ab") && !valid_emoji("a b") && !valid_emoji(&"😀".repeat(5)));
    }

    #[test]
    fn react_toggle_switch_and_local_refusals() {
        let (mk, st, m) = up();
        let mid = st.lock().unwrap().msgs[0]["id"].as_str().unwrap().to_string();
        assert!(react(&m, "zz", "👍").is_err() && react(&m, &mid, "x").is_err());
        assert!(mk.called("react_chat").is_empty());
        react(&m, &mid, "👍").unwrap();
        assert_eq!(st.lock().unwrap().msgs[0]["reactions"]["👍"], json!(["me"]));
        react(&m, &mid, "🎉").unwrap();
        let r = st.lock().unwrap().msgs[0]["reactions"].clone();
        assert!(r.get("👍").is_none() && r["🎉"] == json!(["me"]));
        react(&m, &mid, "🎉").unwrap();
        assert!(st.lock().unwrap().msgs[0]["reactions"].as_object().unwrap().is_empty());
        assert!(react(&m, &"0".repeat(32), "👍").is_err());
    }

    #[test]
    fn delete_own_only_and_tombstone() {
        let (_mk, st, m) = up();
        let bob = st.lock().unwrap().msgs[0]["id"].as_str().unwrap().to_string();
        let mine = m.call("chat_send", json!({"body":"x"})).unwrap()["id"].as_str().unwrap().to_string();
        assert!(delete(&m, &bob).unwrap_err().contains("author"));
        delete(&m, &mine).unwrap();
        let g = crate::model::parse_chat(&m.call("read_state", json!({"surface":"chat"})).unwrap()).0;
        let d = g.iter().find(|x| x.id == mine).unwrap();
        assert!(d.deleted && d.body.contains("me"));
        assert!(!is_own(d, "me"));
    }

    #[test]
    fn threads_and_forget() {
        let (mk, st, m) = up();
        m.call("chat_send", json!({"body":"hi","channel":{"kind":"topic","name":"T"}})).unwrap();
        assert!(thread(&m, Thread::Close, " T").is_err() && mk.called("thread_close").is_empty());
        thread(&m, Thread::Close, "T").unwrap();
        assert!(m.call("chat_send", json!({"body":"more","channel":{"kind":"topic","name":"T"}})).unwrap_err().contains("closed"));
        thread(&m, Thread::Reopen, "T").unwrap();
        m.call("chat_send", json!({"body":"more","channel":{"kind":"topic","name":"T"}})).unwrap();
        thread(&m, Thread::Forget, "T").unwrap();
        assert!(st.lock().unwrap().msgs.iter().all(|x| x["channel"]["name"] != "T"));
        assert!(thread(&m, Thread::Forget, "nope").is_err());
    }

    #[test]
    fn remove_file_sharer_only() {
        let (_mk, st, m) = up();
        let (theirs, mine) = { let mut s = st.lock().unwrap(); (s.say_file("bob", "a.txt", "File", b"x", None), s.say_file("me", "b.txt", "File", b"y", None)) };
        assert!(remove_file(&m, &theirs).unwrap_err().contains("sharer"));
        remove_file(&m, &mine).unwrap();
        let u = m.call("read_uploads", json!({})).unwrap();
        let row = u["uploads"].as_array().unwrap().iter().find(|x| x["id"] == mine.as_str()).unwrap();
        assert_eq!(row["available"], false);
    }

    #[test]
    fn mark_read_only_given_ids() {
        let (mk, _st, m) = up();
        let a = "a".repeat(32);
        assert_eq!(mark_read(&m, &["bad"]).unwrap()["skipped"], true);
        assert!(mk.called("mark_read").is_empty());
        mark_read(&m, &[&a, "bad"]).unwrap();
        assert_eq!(mk.called("mark_read")[0]["ids"], json!([a]));
    }

    #[test]
    fn writes_never_retry_blindly() { assert!(!crate::mcp::is_read_only("react_chat") && !crate::mcp::is_read_only("delete_chat") && !crate::mcp::is_read_only("mark_read")) }
}
