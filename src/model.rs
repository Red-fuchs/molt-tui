use crate::mcp::Mcp;
use crate::text::{clean, one};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Chan { Group, Topic(String), Patch(u64) }

impl Chan {
    pub fn from_json(v: &Value) -> Chan {
        match v.get("kind").and_then(Value::as_str) {
            Some("patch") => Chan::Patch(v.get("id").and_then(Value::as_u64).unwrap_or(0)),
            Some("topic") => Chan::Topic(one(v.get("name").and_then(Value::as_str).unwrap_or(""))),
            _ => Chan::Group,
        }
    }
    pub fn to_json(&self) -> Option<Value> {
        match self {
            Chan::Group => None,
            Chan::Topic(n) => Some(json!({"kind":"topic","name":n})),
            Chan::Patch(i) => Some(json!({"kind":"patch","id":i})),
        }
    }
    pub fn name(&self) -> String {
        match self { Chan::Group => "Allgemein".into(), Chan::Topic(n) => n.clone(), Chan::Patch(i) => format!("Vorschlag #{i}") }
    }
}

#[derive(Clone, Debug)]
pub struct Msg {
    pub id: String, pub from: String, pub ts: i64, pub body: String, pub chan: Chan,
    pub quote: Option<String>, pub reactions: Vec<(String, usize)>,
}

#[derive(Clone, Debug)]
pub struct ChanInfo { pub chan: Chan, pub count: u64, pub unread: u64, pub last_ts: i64, pub state: Option<String> }

#[derive(Clone, Debug, Default)]
pub struct Prop {
    pub id: u64, pub state: String, pub by: String, pub surface: String, pub op: String, pub summary: String,
    pub paths: Vec<String>, pub approvals: u64, pub threshold: u64, pub votes: Vec<(String, String)>,
    pub approved_by_me: bool, pub declined_by_me: bool, pub superseded: Option<String>,
    pub value: String, pub current: String, pub proposed: String,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Act { pub kind: String, pub proposal: Option<u64> }
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Doc { pub path: String, pub title: String, pub bytes: u64 }

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub name: String, pub member: String, pub members: Vec<String>, pub active_1h: u64,
    pub relays: usize, pub missing_relays: usize, pub diverged: bool, pub lag: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Snap {
    pub st: Status, pub msgs: Vec<Msg>, pub chans: Vec<ChanInfo>, pub props: Vec<Prop>,
    pub acts: Vec<Act>, pub docs: Vec<Doc>, pub wiki_rev: u64,
}

fn s(v: &Value, k: &str) -> String { one(v.get(k).and_then(Value::as_str).unwrap_or("")) }
fn u(v: &Value, k: &str) -> u64 { v.get(k).and_then(Value::as_u64).unwrap_or(0) }
fn arr<'a>(v: &'a Value, k: &str) -> impl Iterator<Item = &'a Value> { v.get(k).and_then(Value::as_array).into_iter().flatten() }

pub fn parse_status(v: &Value) -> Status {
    Status {
        name: s(v, "name"), member: s(v, "member"),
        members: arr(v, "members").filter_map(Value::as_str).map(one).collect(),
        active_1h: u(v, "active_1h"),
        relays: arr(v, "relays").count(), missing_relays: arr(v, "missing_relays").count(),
        diverged: arr(v, "chain_diverged").count() > 0,
        lag: v.pointer("/chain_lag/peers_ahead").and_then(Value::as_u64).unwrap_or(0),
    }
}

pub fn parse_chat(v: &Value) -> (Vec<Msg>, Vec<ChanInfo>, u64) {
    let msgs = arr(v, "applied").map(|m| Msg {
        id: s(m, "id"), from: s(m, "from"), ts: m.get("ts").and_then(Value::as_i64).unwrap_or(0),
        body: clean(m.get("body").and_then(Value::as_str).unwrap_or("")),
        chan: m.get("channel").map(Chan::from_json).unwrap_or(Chan::Group),
        quote: m.get("quote").or(m.get("quoted")).and_then(Value::as_str).map(str::to_string),
        reactions: m.get("reactions").and_then(Value::as_object).into_iter().flatten()
            .map(|(k, v)| (one(k), v.as_array().map_or(0, Vec::len))).collect(),
    }).collect();
    let chans = arr(v, "channels").map(|c| ChanInfo {
        chan: c.get("channel").map(Chan::from_json).unwrap_or(Chan::Group),
        count: u(c, "count"), unread: u(c, "unread"), last_ts: c.get("last_ts").and_then(Value::as_i64).unwrap_or(0),
        state: c.get("state").and_then(Value::as_str).map(one),
    }).collect();
    (msgs, chans, u(v, "wiki_rev"))
}

pub fn parse_prop(p: &Value) -> Prop {
    let g = |k: &str| clean(p.get(k).and_then(Value::as_str).unwrap_or(""));
    Prop {
        id: u(p, "id"), state: s(p, "state"), by: s(p, "by"), surface: s(p, "surface"),
        op: one(p.pointer("/payload/op").and_then(Value::as_str).unwrap_or("")),
        summary: one(p.pointer("/payload/summary").and_then(Value::as_str).unwrap_or("")),
        paths: arr(p, "paths").filter_map(Value::as_str).map(one).collect(),
        approvals: u(p, "approvals"), threshold: u(p, "threshold"),
        votes: arr(p, "votes").map(|v| (s(v, "member"), s(v, "vote"))).collect(),
        approved_by_me: p.get("approved_by_me").and_then(Value::as_bool) == Some(true),
        declined_by_me: p.get("declined_by_me").and_then(Value::as_bool) == Some(true),
        superseded: p.get("superseded").and_then(Value::as_str).map(one),
        value: clean(p.pointer("/payload/value").and_then(Value::as_str).unwrap_or("")),
        current: g("current"), proposed: g("proposed"),
    }
}

impl Prop {
    pub fn open(&self) -> bool { self.state == "proposed" }
    pub fn todo(&self) -> bool { self.open() && !self.approved_by_me && !self.declined_by_me }
}

pub fn state_de(s: &str) -> &str {
    match s {
        "proposed" => "offen", "applied" => "angenommen", "rejected" | "declined" => "abgelehnt",
        "withdrawn" => "zurückgezogen", "superseded" => "überholt", x => x,
    }
}

pub fn parse_snap(st: &Value, chat: &Value, props: &Value, acts: &Value, wiki: &Value) -> Snap {
    let (msgs, chans, wiki_rev) = parse_chat(chat);
    Snap {
        st: parse_status(st), msgs, chans,
        props: arr(props, "proposals").map(parse_prop).collect(),
        acts: arr(acts, "actions").map(|a| Act { kind: s(a, "kind"), proposal: a.get("proposal").and_then(Value::as_u64) }).collect(),
        docs: arr(wiki, "docs").map(|d| Doc { path: s(d, "path"), title: s(d, "title"), bytes: u(d, "bytes") }).collect(),
        wiki_rev,
    }
}

pub fn fetch_snap(m: &Mcp) -> Result<Snap, String> {
    let st = m.call("status", json!({}))?;
    let chat = m.call("read_state", json!({"surface":"chat"}))?;
    let props = m.call("list_proposals", json!({"state":"all"}))?;
    let acts = m.call("read_actions", json!({}))?;
    let wiki = m.call("wiki_list", json!({"limit":500}))?;
    Ok(parse_snap(&st, &chat, &props, &acts, &wiki))
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Entry { pub chan: Chan, pub label: String, pub unread: u64, pub count: u64, pub section: u8 }

/// Sidebar order: Allgemein, topics (newest activity first), proposal discussions (open first, id desc).
pub fn group_channels(chans: &[ChanInfo], props: &[Prop], extra: &[String]) -> Vec<Entry> {
    let info = |c: &Chan| chans.iter().find(|i| &i.chan == c);
    let mut out = vec![];
    let g = info(&Chan::Group);
    out.push(Entry { chan: Chan::Group, label: "Allgemein".into(), unread: g.map_or(0, |i| i.unread), count: g.map_or(0, |i| i.count), section: 0 });
    let mut tops: Vec<(i64, Entry)> = chans.iter().filter_map(|i| match &i.chan {
        Chan::Topic(n) => Some((i.last_ts, Entry { chan: i.chan.clone(), label: n.clone(), unread: i.unread, count: i.count, section: 1 })),
        _ => None,
    }).collect();
    for n in extra {
        if !tops.iter().any(|(_, e)| e.chan == Chan::Topic(n.clone())) {
            tops.push((i64::MAX, Entry { chan: Chan::Topic(n.clone()), label: format!("{n} (neu)"), unread: 0, count: 0, section: 1 }));
        }
    }
    tops.sort_by(|a, b| b.0.cmp(&a.0));
    out.extend(tops.into_iter().map(|t| t.1));
    let mut ids: Vec<u64> = props.iter().map(|p| p.id).collect();
    for i in chans { if let Chan::Patch(id) = i.chan { if !ids.contains(&id) { ids.push(id) } } }
    let mut ps: Vec<Entry> = ids.into_iter().map(|id| {
        let p = props.iter().find(|p| p.id == id);
        let i = info(&Chan::Patch(id));
        let st = p.map(|p| p.state.clone()).or_else(|| i.and_then(|i| i.state.clone())).unwrap_or_default();
        let sum = p.map(|p| if p.summary.is_empty() { p.op.clone() } else { format!("{} {}", p.op, p.summary) }).unwrap_or_default();
        let label = format!("#{id} ({}){}", state_de(&st), if sum.is_empty() { String::new() } else { format!(" {sum}") });
        Entry { chan: Chan::Patch(id), label, unread: i.map_or(0, |i| i.unread), count: i.map_or(0, |i| i.count), section: if st == "proposed" { 2 } else { 3 } }
    }).collect();
    ps.sort_by(|a, b| a.section.cmp(&b.section).then_with(|| match (&a.chan, &b.chan) {
        (Chan::Patch(x), Chan::Patch(y)) => y.cmp(x), _ => std::cmp::Ordering::Equal }));
    out.extend(ps);
    out
}

pub fn props_sorted(ps: &[Prop]) -> Vec<&Prop> {
    let mut v: Vec<&Prop> = ps.iter().collect();
    v.sort_by(|a, b| b.open().cmp(&a.open()).then(b.id.cmp(&a.id)));
    v
}

#[cfg(test)]
pub mod fixtures {
    use super::*;
    pub fn snap() -> Snap {
        let st = json!({"name":"Demo Republic","member":"alice","members":["bob","alice","carol"],"active_1h":3,
            "relays":["ws://a","ws://b"],"missing_relays":[],"chain_diverged":[],"chain_lag":{"peers_ahead":0}});
        let chat = json!({"wiki_rev":1,"channels":[
            {"channel":{"kind":"group"},"count":3,"last_ts":1794411531,"state":null,"unread":2},
            {"channel":{"id":1,"kind":"patch"},"count":1,"last_ts":1794411143,"state":"applied","unread":0},
            {"channel":{"kind":"topic","name":"Ideen"},"count":1,"last_ts":1794411900,"unread":1},
            {"channel":{"id":4,"kind":"patch"},"count":1,"last_ts":1794412178,"state":"proposed","unread":1}],
          "applied":[
            {"id":"a1","from":"bob","ts":1794410193,"body":"Hallo zusammen,\nwie läuft's? \u{1b}[31mROT\u{1b}[0m \u{202e}evil","channel":null,"reactions":{"❤️":["x"]}},
            {"id":"a2","from":"alice","ts":1794410293,"body":"Ein sehr langer Text der umgebrochen werden muss, weil er nicht in eine Zeile passt und noch weiter geht.","quote":"a1"},
            {"id":"a3","from":"carol","ts":1794411293,"body":"Idee: Wiki ausbauen","channel":{"kind":"topic","name":"Ideen"}},
            {"id":"a4","from":"bob","ts":1794412178,"body":"Vorschlag #4: Ontologie","channel":{"id":4,"kind":"patch"}}]});
        let props = json!({"proposals":[
            {"id":1,"state":"applied","by":"bob","surface":"memory","payload":{"op":"wiki_patch","summary":"+3"},"approvals":2,"threshold":2,"votes":[]},
            {"id":4,"state":"proposed","by":"bob","surface":"memory","payload":{"op":"wiki_patch","summary":"+1 ~163"},"approvals":1,"threshold":2,
             "paths":["home.md"],"approved_by_me":false,"declined_by_me":false,
             "votes":[{"member":"bob","vote":"approved"},{"member":"alice","vote":"open"},{"member":"carol","vote":"declined"}]}]});
        let acts = json!({"actions":[{"kind":"vote","proposal":4}]});
        let wiki = json!({"docs":[{"path":"home.md","title":"Startseite","bytes":1055},{"path":"themen/index.md","title":"Themenübersicht","bytes":915}]});
        parse_snap(&st, &chat, &props, &acts, &wiki)
    }
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn sanitized_on_parse() {
        let s = fixtures::snap();
        assert!(s.msgs[0].body.contains("ROT evil"));
        assert!(!s.msgs[0].body.contains('\x1b') && !s.msgs[0].body.contains('\u{202e}'));
        assert_eq!(s.msgs[0].chan, Chan::Group);
        assert_eq!(s.msgs[1].quote.as_deref(), Some("a1"));
    }
    #[test]
    fn grouping() {
        let s = fixtures::snap();
        let e = group_channels(&s.chans, &s.props, &["Neu".into()]);
        let l: Vec<_> = e.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(l, vec!["Allgemein", "Neu (neu)", "Ideen", "#4 (offen) wiki_patch +1 ~163", "#1 (angenommen) wiki_patch +3"]);
        assert_eq!(e[0].unread, 2);
    }
    #[test]
    fn chan_json() {
        assert_eq!(Chan::Patch(4).to_json().unwrap()["id"], 4);
        assert!(Chan::Group.to_json().is_none());
        assert_eq!(Chan::from_json(&json!({"kind":"topic","name":"x"})), Chan::Topic("x".into()));
    }
}
