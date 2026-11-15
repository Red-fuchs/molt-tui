//! Poke flow: chat_send first, then fire-and-forget pokes for @mentions. A poke failure never fails the chat.
use crate::mcp::Mcp;
use crate::mention::{extract, Roster};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Receiver reacts at most once per sender per minute; we match that.
pub const LIMIT_SECS: u64 = 60;
/// Pokes per message without explicit confirmation.
pub const MAX_PER_MSG: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PokeResult { Sent, RateLimited(u64), Disabled, Offline, Failed(String) }

#[derive(Default, Debug)]
pub struct PokeGate { last: HashMap<String, u64> }
impl PokeGate {
    /// Ok = allowed (and recorded); Err(secs) = wait that long. `now` in unix seconds.
    pub fn take(&mut self, target: &str, now: u64) -> Result<(), u64> {
        let k = target.to_lowercase();
        if let Some(&t) = self.last.get(&k) {
            let el = now.saturating_sub(t);
            if el < LIMIT_SECS { return Err(LIMIT_SECS - el) }
        }
        self.last.insert(k, now);
        Ok(())
    }
}

pub fn poke_enabled(m: &Mcp) -> bool {
    m.call("read_session", json!({})).ok().and_then(|v| v.pointer("/settings/poke_enabled").and_then(Value::as_bool)).unwrap_or(false)
}

/// Poke one member (roster decides Offline; gate decides RateLimited).
pub fn poke_one(m: &Mcp, gate: &mut PokeGate, roster: &Roster, enabled: bool, target: &str, now: u64) -> PokeResult {
    if !enabled { return PokeResult::Disabled }
    if roster.iter().find(|r| r.name == target).is_some_and(|r| r.presence == 0) { return PokeResult::Offline }
    if let Err(s) = gate.take(target, now) { return PokeResult::RateLimited(s) }
    match m.call("poke", json!({"member": target})) { Ok(_) => PokeResult::Sent, Err(e) => PokeResult::Failed(e) }
}

/// Targets for a message: mentions minus own seat, deduped (extract dedupes), capped; second value = skipped by the cap.
pub fn targets(body: &str, roster: &Roster, me: &str, confirmed_all: bool) -> (Vec<String>, usize) {
    let mut v: Vec<String> = extract(body, roster).into_iter().filter(|n| !n.eq_ignore_ascii_case(me) && n != me).collect();
    let skipped = if confirmed_all || v.len() <= MAX_PER_MSG { 0 } else { v.len() - MAX_PER_MSG };
    if skipped > 0 { v.truncate(MAX_PER_MSG) }
    (v, skipped)
}

pub struct Outcome { pub sent: Result<Value, String>, pub pokes: Vec<(String, PokeResult)>, pub skipped: usize }

/// Pokes for the @mentions of an already sent `body`; returns (results, skipped by the cap).
#[allow(clippy::too_many_arguments)]
pub fn poke_mentions(m: &Mcp, gate: &mut PokeGate, roster: &Roster, me: &str, enabled: bool, confirmed_all: bool, body: &str, now: u64) -> (Vec<(String, PokeResult)>, usize) {
    let (ts, skipped) = targets(body, roster, me, confirmed_all);
    (ts.into_iter().map(|t| { let r = poke_one(m, gate, roster, enabled, &t, now); (t, r) }).collect(), skipped)
}

/// chat_send, THEN pokes. `args` = chat_send arguments (body inside is what gets scanned).
#[allow(clippy::too_many_arguments)]
pub fn send_with_pokes(m: &Mcp, gate: &mut PokeGate, roster: &Roster, me: &str, enabled: bool, confirmed_all: bool, args: Value, now: u64) -> Outcome {
    let body = args.get("body").and_then(Value::as_str).unwrap_or("").to_string();
    let sent = m.call("chat_send", args);
    if sent.is_err() { return Outcome { sent, pokes: vec![], skipped: 0 } }
    let (pokes, skipped) = poke_mentions(m, gate, roster, me, enabled, confirmed_all, &body, now);
    Outcome { sent, pokes, skipped }
}

/// Short German status for toasts; None when nothing worth saying.
pub fn summary_de(o: &Outcome) -> Option<String> {
    let mut p: Vec<String> = o.pokes.iter().map(|(n, r)| match r {
        PokeResult::Sent => format!("{n} angestupst"),
        PokeResult::RateLimited(s) => format!("{n}: Stups erst in {s}s"),
        PokeResult::Disabled => format!("{n}: Stupsen ist aus"),
        PokeResult::Offline => format!("{n} offline, kein Stups"),
        PokeResult::Failed(e) => format!("{n}: Stups fehlgeschlagen ({})", crate::text::one(e)),
    }).collect();
    if o.skipped > 0 { p.push(format!("{} weitere Erwähnung(en) ohne Stups", o.skipped)) }
    if p.is_empty() { None } else { Some(p.join("; ")) }
}

#[cfg(test)]
mod t {
    use super::*;
    use crate::mock::{sim::Sim, start};
    use crate::model::Member;
    fn mem(n: &str, p: u64) -> Member { Member { name: n.into(), presence: p, ..Default::default() } }
    fn setup() -> (crate::mock::Mock, Mcp, std::sync::Arc<std::sync::Mutex<Sim>>) {
        let (h, st) = Sim::new("me").into_handler(None);
        let mk = start("T", h);
        let m = Mcp::with(mk.url.clone(), "T".into());
        (mk, m, st)
    }
    #[test]
    fn gate() {
        let mut g = PokeGate::default();
        assert_eq!(g.take("Bob", 100), Ok(()));
        assert_eq!(g.take("bob", 130), Err(30));
        assert_eq!(g.take("carol", 130), Ok(()));
        assert_eq!(g.take("bob", 160), Ok(()));
    }
    #[test]
    fn chat_first_then_pokes_dedupe_no_self() {
        let (mk, m, _) = setup();
        let r = vec![mem("bob", 1), mem("carol", 1), mem("me", 1)];
        let o = send_with_pokes(&m, &mut PokeGate::default(), &r, "me", true, false, json!({"body":"@bob @{bob} @me @carol hi"}), 1000);
        assert!(o.sent.is_ok());
        assert_eq!(o.pokes, vec![("bob".into(), PokeResult::Sent), ("carol".into(), PokeResult::Sent)]);
        let order: Vec<String> = mk.calls.lock().unwrap().iter().map(|c| c.0.clone()).filter(|n| n == "chat_send" || n == "poke").collect();
        assert_eq!(order, ["chat_send", "poke", "poke"]);
    }
    #[test]
    fn cap_and_confirm() {
        let r = vec![mem("a", 1), mem("b", 1), mem("c", 1)];
        assert_eq!(targets("@a @b @c", &r, "me", false), (vec!["a".into(), "b".into()], 1));
        assert_eq!(targets("@a @b @c", &r, "me", true).0.len(), 3);
    }
    #[test]
    fn results_disabled_offline_ratelimit_failure() {
        let (_, m, st) = setup();
        let r = vec![mem("bob", 1), mem("carol", 0)];
        let mut g = PokeGate::default();
        assert_eq!(poke_one(&m, &mut g, &r, false, "bob", 1), PokeResult::Disabled);
        assert_eq!(poke_one(&m, &mut g, &r, true, "carol", 1), PokeResult::Offline);
        assert_eq!(poke_one(&m, &mut g, &r, true, "bob", 1), PokeResult::Sent);
        assert_eq!(poke_one(&m, &mut g, &r, true, "bob", 20), PokeResult::RateLimited(41));
        st.lock().unwrap().poke_enabled = false; // daemon refuses -> Failed, chat unaffected
        let o = send_with_pokes(&m, &mut PokeGate::default(), &r, "me", true, false, json!({"body":"@bob"}), 500);
        assert!(o.sent.is_ok() && matches!(o.pokes[0].1, PokeResult::Failed(_)));
        assert!(!poke_enabled(&m));
    }
    #[test]
    fn chat_failure_skips_pokes() {
        let (mk, m, _) = setup();
        let o = send_with_pokes(&m, &mut PokeGate::default(), &[mem("bob", 1)], "me", true, false, json!({"body":"  "}), 1);
        assert!(o.sent.is_err() && o.pokes.is_empty() && mk.called("poke").is_empty());
    }
}
