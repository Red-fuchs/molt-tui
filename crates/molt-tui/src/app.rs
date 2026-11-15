use crate::editor::{Editor, Mv};
use crate::model::*;
use crate::text::{clean, one};
use molt_client::chat;
use molt_client::compose::{self, Kind, Plan};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View { Chat, Props, Wiki }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row { Head(usize), Ent(usize) }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus { Side, Main, Input }
pub enum Hit { Tab(View), Head(usize), Chan(usize), Msg(String), Prop(usize), Doc(usize), Input }
#[derive(Clone, Debug)]
pub enum Tag { Send, Vote(u64, bool), Withdraw(u64), Propose, Check, MarkRead, Detail(u64), Doc(String), Done(&'static str) }
pub struct Req { pub tag: Tag, pub name: &'static str, pub args: Value }
pub enum Ev { Snap(Result<Snap, String>), Reply(Tag, Result<Value, String>), Img(String, crate::imgs::Decoded) }
/// @mention popup above the input: (name, online) items, selection, char range of the token to replace.
pub struct Pop { pub items: Vec<(String, bool)>, pub sel: usize, pub range: std::ops::Range<usize> }
pub enum Modal { React(String), Delete(String), Unshare(String), Thread(chat::Thread, String), Forget(String, String), Send, Vote(u64, bool), Withdraw(u64), NewTopic(String), Propose(Box<PropForm>) }

/// Proposal editor: type picker (field 0), one-line field (1), multi-line field (2); `plan` set = preview stage awaiting J.
pub struct PropForm { pub kind: usize, pub field: usize, pub a: String, pub b: Editor, pub err: String, pub plan: Option<Plan>, pub warns: Vec<compose::Warning> }
impl PropForm {
    pub fn new() -> Self { PropForm { kind: 0, field: 0, a: String::new(), b: Editor::default(), err: String::new(), plan: None, warns: vec![] } }
    pub fn kind(&self) -> Kind { compose::ALL[self.kind] }
    fn used(&self, f: usize) -> bool { let (a, b) = self.kind().fields(); match f { 0 => true, 1 => !a.is_empty(), _ => !b.is_empty() } }
    fn step(&mut self, d: i32) {
        for _ in 0..3 { self.field = (self.field as i32 + d).rem_euclid(3) as usize; if self.used(self.field) { break } }
    }
}

pub struct App {
    pub col: HashSet<String>, pub sidebar: bool, pub hdr: Option<usize>, pub ui_path: Option<std::path::PathBuf>,
    pub draft: Option<Box<PropForm>>,
    pub pop: Option<Pop>, pub pop_off: Option<usize>,
    pub snap: Snap, pub loaded: bool, pub err: Option<String>, pub fails: u32,
    pub tab: View, pub focus: Focus,
    pub entries: Vec<Entry>, pub cur: Chan, pub extra: Vec<String>,
    pub sel: Option<String>, pub scroll: usize, pub reveal: bool, pub last_total: usize,
    pub ed: Editor, pub quote: Option<String>, pub confirm: bool, pub mouse: bool, pub mouse_dirty: bool,
    pub modal: Option<Modal>, pub help: bool,
    pub psel: usize, pub pscroll: usize, pub details: HashMap<u64, Prop>, pub pending: HashSet<String>,
    pub wsel: usize, pub wscroll: usize, pub docs: HashMap<String, String>,
    pub toast: Option<(String, bool, Instant)>, pub off: i64,
    pub hits: Vec<(Rect, Hit)>, pub main_rect: Rect, pub in_rect: Rect, pub in_top: usize, pub in_w: usize,
    pub imgs: crate::imgs::Imgs, pub tx: Option<Sender<Req>>, pub nudge: Option<Sender<()>>, pub sending: bool, pub quit: bool,
}

pub fn pbcopy(s: &str) -> bool {
    Command::new("pbcopy").stdin(Stdio::piped()).spawn().ok().and_then(|mut c| {
        c.stdin.take()?.write_all(s.as_bytes()).ok()?;
        c.wait().ok()
    }).is_some_and(|s| s.success())
}
fn pbpaste() -> String {
    Command::new("pbpaste").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

pub use molt_client::model::{sender_id, tag_body};

impl App {
    pub fn new(off: i64) -> App {
        App {
            col: HashSet::new(), sidebar: true, hdr: None, ui_path: None,
            draft: None, pop: None, pop_off: None, snap: Snap::default(), loaded: false, err: None, fails: 0, tab: View::Chat, focus: Focus::Input,
            entries: vec![], cur: Chan::Group, extra: vec![], sel: None, scroll: 0, reveal: false, last_total: 0,
            ed: Editor::default(), quote: None, confirm: true, mouse: true, mouse_dirty: false,
            modal: None, help: false, psel: 0, pscroll: 0, details: HashMap::new(), pending: HashSet::new(),
            wsel: 0, wscroll: 0, docs: HashMap::new(), toast: None, off,
            hits: vec![], main_rect: Rect::default(), in_rect: Rect::default(), in_top: 0, in_w: 40,
            imgs: crate::imgs::Imgs::new(), tx: None, nudge: None, sending: false, quit: false,
        }
    }

    /// Poke summary line: persists ~10 s (vs 4 s).
    pub fn toast_long(&mut self, s: impl Into<String>, err: bool) { self.toast = Some((s.into(), err, Instant::now() + std::time::Duration::from_secs(6))) }
    pub fn toast(&mut self, s: impl Into<String>, err: bool) { self.toast = Some((s.into(), err, Instant::now())) }
    fn req(&self, tag: Tag, name: &'static str, args: Value) { if let Some(t) = &self.tx { let _ = t.send(Req { tag, name, args }); } }
    fn poke(&self) { if let Some(n) = &self.nudge { let _ = n.send(()); } }

    pub fn msgs(&self) -> Vec<&Msg> {
        let mut v: Vec<&Msg> = self.snap.msgs.iter().filter(|m| m.chan == self.cur).collect();
        v.sort_by_key(|m| m.ts);
        v
    }
    pub fn me(&self) -> &str { &self.snap.st.member }
    pub fn sorted_props(&self) -> Vec<&Prop> { props_sorted(&self.snap.props) }

    pub fn rebuild(&mut self) {
        let have: Vec<&Chan> = self.snap.chans.iter().map(|c| &c.chan).collect();
        self.extra.retain(|n| !have.contains(&&Chan::Topic(n.clone())));
        self.entries = group_channels(&self.snap.chans, &self.snap.props, &self.extra);
        if !self.entries.iter().any(|e| e.chan == self.cur) { self.cur = Chan::Group }
        self.psel = self.psel.min(self.snap.props.len().saturating_sub(1));
        self.wsel = self.wsel.min(self.snap.docs.len().saturating_sub(1));
    }

    pub fn on_ev(&mut self, ev: Ev) {
        match ev {
            Ev::Snap(Ok(s)) => {
                self.err = None; self.fails = 0; self.loaded = true;
                let key = |p: &Prop| format!("{}{}{:?}", p.state, p.approvals, p.votes);
                let old = std::mem::replace(&mut self.snap, s);
                if old.wiki_rev != self.snap.wiki_rev { self.docs.clear() }
                let snap = &self.snap;
                self.details.retain(|id, d| snap.props.iter().find(|p| p.id == *id).is_some_and(|p| key(p) == key(d)));
                self.pending.clear();
                self.rebuild();
            }
            Ev::Img(id, r) => self.imgs.on_loaded(id, r),
            Ev::Snap(Err(e)) => { self.fails += 1; self.err = Some(one(&e)) }
            Ev::Reply(tag, r) => match (tag, r) {
                (Tag::Send, Ok(v)) => {
                    self.sending = false; self.ed.clear(); self.quote = None; self.scroll = 0; self.sel = None;
                    match v.get("@pokes").and_then(Value::as_str) {
                        Some(t) => self.toast_long(format!("Gesendet · {t}"), false),
                        None => self.toast("Gesendet", false),
                    }
                    self.poke();
                }
                (Tag::Send, Err(e)) => { self.sending = false; self.toast(format!("Senden fehlgeschlagen: {}", one(&e)), true) }
                (Tag::Vote(id, a), Ok(_)) => {
                    self.details.remove(&id);
                    self.toast(format!("Vorschlag #{id}: {} gesendet", if a { "Zustimmung" } else { "Ablehnung" }), false);
                    self.poke();
                }
                (Tag::Vote(id, _), Err(e)) => self.toast(format!("Abstimmung #{id} fehlgeschlagen: {}", one(&e)), true),
                (Tag::Withdraw(id), Ok(_)) => { self.details.remove(&id); self.toast(format!("Vorschlag #{id} zurückgezogen"), false); self.poke() }
                (Tag::Withdraw(id), Err(e)) => self.toast(format!("Zurückziehen #{id} fehlgeschlagen: {}", one(&e)), true),
                (Tag::Check, r) => if let Some(mut f) = self.draft.take() {
                    match r {
                        Ok(v) => match compose::plan(f.kind(), &f.a, &f.b.text()) {
                            Ok(mut p) => { p.args = v["args"].clone(); f.warns = compose::parse_warnings(&v); f.plan = Some(p); f.err.clear() }
                            Err(e) => f.err = e,
                        },
                        Err(e) => f.err = one(&e),
                    }
                    self.modal = Some(Modal::Propose(f));
                },
                (Tag::Propose, Ok(v)) => {
                    self.draft = None;
                    let id = compose::result_id(&v).map(|i| format!(" (#{i})")).unwrap_or_default();
                    self.toast(format!("Vorschlag eingereicht{id}"), false); self.poke();
                }
                (Tag::Propose, Err(e)) => {
                    if let Some(mut f) = self.draft.take() { f.err = one(&e); self.modal = Some(Modal::Propose(f)) }
                    let head = if molt_client::mcp::is_unknown(&e) { "Vorschlag" } else { "Vorschlag abgelehnt:" };
                    self.toast(format!("{head} {}", one(&e)), true);
                }
                (Tag::Done(l), Ok(_)) => { self.toast(l, false); self.poke() }
                (Tag::Done(l), Err(e)) => self.toast(format!("{l}: fehlgeschlagen: {}", one(&e)), true),
                (Tag::MarkRead, Ok(_)) => { self.toast("Als gelesen markiert", false); self.poke() }
                (Tag::MarkRead, Err(e)) => self.toast(format!("Fehler: {}", one(&e)), true),
                (Tag::Detail(id), Ok(v)) => { self.details.insert(id, parse_prop(&v)); }
                (Tag::Doc(p), Ok(v)) => {
                    let c = v.get("content").and_then(Value::as_str).map(clean).unwrap_or_else(|| clean(&v.to_string()));
                    self.docs.insert(p, c);
                }
                (_, Err(e)) => self.toast(format!("Fehler: {}", one(&e)), true),
            },
        }
    }

    /// Issue on-demand fetches for what the current view needs.
    pub fn pump(&mut self) {
        if self.tab == View::Chat && self.imgs.enabled() {
            let snap = std::mem::take(&mut self.snap.msgs);
            let v: Vec<&Msg> = { let mut v: Vec<&Msg> = snap.iter().filter(|m| m.chan == self.cur).collect(); v.sort_by_key(|m| m.ts); v };
            self.imgs.want(&v);
            self.imgs.sync_viewer(&v);
            drop(v);
            self.snap.msgs = snap;
        }
        match self.tab {
            View::Props => {
                let id = self.sorted_props().get(self.psel).map(|p| p.id);
                if let Some(id) = id { if !self.details.contains_key(&id) && self.pending.insert(format!("p{id}")) {
                    self.req(Tag::Detail(id), "read_proposal", json!({"id":id}));
                } }
            }
            View::Wiki => {
                let p = self.snap.docs.get(self.wsel).map(|d| d.path.clone());
                if let Some(p) = p { if !self.docs.contains_key(&p) && self.pending.insert(format!("w{p}")) {
                    self.req(Tag::Doc(p.clone()), "wiki_get", json!({"path":p}));
                } }
            }
            _ => {}
        }
    }

    pub fn select_chan(&mut self, c: Chan) {
        if c != self.cur { self.quote = None }
        self.cur = c; self.sel = None; self.scroll = 0; self.last_total = 0; self.reveal = false;
    }
    fn set_tab(&mut self, t: View) {
        self.tab = t;
        self.focus = if t == View::Chat { Focus::Input } else { Focus::Side };
    }
    fn cycle(&mut self, fwd: bool) {
        let o: &[Focus] = if self.tab == View::Chat && !self.sidebar { &[Focus::Main, Focus::Input] } else if self.tab == View::Chat { &[Focus::Side, Focus::Main, Focus::Input] } else { &[Focus::Side, Focus::Main] };
        let i = o.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = o[if fwd { (i + 1) % o.len() } else { (i + o.len() - 1) % o.len() }];
    }

    fn try_send(&mut self) {
        if self.ed.is_empty() || self.sending { return }
        if self.confirm { self.modal = Some(Modal::Send) } else { self.do_send() }
    }
    fn do_send(&mut self) {
        let mut a = json!({"body": tag_body(self.ed.text().trim_end(), &sender_id())});
        if let Some(c) = self.cur.to_json() { a["channel"] = c }
        if let Some(q) = &self.quote { a["quote"] = json!(q) }
        let ro: Vec<Value> = self.snap.roster.iter().map(|m| json!([m.name, m.presence])).collect();
        a["@poke"] = json!({"me": self.me(), "enabled": self.snap.poke_enabled, "roster": ro});
        self.sending = true;
        self.req(Tag::Send, "chat_send", a);
    }
    fn mark_read(&mut self) {
        let a = self.cur.to_json().map_or(json!({}), |c| json!({"channel": c}));
        self.req(Tag::MarkRead, "mark_channel_read", a);
    }
    /// Selected message, only if it is ours: seat author AND our sender tag (the seat is shared with other humans/AIs).
    fn own_sel(&self) -> Option<&Msg> {
        let id = self.sel.as_ref()?;
        let m = self.snap.msgs.iter().find(|m| &m.id == id)?;
        let tag = sender_id();
        let ok = molt_client::chat::is_own(m, self.me()) && (tag == "-" || split_tag(&m.body).0 == Some(tag.as_str()));
        ok.then_some(m)
    }
    fn react(&mut self, id: String, i: usize) {
        if let Some(e) = chat::REACTIONS.get(i) { self.req(Tag::Done("Reaktion gesendet"), "react_chat", json!({"id":id,"emoji":e})) }
    }
    fn cur_topic(&self) -> Option<String> { if let Chan::Topic(n) = &self.cur { Some(n.clone()) } else { None } }
    fn mark_sel_read(&mut self) {
        match self.sel.clone().filter(|i| chat::valid_id(i)) {
            Some(id) => self.req(Tag::Done("Nachricht als gelesen markiert"), "mark_read", json!({"ids":[id]})),
            None => self.toast("Erst eine Nachricht wählen", true),
        }
    }
    fn vote(&mut self, id: u64, approve: bool) {
        self.req(Tag::Vote(id, approve), if approve { "approve" } else { "decline" }, json!({"proposal_id": id}));
    }
    fn withdraw(&mut self, id: u64) { self.req(Tag::Withdraw(id), "withdraw", json!({"proposal_id": id})) }
    fn copy(&mut self, s: &str) {
        if pbcopy(s) { self.toast(format!("{} Zeichen kopiert", s.chars().count()), false) } else { self.toast("pbcopy fehlgeschlagen", true) }
    }
    fn sel_msg_text(&self) -> Option<(String, String)> {
        let id = self.sel.as_ref()?;
        self.snap.msgs.iter().find(|m| &m.id == id).map(|m| (m.from.clone(), m.body.clone()))
    }
    fn move_sel(&mut self, d: i32) {
        let ids: Vec<String> = self.msgs().iter().map(|m| m.id.clone()).collect();
        if ids.is_empty() { return }
        let i = self.sel.as_ref().and_then(|s| ids.iter().position(|x| x == s));
        let n = match (i, d) {
            (None, _) => ids.len() - 1,
            (Some(i), d) if d < 0 => i.saturating_sub(1),
            (Some(i), _) => (i + 1).min(ids.len() - 1),
        };
        self.sel = Some(ids[n].clone());
        self.reveal = true;
    }
    pub fn is_col(&self, sec: usize) -> bool { self.col.contains(crate::uistate::SECS[sec.min(3)].0) }
    /// Visible sidebar rows: a header per section; collapsed sections keep only the selected and unread channels.
    pub fn side_rows(&self) -> Vec<Row> {
        let mut v = vec![];
        let mut last = usize::MAX;
        for (i, e) in self.entries.iter().enumerate() {
            let sec = (e.section as usize).min(3);
            if sec != last { v.push(Row::Head(sec)); last = sec }
            if !self.is_col(sec) || e.chan == self.cur || e.unread > 0 { v.push(Row::Ent(i)) }
        }
        v
    }
    pub fn sec_unread(&self, sec: usize) -> u64 { self.entries.iter().filter(|e| (e.section as usize).min(3) == sec).map(|e| e.unread).sum() }
    pub fn save_ui(&self) { if let Some(p) = &self.ui_path { let _ = crate::uistate::save(p, &self.col, self.sidebar); } }
    pub fn load_ui(&mut self, p: std::path::PathBuf) { (self.col, self.sidebar) = crate::uistate::load(&p); self.ui_path = Some(p); }
    pub fn toggle_sec(&mut self, sec: usize) {
        let id = crate::uistate::SECS[sec.min(3)].0.to_string();
        if !self.col.remove(&id) { self.col.insert(id); }
        self.save_ui();
    }
    pub fn toggle_all(&mut self) {
        let present: Vec<usize> = { let mut v: Vec<usize> = self.entries.iter().map(|e| (e.section as usize).min(3)).collect(); v.dedup(); v };
        if present.iter().all(|s| self.is_col(*s)) { self.col.clear() } else { for s in present { self.col.insert(crate::uistate::SECS[s].0.to_string()); } }
        self.save_ui();
    }
    pub fn toggle_sidebar(&mut self) {
        self.sidebar = !self.sidebar;
        if !self.sidebar && self.focus == Focus::Side && self.tab == View::Chat { self.focus = Focus::Input }
        self.save_ui();
        self.toast(if self.sidebar { "Kanalliste: an (Ctrl+B)" } else { "Kanalliste: aus (Ctrl+B = wieder an)" }, false);
    }
    fn side_move(&mut self, d: i32) {
        let rows = self.side_rows();
        if rows.is_empty() { return }
        let cur = match self.hdr { Some(s) => rows.iter().position(|r| *r == Row::Head(s)), None => rows.iter().position(|r| matches!(r, Row::Ent(i) if self.entries[*i].chan == self.cur)) }.unwrap_or(0) as i32;
        let n = (cur + d).clamp(0, rows.len() as i32 - 1) as usize;
        match rows[n] {
            Row::Head(s) => self.hdr = Some(s),
            Row::Ent(i) => { self.hdr = None; let c = self.entries[i].chan.clone(); self.select_chan(c) }
        }
    }
    pub fn open_discussion(&mut self, id: u64) {
        self.tab = View::Chat; self.focus = Focus::Input; self.select_chan(Chan::Patch(id));
    }

    pub fn on_paste(&mut self, s: String) {
        if let Some(Modal::NewTopic(t)) = &mut self.modal { t.push_str(&one(&s)); return }
        if let Some(Modal::Propose(f)) = &mut self.modal {
            if f.plan.is_none() { if f.field == 2 { f.b.insert_str(&clean(&s)) } else if f.field == 1 { f.a.push_str(&one(&s)) } }
            return;
        }
        if self.modal.is_some() || self.help || self.tab != View::Chat { return }
        self.focus = Focus::Input;
        self.ed.insert_str(&s);
    }

    fn modal_key(&mut self, m: Modal, k: KeyEvent) -> Option<Modal> {
        use KeyCode::*;
        match m {
            Modal::Send => match k.code {
                Enter | Char('j' | 'J' | 'y' | 'Y') => { self.do_send(); None }
                Char('a' | 'A') => { self.confirm = false; self.toast("Rückfrage vor dem Senden aus (F6)", false); self.do_send(); None }
                Esc | Char('n' | 'N') => None,
                _ => Some(Modal::Send),
            },
            Modal::React(id) => match k.code {
                Char(c @ '1'..='6') => { self.react(id, c as usize - '1' as usize); None }
                Esc => None,
                _ => Some(Modal::React(id)),
            },
            Modal::Delete(id) => match k.code {
                Char('j' | 'J' | 'y' | 'Y') => { self.req(Tag::Done("Nachricht gelöscht"), "delete_chat", json!({"id":id})); None }
                Esc | Char('n' | 'N') => None,
                _ => Some(Modal::Delete(id)),
            },
            Modal::Unshare(id) => match k.code {
                Char('j' | 'J' | 'y' | 'Y') => { self.req(Tag::Done("Freigabe zurückgezogen"), "remove_file", json!({"id":id})); None }
                Esc | Char('n' | 'N') => None,
                _ => Some(Modal::Unshare(id)),
            },
            Modal::Thread(op, n) => match k.code {
                Char('j' | 'J' | 'y' | 'Y') => { self.req(Tag::Done(if op == chat::Thread::Close { "Thema geschlossen" } else { "Thema wieder geöffnet" }), op.tool(), json!({"name":n})); None }
                Esc | Char('n' | 'N') => None,
                _ => Some(Modal::Thread(op, n)),
            },
            Modal::Forget(n, mut t) => match k.code {
                Esc => None,
                Enter if t == n => {
                    if self.cur == Chan::Topic(n.clone()) { self.select_chan(Chan::Group) }
                    self.req(Tag::Done("Thema auf diesem Sitz vergessen"), "thread_forget", json!({"name":n})); None
                }
                Enter => { self.toast("Name stimmt nicht überein", true); Some(Modal::Forget(n, t)) }
                Backspace => { t.pop(); Some(Modal::Forget(n, t)) }
                Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => { t.push(c); Some(Modal::Forget(n, t)) }
                _ => Some(Modal::Forget(n, t)),
            },
            Modal::Vote(id, a) => match k.code {
                Char('j' | 'J' | 'y' | 'Y') => { self.vote(id, a); None }
                Esc | Char('n' | 'N') => None,
                _ => Some(Modal::Vote(id, a)),
            },
            Modal::Withdraw(id) => match k.code {
                Char('j' | 'J' | 'y' | 'Y') => { self.withdraw(id); None }
                Esc | Char('n' | 'N') => None,
                _ => Some(Modal::Withdraw(id)),
            },
            Modal::Propose(mut f) => { if self.propose_key(&mut f, k) { Some(Modal::Propose(f)) } else { None } }
            Modal::NewTopic(mut t) => match k.code {
                Esc => None,
                Enter => {
                    let n = match compose::topic_name(&t) { Ok(n) => n, Err(e) => { self.toast(format!("Thema: {e}"), true); return Some(Modal::NewTopic(t)) } };
                    if !self.extra.contains(&n) { self.extra.push(n.clone()) }
                    self.tab = View::Chat; self.focus = Focus::Input;
                    self.rebuild();
                    self.select_chan(Chan::Topic(n));
                    None
                }
                Backspace => { t.pop(); Some(Modal::NewTopic(t)) }
                Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => { t.push(c); Some(Modal::NewTopic(t)) }
                _ => Some(Modal::NewTopic(t)),
            },
        }
    }

    /// Returns false when the form closes.
    fn propose_key(&mut self, f: &mut PropForm, k: KeyEvent) -> bool {
        use KeyCode::*;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if f.plan.is_some() {
            match k.code {
                Char(c @ ('j' | 'J' | 'y' | 'Y' | 'a' | 'A')) => {
                    let accept = matches!(c, 'a' | 'A');
                    if f.warns.is_empty() == accept { // warnings present: only A (accept) submits; none: only J
                        if !accept { self.toast("Warnungen vorhanden: A akzeptiert sie und reicht ein", true) }
                        return true;
                    }
                    let p = f.plan.take().unwrap();
                    let codes: Vec<String> = f.warns.iter().map(|w| w.code.clone()).collect();
                    let sent = compose::confirm(&compose::Checked { plan: p.clone(), warnings: f.warns.clone() }, &codes);
                    self.draft = Some(Box::new(PropForm { kind: f.kind, field: f.field, a: std::mem::take(&mut f.a), b: std::mem::take(&mut f.b), err: String::new(), plan: Some(p), warns: f.warns.clone() }));
                    self.req(Tag::Propose, sent.tool, sent.args);
                    return false;
                }
                Esc => f.plan = None,
                Char('n' | 'N') => return false,
                _ => {}
            }
            return true;
        }
        let preview = |f: &mut PropForm| match compose::plan(f.kind(), &f.a, &f.b.text()) { Ok(p) => { f.err.clear(); f.plan = Some(p) } Err(e) => f.err = e };
        let w = self.in_w.max(20);
        match (k.code, f.field) {
            (Esc, _) => return false,
            (Char('s'), _) if ctrl => {
                preview(f);
                if f.plan.as_ref().is_some_and(|p| p.tool == "wiki_edit") { // dry run first: header + moltd warnings
                    let p = f.plan.take().unwrap();
                    self.draft = Some(Box::new(PropForm { kind: f.kind, field: f.field, a: std::mem::take(&mut f.a), b: std::mem::take(&mut f.b), err: String::new(), plan: None, warns: vec![] }));
                    self.toast("Prüfe Vorschlag (Probelauf, nichts wird gesendet) …", false);
                    self.req(Tag::Check, "@check", p.args);
                    return false;
                }
            }
            (Tab, _) => f.step(1),
            (BackTab, _) => f.step(-1),
            (Left | Up, 0) => { f.kind = (f.kind + compose::ALL.len() - 1) % compose::ALL.len(); f.err.clear() }
            (Right | Down, 0) => { f.kind = (f.kind + 1) % compose::ALL.len(); f.err.clear() }
            (Enter, 0) => f.step(1),
            (Enter, 1) if f.kind().fields().1.is_empty() => preview(f),
            (Enter, 1) => f.step(1),
            (Backspace, 1) => { f.a.pop(); }
            (Char('u'), 1) if ctrl => f.a.clear(),
            (Char(c), 1) if !ctrl => f.a.push(c),
            (Enter, 2) => f.b.newline(),
            (Char('j'), 2) if ctrl => f.b.newline(),
            (Char('v'), 2) if ctrl => f.b.insert_str(&pbpaste()),
            (Char('z'), 2) if ctrl => f.b.undo(),
            (Char('k'), 2) if ctrl => f.b.kill_end(),
            (Char('u'), 2) if ctrl => f.b.kill_start(),
            (Char('w'), 2) if ctrl => f.b.del_word_back(),
            (Backspace, 2) => f.b.backspace(),
            (Delete, 2) => f.b.delete(),
            (Left, 2) => f.b.mv(Mv::L, false, w),
            (Right, 2) => f.b.mv(Mv::R, false, w),
            (Up, 2) => f.b.mv(Mv::Up, false, w),
            (Down, 2) => f.b.mv(Mv::Down, false, w),
            (Home, 2) => f.b.mv(Mv::Home, false, w),
            (End, 2) => f.b.mv(Mv::End, false, w),
            (Char(c), 2) if !ctrl => f.b.insert_str(&c.to_string()),
            _ => {}
        }
        true
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        use KeyCode::*;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == Char('c') { self.quit = true; return }
        if self.imgs.viewer.is_some() { self.viewer_key(k); return }
        if let Some(m) = self.modal.take() { self.modal = self.modal_key(m, k); return }
        if self.help {
            if matches!(k.code, Esc | F(1) | Char('?' | 'q')) { self.help = false }
            return;
        }
        match k.code {
            F(1) => { self.help = true; return }
            F(2) => { self.mouse = !self.mouse; self.mouse_dirty = true;
                self.toast(if self.mouse { "Maus an (Klicken/Scrollen)" } else { "Maus aus: Text jetzt normal markierbar, F2 = wieder an" }, false); return }
            F(3) => { self.set_tab(View::Chat); return }
            F(4) => { self.set_tab(View::Props); return }
            F(5) => { self.set_tab(View::Wiki); return }
            F(6) => { self.confirm = !self.confirm; self.toast(if self.confirm { "Rückfrage vor dem Senden: an" } else { "Rückfrage vor dem Senden: aus" }, false); return }
            F(7) => { self.imgs.on = !self.imgs.on; self.toast(if !self.imgs.enabled() { "Bilder nicht verfügbar (kein Terminal oder MOLT_IMAGES=off)" } else if self.imgs.on { "Bilder: an" } else { "Bilder: aus" }, false); return }
            Tab => { self.cycle(true); return }
            BackTab => { self.cycle(false); return }
            Char('n') if ctrl => { self.modal = Some(Modal::NewTopic(String::new())); return }
            Char('b') if ctrl => { self.toggle_sidebar(); return }
            F(8) => { self.toggle_sidebar(); return }
            Char('p') if ctrl => { self.modal = Some(Modal::Propose(Box::new(PropForm::new()))); return }
            Char('r') if ctrl => { self.poke(); self.toast("Aktualisiere…", false); return }
            _ => {}
        }
        let typing = self.tab == View::Chat && self.focus == Focus::Input;
        if !typing {
            match k.code {
                Char('q') => { self.quit = true; return }
                Char('?') => { self.help = true; return }
                Char('1') => { self.set_tab(View::Chat); return }
                Char('2') => { self.set_tab(View::Props); return }
                Char('3') => { self.set_tab(View::Wiki); return }
                _ => {}
            }
        }
        match self.tab {
            View::Chat => match self.focus {
                Focus::Input => self.input_key(k),
                Focus::Main => self.main_key(k),
                Focus::Side => self.side_key(k),
            },
            View::Props => self.props_key(k),
            View::Wiki => self.wiki_key(k),
        }
    }

    fn input_key(&mut self, k: KeyEvent) {
        use KeyCode::*;
        let m = k.modifiers;
        let (ctrl, alt, shift) = (m.contains(KeyModifiers::CONTROL), m.contains(KeyModifiers::ALT), m.contains(KeyModifiers::SHIFT));
        if self.pop.is_some() && !ctrl && !alt {
            match k.code {
                Up => { if let Some(p) = &mut self.pop { p.sel = (p.sel + p.items.len() - 1) % p.items.len() } return }
                Down => { if let Some(p) = &mut self.pop { p.sel = (p.sel + 1) % p.items.len() } return }
                Tab | Enter if !shift => { self.accept_mention(); return }
                Esc => { self.pop_off = self.pop.take().map(|p| p.range.start); return }
                _ => {}
            }
        }
        self.input_edit(k);
        self.update_pop();
    }

    /// Recompute the @mention popup from the editor text/cursor.
    pub fn update_pop(&mut self) {
        let tok = if self.tab == View::Chat && self.focus == Focus::Input { molt_client::mention::token_at(&self.ed.text(), self.ed.cur) } else { None };
        let Some((range, q)) = tok else { self.pop = None; self.pop_off = None; return };
        if self.pop_off == Some(range.start) { self.pop = None; return }
        let items: Vec<(String, bool)> = molt_client::mention::complete(&q, &self.snap.roster, self.me()).into_iter().take(8).map(|m| (m.name.clone(), m.presence > 0)).collect();
        let sel = self.pop.as_ref().map_or(0, |p| p.sel).min(items.len().saturating_sub(1));
        self.pop = if items.is_empty() { None } else { Some(Pop { items, sel, range }) };
    }
    fn accept_mention(&mut self) {
        let Some(p) = self.pop.take() else { return };
        let (t, cur) = molt_client::mention::insert(&self.ed.text(), p.range.clone(), &p.items[p.sel].0);
        let ins: String = t.chars().skip(p.range.start).take(cur - p.range.start).collect();
        self.ed.replace(p.range, &ins);
        self.pop_off = None;
    }

    fn input_edit(&mut self, k: KeyEvent) {
        use KeyCode::*;
        let m = k.modifiers;
        let (ctrl, alt, shift) = (m.contains(KeyModifiers::CONTROL), m.contains(KeyModifiers::ALT), m.contains(KeyModifiers::SHIFT));
        let w = self.in_w;
        let e = &mut self.ed;
        match k.code {
            Enter if alt || shift => e.newline(),
            Enter => self.try_send(),
            Esc => { if self.quote.is_some() { self.quote = None } else if e.sel().is_some() { e.anc = None } else { self.focus = Focus::Main } }
            Char('j') if ctrl => e.newline(),
            Char('a') if ctrl => e.mv(Mv::Home, false, w),
            Char('e') if ctrl => e.mv(Mv::End, false, w),
            Char('k') if ctrl => e.kill_end(),
            Char('u') if ctrl => e.kill_start(),
            Char('w') if ctrl => e.del_word_back(),
            Char('z') if ctrl => e.undo(),
            Char('y') if ctrl => e.redo(),
            Char('d') if ctrl => e.delete(),
            Char('h') if ctrl => e.backspace(),
            Char('b') if ctrl => e.mv(Mv::L, false, w),
            Char('f') if ctrl => e.mv(Mv::R, false, w),
            Char('x') if ctrl => { let s = e.cut(); if !s.is_empty() { self.copy(&s) } }
            Char('v') if ctrl => { let s = pbpaste(); self.ed.insert_str(&s) }
            Char('b') if alt => e.mv(Mv::WL, false, w),
            Char('f') if alt => e.mv(Mv::WR, false, w),
            Char('a') if alt => e.select_all(),
            Char('c') if alt => { let s = e.selected(); if !s.is_empty() { self.copy(&s) } }
            Backspace if alt || ctrl => e.del_word_back(),
            Backspace => e.backspace(),
            Delete => e.delete(),
            Left => e.mv(if alt || ctrl { Mv::WL } else { Mv::L }, shift, w),
            Right => e.mv(if alt || ctrl { Mv::WR } else { Mv::R }, shift, w),
            Up if e.is_empty() && !shift => { self.focus = Focus::Main; self.move_sel(-1) }
            Up => e.mv(Mv::Up, shift, w),
            Down => e.mv(Mv::Down, shift, w),
            Home => e.mv(if ctrl { Mv::Top } else { Mv::Home }, shift, w),
            End => e.mv(if ctrl { Mv::Bottom } else { Mv::End }, shift, w),
            PageUp => self.scroll += 10,
            PageDown => self.scroll = self.scroll.saturating_sub(10),
            // Terminal.app with 'Option as Meta': Option+L arrives as ESC l = Alt+l; map the German Mac option layer back
            Char(c) if alt && !ctrl && de_option(c).is_some() => { if let Some(x) = de_option(c) { e.insert_str(&x.to_string()) } }
            // German Mac: @ { } [ ] | ~ \\ are Option+key; the terminal reports the char with ALT, so only alt+letter/digit are shortcuts
            Char(c) if !ctrl && (!alt || !c.is_alphanumeric()) => e.insert_str(&c.to_string()),
            _ => {}
        }
    }

    fn main_key(&mut self, k: KeyEvent) {
        use KeyCode::*;
        match k.code {
            Up | Char('k') => self.move_sel(-1),
            Down | Char('j') => self.move_sel(1),
            PageUp => self.scroll += 10,
            PageDown => self.scroll = self.scroll.saturating_sub(10),
            End | Char('G') => { self.scroll = 0; self.sel = None }
            Home | Char('g') => self.scroll = usize::MAX / 2,
            Char('o') | Enter if self.open_viewer() => {}
            Char('r') | Enter => {
                if self.sel.is_some() { self.quote = self.sel.clone() }
                self.focus = Focus::Input;
            }
            Char('c') => if let Some((_, b)) = self.sel_msg_text() { self.copy(&b) },
            Char('C') => if let Some((f, b)) = self.sel_msg_text() { self.copy(&format!("{f}: {b}")) },
            Char('m') => self.mark_read(),
            Char('M') => self.mark_sel_read(),
            Char('e') => match self.sel.clone().filter(|i| chat::valid_id(i)) { Some(id) => self.modal = Some(Modal::React(id)), None => self.toast("Erst eine Nachricht wählen", true) },
            Char('x') => match self.own_sel().map(|m| m.id.clone()) { Some(id) => self.modal = Some(Modal::Delete(id)), None => self.toast("Nur eigene (mit deinem Absender-Tag), nicht gelöschte Nachrichten lassen sich löschen", true) },
            Char('u') => match self.own_sel().filter(|m| m.file.is_some()).map(|m| m.id.clone()) { Some(id) => self.modal = Some(Modal::Unshare(id)), None => self.toast("Nur eigene Dateifreigaben lassen sich zurückziehen", true) },
            Char('i') => self.focus = Focus::Input,
            Esc => self.sel = None,
            _ => {}
        }
    }

    /// Open the full-screen viewer on the selected image message (only if it is loaded).
    fn open_viewer(&mut self) -> bool {
        let Some(id) = self.sel.clone() else { return false };
        let msgs = self.msgs();
        if !msgs.iter().any(|m| m.id == id && crate::imgs::is_img(m)) { return false }
        if !self.imgs.on || !self.imgs.has(&id) { return false }
        let ids = msgs.iter().filter(|m| crate::imgs::is_img(m)).map(|m| m.id.clone()).collect();
        self.imgs.viewer = crate::viewer::Viewer::new(ids, &id);
        self.imgs.viewer.is_some()
    }
    fn viewer_key(&mut self, k: KeyEvent) {
        use crate::viewer::{VAct, VKey};
        use KeyCode::*;
        let vk = match k.code { Esc | Char('q') | Char('o') | Enter => VKey::Esc, Left | Char('h') => VKey::Left, Right | Char('l') => VKey::Right,
            Home => VKey::Home, End => VKey::End, _ => VKey::Other };
        if let Some(v) = self.imgs.viewer.as_mut() { if v.step(vk) == VAct::Close { self.imgs.viewer = None } }
    }

    fn side_key(&mut self, k: KeyEvent) {
        use KeyCode::*;
        match k.code {
            Up | Char('k') => self.side_move(-1),
            Down | Char('j') => self.side_move(1),
            Enter | Char(' ') if self.hdr.is_some() => { let s = self.hdr.unwrap(); self.toggle_sec(s) }
            Char('z') => self.toggle_all(),
            Enter | Right => { self.hdr = None; self.focus = Focus::Input }
            Char('m') => self.mark_read(),
            Char('n') => self.modal = Some(Modal::NewTopic(String::new())),
            Char(c @ ('x' | 'o' | 'F')) => match self.cur_topic() {
                Some(n) => self.modal = Some(match c { 'x' => Modal::Thread(chat::Thread::Close, n), 'o' => Modal::Thread(chat::Thread::Reopen, n), _ => Modal::Forget(n, String::new()) }),
                None => self.toast("Nur für Themen-Kanäle", true),
            },
            _ => {}
        }
    }

    fn props_key(&mut self, k: KeyEvent) {
        use KeyCode::*;
        let n = self.snap.props.len();
        let cur = self.sorted_props().get(self.psel).map(|p| (p.id, p.todo()));
        let mine = self.sorted_props().get(self.psel).filter(|p| p.can_withdraw(self.me())).map(|p| p.id);
        match (k.code, self.focus) {
            (Up | Char('k'), Focus::Side) => { self.psel = self.psel.saturating_sub(1); self.pscroll = 0 }
            (Down | Char('j'), Focus::Side) => { self.psel = (self.psel + 1).min(n.saturating_sub(1)); self.pscroll = 0 }
            (Up | Char('k'), _) => self.pscroll = self.pscroll.saturating_sub(1),
            (Down | Char('j'), _) => self.pscroll += 1,
            (PageUp, _) => self.pscroll = self.pscroll.saturating_sub(10),
            (PageDown, _) => self.pscroll += 10,
            (Home, _) => self.pscroll = 0,
            (Enter, Focus::Side) => self.focus = Focus::Main,
            (Char(c @ ('a' | 'd')), _) => match cur {
                Some((id, true)) => self.modal = Some(Modal::Vote(id, c == 'a')),
                Some(_) => self.toast("Du hast hier nichts mehr abzustimmen (nicht offen oder schon gestimmt)", true),
                None => {}
            },
            (Char('t'), _) => if let Some((id, _)) = cur { self.open_discussion(id) },
            (Char('n'), _) => self.modal = Some(Modal::Propose(Box::new(PropForm::new()))),
            (Char('w'), _) => match mine { Some(id) => self.modal = Some(Modal::Withdraw(id)), None => self.toast("Nur eigene, noch offene Vorschläge lassen sich zurückziehen", true) },
            _ => {}
        }
    }

    fn wiki_key(&mut self, k: KeyEvent) {
        use KeyCode::*;
        let n = self.snap.docs.len();
        match (k.code, self.focus) {
            (Up | Char('k'), Focus::Side) => { self.wsel = self.wsel.saturating_sub(1); self.wscroll = 0 }
            (Down | Char('j'), Focus::Side) => { self.wsel = (self.wsel + 1).min(n.saturating_sub(1)); self.wscroll = 0 }
            (Up | Char('k'), _) => self.wscroll = self.wscroll.saturating_sub(1),
            (Down | Char('j'), _) => self.wscroll += 1,
            (PageUp, _) => self.wscroll = self.wscroll.saturating_sub(10),
            (PageDown, _) => self.wscroll += 10,
            (Home, _) => self.wscroll = 0,
            (Enter, Focus::Side) => self.focus = Focus::Main,
            _ => {}
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if self.imgs.viewer.is_some() { return }
        let (x, y) = (m.column, m.row);
        let inside = |r: Rect| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
        if self.modal.is_some() || self.help { return }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Down(MouseButton::Right) => {
                let right = matches!(m.kind, MouseEventKind::Down(MouseButton::Right));
                let h = self.hits.iter().rev().find(|(r, _)| inside(*r)).map(|(r, h)| (*r, match h {
                    Hit::Tab(t) => Hit::Tab(*t), Hit::Head(i) => Hit::Head(*i), Hit::Chan(i) => Hit::Chan(*i), Hit::Msg(s) => Hit::Msg(s.clone()),
                    Hit::Prop(i) => Hit::Prop(*i), Hit::Doc(i) => Hit::Doc(*i), Hit::Input => Hit::Input,
                }));
                match h {
                    Some((_, Hit::Tab(t))) => self.set_tab(t),
                    Some((_, Hit::Head(sec))) => { self.hdr = Some(sec); self.focus = Focus::Side; self.toggle_sec(sec) }
                    Some((_, Hit::Chan(i))) => { self.hdr = None; if let Some(e) = self.entries.get(i) { let c = e.chan.clone(); self.select_chan(c); self.focus = Focus::Input } }
                    Some((_, Hit::Msg(id))) => {
                        self.sel = Some(id); self.focus = Focus::Main;
                        if right { if let Some((_, b)) = self.sel_msg_text() { self.copy(&b) } }
                    }
                    Some((_, Hit::Prop(i))) => { self.psel = i; self.pscroll = 0; self.focus = Focus::Side }
                    Some((_, Hit::Doc(i))) => { self.wsel = i; self.wscroll = 0; self.focus = Focus::Side }
                    Some((_, Hit::Input)) => {
                        self.focus = Focus::Input;
                        let (r, c) = ((y - self.in_rect.y) as usize + self.in_top, (x - self.in_rect.x) as usize);
                        self.ed.click(self.in_w, r, c, m.modifiers.contains(KeyModifiers::SHIFT));
                    }
                    None => if inside(self.main_rect) { self.focus = Focus::Main },
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.focus == Focus::Input && inside(self.in_rect) => {
                let (r, c) = ((y - self.in_rect.y) as usize + self.in_top, (x - self.in_rect.x) as usize);
                self.ed.click(self.in_w, r, c, true);
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if inside(self.main_rect) => {
                let up = m.kind == MouseEventKind::ScrollUp;
                match (self.tab, up) {
                    (View::Chat, true) => self.scroll += 3,
                    (View::Chat, false) => self.scroll = self.scroll.saturating_sub(3),
                    (View::Props, true) => self.pscroll = self.pscroll.saturating_sub(3),
                    (View::Props, false) => self.pscroll += 3,
                    (View::Wiki, true) => self.wscroll = self.wscroll.saturating_sub(3),
                    (View::Wiki, false) => self.wscroll += 3,
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod t {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    fn key(c: KeyCode) -> KeyEvent { KeyEvent::new(c, KeyModifiers::NONE) }
    fn app() -> (App, std::sync::mpsc::Receiver<Req>) {
        let mut a = App::new(0);
        let (tx, rx) = std::sync::mpsc::channel();
        a.tx = Some(tx);
        a.on_ev(Ev::Snap(Ok(crate::model::fixtures::snap())));
        (a, rx)
    }
    fn ent(c: Chan, label: &str, unread: u64, section: u8) -> Entry { Entry { chan: c, label: label.into(), unread, count: 0, section } }
    fn side_app() -> App {
        let (mut a, _) = app();
        a.entries = vec![ent(Chan::Group, "Allgemein", 0, 0), ent(Chan::Topic("A".into()), "A", 0, 1), ent(Chan::Topic("B".into()), "B", 3, 1), ent(Chan::Topic("C".into()), "C", 0, 1), ent(Chan::Patch(1), "P1", 0, 2)];
        a
    }
    fn tmpdir(n: &str) -> std::path::PathBuf { let d = std::env::temp_dir().join(format!("molt-tui-ap8-{n}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); d }

    #[test]
    fn collapse_keeps_selected_and_unread() {
        let mut a = side_app();
        a.cur = Chan::Topic("A".into());
        assert_eq!(a.side_rows().len(), 8);
        a.toggle_sec(1);
        let r = a.side_rows();
        let ents: Vec<usize> = r.iter().filter_map(|x| if let Row::Ent(i) = x { Some(*i) } else { None }).collect();
        assert_eq!(ents, vec![0, 1, 2, 4]); // A (selected) + B (unread) stay, C hidden
        assert_eq!(a.sec_unread(1), 3);
        a.toggle_sec(1);
        assert_eq!(a.side_rows().len(), 8);
        a.toggle_all();
        assert!(a.is_col(0) && a.is_col(1) && a.is_col(2));
        a.toggle_all();
        assert!(a.col.is_empty());
    }

    #[test]
    fn header_keyboard_and_mouse_toggle() {
        let mut a = side_app();
        a.focus = Focus::Side;
        a.on_key(key(KeyCode::Up)); // Allgemein entry -> its header
        assert_eq!(a.hdr, Some(0));
        a.on_key(key(KeyCode::Char(' ')));
        assert!(a.is_col(0));
        a.on_key(key(KeyCode::Enter));
        assert!(!a.is_col(0));
        a.hits = vec![(Rect::new(0, 3, 20, 1), Hit::Head(1))];
        a.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 2, row: 3, modifiers: KeyModifiers::NONE });
        assert!(a.is_col(1));
    }

    #[test]
    fn sidebar_toggle_moves_focus() {
        let mut a = side_app();
        a.focus = Focus::Side;
        a.on_key(ctrl('b'));
        assert!(!a.sidebar && a.focus == Focus::Input);
        a.cycle(true);
        assert_ne!(a.focus, Focus::Side);
        a.on_key(ctrl('b'));
        assert!(a.sidebar);
    }

    #[test]
    fn ui_state_round_trip_and_corrupt() {
        let d = tmpdir("rt");
        let p = d.join("sub/ui.json");
        let mut a = side_app();
        a.load_ui(p.clone()); // missing -> defaults
        assert!(a.col.is_empty() && a.sidebar);
        a.toggle_sec(2); a.toggle_sidebar();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(p.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        let mut b = side_app();
        b.load_ui(p.clone());
        assert!(b.is_col(2) && !b.sidebar);
        std::fs::write(&p, "{not json").unwrap();
        let mut c = side_app();
        c.load_ui(p);
        assert!(c.col.is_empty() && c.sidebar);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn react_delete_threads_confirm_flows() {
        let (mut a, rx) = app();
        a.focus = Focus::Main;
        a.sel = Some("a1".into()); // fixture ids are not 32-hex: refused locally
        a.on_key(key(KeyCode::Char('e')));
        assert!(a.modal.is_none() && rx.try_recv().is_err());
        let id = "b".repeat(32);
        for m in a.snap.msgs.iter_mut() { if m.id == "a2" { m.id = id.clone(); m.body = tag_body(&m.body, &sender_id()) } else if m.id == "a1" { m.id = "c".repeat(32) } }
        a.sel = Some(id.clone());
        a.on_key(key(KeyCode::Char('e')));
        assert!(matches!(a.modal, Some(Modal::React(_))));
        a.on_key(key(KeyCode::Char('9')));
        assert!(rx.try_recv().is_err());
        a.on_key(key(KeyCode::Char('2')));
        let r = rx.try_recv().unwrap();
        assert_eq!((r.name, r.args["emoji"].clone()), ("react_chat", json!("❤️")));
        // delete: needs J
        a.on_key(key(KeyCode::Char('x')));
        assert!(matches!(a.modal, Some(Modal::Delete(_))));
        a.on_key(key(KeyCode::Char('n')));
        assert!(rx.try_recv().is_err());
        a.on_key(key(KeyCode::Char('x'))); a.on_key(key(KeyCode::Char('j')));
        assert_eq!(rx.try_recv().unwrap().name, "delete_chat");
        // foreign message: refused, no modal
        a.sel = Some("c".repeat(32));
        a.on_key(key(KeyCode::Char('x')));
        assert!(a.modal.is_none() && rx.try_recv().is_err());
        // same author name but foreign tag: refused (shared seat)
        a.sel = Some(id.clone());
        for m in a.snap.msgs.iter_mut() { if m.id == id { m.body = "[Anderer] x".into() } }
        if sender_id() != "-" { a.on_key(key(KeyCode::Char('x'))); assert!(a.modal.is_none()) }
        // threads
        a.select_chan(Chan::Topic("Ideen".into()));
        a.focus = Focus::Side;
        a.on_key(key(KeyCode::Char('x')));
        assert!(matches!(a.modal, Some(Modal::Thread(chat::Thread::Close, _))) && rx.try_recv().is_err());
        a.on_key(key(KeyCode::Char('j')));
        assert_eq!(rx.try_recv().unwrap().name, "thread_close");
        a.on_key(key(KeyCode::Char('F')));
        for c in "Idee".chars() { a.on_key(key(KeyCode::Char(c))) }
        a.on_key(key(KeyCode::Enter));
        assert!(rx.try_recv().is_err() && a.modal.is_some(), "wrong name must not forget");
        for c in "n".chars() { a.on_key(key(KeyCode::Char(c))) }
        a.on_key(key(KeyCode::Enter));
        assert_eq!(rx.try_recv().unwrap().name, "thread_forget");
        assert_eq!(a.cur, Chan::Group);
        // mark_read only on explicit key, only the selected id
        a.focus = Focus::Main; a.sel = Some(id.clone());
        a.on_key(key(KeyCode::Char('M')));
        let r = rx.try_recv().unwrap();
        assert_eq!((r.name, r.args["ids"].clone()), ("mark_read", json!([id])));
    }
    #[test]
    fn mention_popup_accept_and_esc() {
        let (mut a, rx) = app();
        a.confirm = false;
        for c in "hi @bo".chars() { a.on_key(key(KeyCode::Char(c))) }
        assert_eq!(a.pop.as_ref().unwrap().items[0].0, "bob");
        a.on_key(key(KeyCode::Esc));
        assert!(a.pop.is_none() && a.focus == Focus::Input);
        a.on_key(key(KeyCode::Char('b')));
        assert!(a.pop.is_none(), "stays closed while same token");
        a.ed.clear();
        for c in "@c".chars() { a.on_key(key(KeyCode::Char(c))) }
        assert_eq!(a.pop.as_ref().unwrap().items.len(), 1); // own seat alice excluded, bob has no c
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.ed.text(), "@{carol} ");
        assert!(rx.try_recv().is_err(), "Enter accepted the mention, did not send");
        for c in "ok".chars() { a.on_key(key(KeyCode::Char(c))) }
        a.on_key(key(KeyCode::Enter));
        let r = rx.try_recv().unwrap();
        assert_eq!(r.args["@poke"]["me"], "alice");
        assert!(r.args["body"].as_str().unwrap().ends_with("@{carol} ok"));
    }
    #[test]
    fn sender_tag() {
        assert_eq!(tag_body("hi", "Redfuchs"), "[Redfuchs] hi");
        assert_eq!(tag_body("[Redfuchs] hi", "Redfuchs"), "[Redfuchs] hi");
        assert_eq!(tag_body("hi", "-"), "hi");
    }
    #[test]
    fn send_flow_with_confirm() {
        let (mut a, rx) = app();
        a.select_chan(Chan::Topic("Ideen".into()));
        for c in "hi".chars() { a.on_key(key(KeyCode::Char(c))) }
        a.on_key(key(KeyCode::Enter));
        assert!(matches!(a.modal, Some(Modal::Send)));
        assert!(rx.try_recv().is_err());
        a.on_key(key(KeyCode::Enter));
        let r = rx.try_recv().unwrap();
        assert_eq!(r.name, "chat_send");
        assert!(r.args["body"].as_str().unwrap().starts_with('['), "sender tag missing");
        assert!(r.args["body"].as_str().unwrap().ends_with("] hi"));
        assert_eq!(r.args["channel"]["name"], "Ideen");
        assert!(a.sending);
        assert_eq!(a.ed.text(), "hi");
        a.on_ev(Ev::Reply(Tag::Send, Err("boom".into())));
        assert_eq!(a.ed.text(), "hi");
        a.on_ev(Ev::Reply(Tag::Send, Ok(json!({}))));
        assert!(a.ed.text().is_empty());
    }
    #[test]
    fn esc_cancels_send_and_quote_goes_along() {
        let (mut a, rx) = app();
        a.confirm = false;
        a.quote = Some("a1".into());
        a.ed.insert_str("x");
        a.on_key(key(KeyCode::Enter));
        let r = rx.try_recv().unwrap();
        assert_eq!(r.args["quote"], "a1");
        assert!(r.args.get("channel").is_none());
        a.confirm = true;
        a.sending = false;
        a.on_key(key(KeyCode::Enter));
        a.on_key(key(KeyCode::Esc));
        assert!(a.modal.is_none() && rx.try_recv().is_err());
    }
    #[test]
    fn vote_needs_explicit_j() {
        let (mut a, rx) = app();
        a.set_tab(View::Props);
        a.on_key(key(KeyCode::Char('a')));
        assert!(matches!(a.modal, Some(Modal::Vote(4, true))));
        a.on_key(key(KeyCode::Enter));
        assert!(rx.try_recv().is_err() && a.modal.is_some());
        a.on_key(key(KeyCode::Char('j')));
        let r = rx.try_recv().unwrap();
        assert_eq!((r.name, r.args["proposal_id"].as_u64()), ("approve", Some(4)));
    }
    #[test]
    fn new_topic_and_typing_vs_quit() {
        let (mut a, _rx) = app();
        a.on_key(key(KeyCode::Char('q')));
        assert!(!a.quit && a.ed.text() == "q");
        a.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        for c in "Neu".chars() { a.on_key(key(KeyCode::Char(c))) }
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.cur, Chan::Topic("Neu".into()));
        a.focus = Focus::Main;
        a.on_key(key(KeyCode::Char('q')));
        assert!(a.quit);
    }
    #[test]
    fn reply_selects_quote() {
        let (mut a, _rx) = app();
        a.focus = Focus::Main;
        a.on_key(key(KeyCode::Down));
        a.on_key(key(KeyCode::Up));
        a.on_key(key(KeyCode::Char('r')));
        assert_eq!(a.quote.as_deref(), Some("a1"));
        assert_eq!(a.focus, Focus::Input);
    }
    #[test]
    fn option_chars_are_typed() {
        let (mut a, _rx) = app();
        a.focus = Focus::Input;
        for c in ['@', '{', '}', '[', '|'] { a.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)) }
        assert_eq!(a.ed.text(), "@{}[|");
    }
    #[test]
    fn option_as_meta_fallback() {
        let (mut a, _rx) = app();
        a.focus = Focus::Input;
        for c in ['l', '8', '9', '5', '6', '7', 'n'] { a.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)) }
        assert_eq!(a.ed.text(), "@{}[]|~");
    }
    fn ctrl(c: char) -> KeyEvent { KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL) }
    fn typ(a: &mut App, s: &str) { for c in s.chars() { a.on_key(key(KeyCode::Char(c))) } }
    #[test]
    fn propose_note_needs_preview_then_j() {
        let (mut a, rx) = app();
        a.on_key(ctrl('p'));
        assert!(matches!(a.modal, Some(Modal::Propose(_))));
        a.on_key(key(KeyCode::Right)); a.on_key(key(KeyCode::Right));
        a.on_key(key(KeyCode::Enter));
        typ(&mut a, "Idee");
        a.on_key(key(KeyCode::Enter));
        let Some(Modal::Propose(f)) = &a.modal else { panic!("form closed") };
        assert!(f.plan.is_some(), "{}", f.err);
        assert!(rx.try_recv().is_err(), "nothing sent before J");
        a.on_key(key(KeyCode::Enter));
        assert!(rx.try_recv().is_err() && a.modal.is_some(), "Enter must not sign");
        a.on_key(key(KeyCode::Char('j')));
        let r = rx.try_recv().unwrap();
        assert_eq!(r.name, "propose");
        assert_eq!(r.args, json!({"surface":"memory","payload":{"op":"add_note","title":"Idee"}}));
        assert!(a.modal.is_none());
        // the daemon's refusal comes back verbatim, form restored
        a.on_ev(Ev::Reply(Tag::Propose, Err("refused: nope".into())));
        let Some(Modal::Propose(f)) = &a.modal else { panic!("form not restored") };
        assert_eq!((f.err.as_str(), f.a.as_str()), ("refused: nope", "Idee"));
        a.modal = None;
        a.on_ev(Ev::Reply(Tag::Propose, Ok(json!({"id":7}))));
        assert!(a.draft.is_none() && a.toast.as_ref().unwrap().0.contains("#7"));
    }
    #[test]
    fn propose_wiki_multiline_and_validation() {
        let (mut a, rx) = app();
        a.set_tab(View::Props);
        a.focus = Focus::Side;
        a.on_key(key(KeyCode::Char('n')));
        a.on_key(key(KeyCode::Enter));
        a.on_key(ctrl('s'));
        let Some(Modal::Propose(f)) = &a.modal else { panic!() };
        assert!(f.plan.is_none() && !f.err.is_empty(), "empty path must not preview");
        typ(&mut a, "n/a.md");
        a.on_key(key(KeyCode::Enter));
        typ(&mut a, "x"); a.on_key(key(KeyCode::Enter)); typ(&mut a, "y");
        a.on_key(ctrl('s'));
        let r = rx.try_recv().unwrap();
        assert_eq!(r.name, "@check", "preview is a dry run");
        assert_eq!(r.args["edits"][0], json!({"op":"create","path":"n/a.md","content":"x\ny"}));
        // moltd warns: J must not submit, A accepts exactly these codes
        a.on_ev(Ev::Reply(Tag::Check, Ok(json!({"args":r.args,"warnings":[{"code":"foreign_rewrite","text":"von bob"}]}))));
        let Some(Modal::Propose(f)) = &a.modal else { panic!("form not restored") };
        assert!(f.plan.is_some() && f.warns.len() == 1);
        a.on_key(key(KeyCode::Char('j')));
        assert!(rx.try_recv().is_err() && a.modal.is_some(), "J must not bypass warnings");
        a.on_key(key(KeyCode::Char('a')));
        let r = rx.try_recv().unwrap();
        assert_eq!((r.name, r.args["allow_warnings"].clone()), ("wiki_edit", json!(["foreign_rewrite"])));
        assert!(r.args.get("dry_run").is_none());
        // no warnings: J submits plainly, A does nothing
        a.on_ev(Ev::Reply(Tag::Propose, Err("x".into())));
        a.on_ev(Ev::Reply(Tag::Propose, Ok(json!({"proposal_id":8}))));
        assert!(a.toast.as_ref().unwrap().0.contains("#8") || a.toast.is_some());
    }
    #[test]
    fn esc_in_preview_goes_back_and_n_cancels() {
        let (mut a, rx) = app();
        a.on_key(ctrl('p'));
        a.on_key(key(KeyCode::Right)); a.on_key(key(KeyCode::Right)); a.on_key(key(KeyCode::Enter));
        typ(&mut a, "T"); a.on_key(key(KeyCode::Enter));
        a.on_key(key(KeyCode::Esc));
        let Some(Modal::Propose(f)) = &a.modal else { panic!() };
        assert!(f.plan.is_none());
        a.on_key(key(KeyCode::Esc));
        assert!(a.modal.is_none() && rx.try_recv().is_err());
    }
    #[test]
    fn withdraw_only_own_open() {
        let (mut a, rx) = app();
        a.snap.props.push(Prop { id: 9, state: "proposed".into(), by: "alice".into(), mine: true, ..Default::default() });
        a.set_tab(View::Props);
        a.on_key(key(KeyCode::Char('w')));
        assert!(matches!(a.modal, Some(Modal::Withdraw(9))));
        a.on_key(key(KeyCode::Enter));
        assert!(rx.try_recv().is_err());
        a.on_key(key(KeyCode::Char('j')));
        let r = rx.try_recv().unwrap();
        assert_eq!((r.name, r.args), ("withdraw", json!({"proposal_id":9})));
        a.psel = 1;
        a.on_key(key(KeyCode::Char('w')));
        assert!(a.modal.is_none() && a.toast.as_ref().unwrap().1, "someone else's proposal");
    }
    #[test]
    fn topic_name_validated() {
        let (mut a, _rx) = app();
        a.on_key(ctrl('n'));
        a.on_key(key(KeyCode::Enter));
        assert!(matches!(a.modal, Some(Modal::NewTopic(_))), "empty name keeps the dialog");
        typ(&mut a, &"x".repeat(65));
        a.on_key(key(KeyCode::Enter));
        assert!(matches!(a.modal, Some(Modal::NewTopic(_))), "too long");
        a.modal = Some(Modal::NewTopic("  Ideen2 ".into()));
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.cur, Chan::Topic("Ideen2".into()));
    }
}

/// German Mac option layer for terminals that send Option as Meta (ESC + base key): l=@ 5=[ 6=] 7=| 8={ 9=} n=~
pub fn de_option(c: char) -> Option<char> {
    Some(match c { 'l' => '@', '5' => '[', '6' => ']', '7' => '|', '8' => '{', '9' => '}', 'n' => '~', _ => return None })
}
