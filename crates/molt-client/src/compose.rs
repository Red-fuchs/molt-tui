//! Proposal builders shared by the UIs: one validated form per proposal type, turned into the exact MCP call and a plain-text preview.
//! Nothing here talks to the daemon; the caller sends `Plan.tool` / `Plan.args` only after an explicit confirmation.
use crate::text::{clean, one};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { WikiNew, WikiEdit, Note, Persist, Retention, Name, Charter, Relays, Features }

pub const ALL: [Kind; 9] = [Kind::WikiNew, Kind::WikiEdit, Kind::Note, Kind::Persist, Kind::Retention, Kind::Name, Kind::Charter, Kind::Relays, Kind::Features];

pub const MAX_WIKI: usize = 48_000;
pub const MAX_CHARTER: usize = 8_000;

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::WikiNew => "wiki_new", Kind::WikiEdit => "wiki_edit", Kind::Note => "note", Kind::Persist => "persist",
            Kind::Retention => "retention", Kind::Name => "name", Kind::Charter => "charter", Kind::Relays => "relays", Kind::Features => "features",
        }
    }
    pub fn from_id(s: &str) -> Option<Kind> { ALL.into_iter().find(|k| k.id() == s) }
    pub fn label(self) -> &'static str {
        match self {
            Kind::WikiNew => "Wiki: neue Seite", Kind::WikiEdit => "Wiki: Seite überschreiben", Kind::Note => "Gedächtnis: Notiz",
            Kind::Persist => "Datei dauerhaft machen", Kind::Retention => "Chat-Aufbewahrung (Tage)", Kind::Name => "Name der Republik",
            Kind::Charter => "Charta", Kind::Relays => "Relays (Erweitert)", Kind::Features => "Funktionen aktivieren (Erweitert)",
        }
    }
    pub fn advanced(self) -> bool { matches!(self, Kind::Relays | Kind::Features) }
    /// (label of the one-line field, label of the multi-line field); empty label = field unused.
    pub fn fields(self) -> (&'static str, &'static str) {
        match self {
            Kind::WikiNew | Kind::WikiEdit => ("Pfad (z.B. notizen/idee.md)", "Inhalt (Markdown)"),
            Kind::Note => ("Titel der Notiz", ""),
            Kind::Persist => ("Datei-ID (32 Hex-Zeichen)", ""),
            Kind::Retention => ("Tage (1-3650)", ""),
            Kind::Name => ("Neuer Name", ""),
            Kind::Charter => ("", "Neue Charta"),
            Kind::Relays => ("Relay-URLs, durch Leerzeichen getrennt (ws:// oder wss://)", ""),
            Kind::Features => ("Vollständige Zielmenge, z.B. memory quests", ""),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan { pub tool: &'static str, pub args: Value, pub preview: String }

fn line(label: &str, s: &str, max: usize) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() { return Err(format!("{label} fehlt")) }
    if t.chars().count() > max { return Err(format!("{label} zu lang (max. {max} Zeichen)")) }
    if clean(t) != t || t.contains('\n') { return Err(format!("{label}: Steuerzeichen oder Zeilenumbruch nicht erlaubt")) }
    Ok(t.to_string())
}

pub fn wiki_path(p: &str) -> Result<String, String> {
    let p = line("Pfad", p, 200)?;
    if p.starts_with('/') || p.contains('\\') || p.split('/').any(|s| s.is_empty() || s == "." || s == "..") { return Err("Pfad: nur relativ, ohne .. und leere Teile".into()) }
    if !p.ends_with(".md") { return Err("Pfad muss auf .md enden".into()) }
    Ok(p)
}

fn org(op: &str, value: &str, what: &str) -> Plan {
    Plan { tool: "propose", args: json!({"surface":"organization","payload":{"op":op,"value":value}}),
        preview: format!("Organisation / {op}\n{what}") }
}

/// Validate the form fields (`a` one-line, `b` multi-line) and build the call.
pub fn plan(kind: Kind, a: &str, b: &str) -> Result<Plan, String> {
    match kind {
        Kind::WikiNew | Kind::WikiEdit => {
            let path = wiki_path(a)?;
            let body = clean(b).trim_end().to_string();
            if body.trim().is_empty() { return Err("Inhalt fehlt".into()) }
            if body.len() > MAX_WIKI { return Err(format!("Inhalt zu groß (max. {} KB)", MAX_WIKI / 1000)) }
            let (op, what) = if kind == Kind::WikiNew { ("create", "legt eine neue Seite an") } else { ("content", "ersetzt den GANZEN Inhalt der Seite (oder legt sie an)") };
            Ok(Plan { tool: "wiki_edit", args: json!({"edits":[{"op":op,"path":path,"content":body}]}),
                preview: format!("Wiki / {op} {path}\n{what}\n\n{body}") })
        }
        Kind::Note => {
            let t = line("Titel", a, 200)?;
            Ok(Plan { tool: "propose", args: json!({"surface":"memory","payload":{"op":"add_note","title":t}}), preview: format!("Gedächtnis / add_note\n{t}") })
        }
        Kind::Persist => {
            let id = a.trim().to_lowercase();
            if id.len() != 32 || !id.bytes().all(|c| c.is_ascii_hexdigit()) { return Err("Datei-ID: 32 Hex-Zeichen erwartet".into()) }
            Ok(Plan { tool: "propose", args: json!({"surface":"files","payload":{"op":"persist","id":id}}), preview: format!("Dateien / persist\nDatei {id} dauerhaft speichern (überlebt die Chat-Aufbewahrung)") })
        }
        Kind::Retention => {
            let d: u32 = a.trim().parse().map_err(|_| "Tage: ganze Zahl erwartet".to_string())?;
            if !(1..=3650).contains(&d) { return Err("Tage: 1 bis 3650".into()) }
            Ok(org("set_chat_retention", &d.to_string(), &format!("Chat-Nachrichten und temporäre Dateien {d} Tage aufbewahren")))
        }
        Kind::Name => { let n = line("Name", a, 64)?; Ok(org("set_name", &n, &format!("Neuer Name: {n}"))) }
        Kind::Charter => {
            let c = clean(b).trim().to_string();
            if c.is_empty() { return Err("Charta fehlt".into()) }
            if c.chars().count() > MAX_CHARTER { return Err(format!("Charta zu lang (max. {MAX_CHARTER} Zeichen)")) }
            Ok(org("set_charter", &c, &format!("Neue Charta:\n\n{c}")))
        }
        Kind::Relays => {
            let v = line("Relays", a, 1000)?;
            let urls: Vec<&str> = v.split_whitespace().collect();
            if urls.len() > 12 || urls.iter().any(|u| !(u.starts_with("ws://") || u.starts_with("wss://")) || u.len() < 8) { return Err("Relays: 1 bis 12 URLs mit ws:// oder wss://".into()) }
            let v = urls.join(" ");
            Ok(org("set_relays", &v, &format!("Relay-Liste ERSETZEN durch:\n{}", urls.join("\n"))))
        }
        Kind::Features => {
            let v = line("Funktionen", a, 100)?.to_lowercase();
            let ks: Vec<&str> = v.split_whitespace().collect();
            if ks.is_empty() || ks.iter().any(|k| !["memory", "quests", "vault", "wallet"].contains(k)) { return Err("Funktionen: memory quests vault wallet".into()) }
            let v = ks.join(" ");
            Ok(org("set_features", &v, &format!("Funktionen aktivieren (nur hinzufügen, nie wieder abschaltbar): {v}")))
        }
    }
}

/// Topic names: trimmed, one line, at most 64 chars (moltd: case-sensitive).
pub fn topic_name(s: &str) -> Result<String, String> {
    let n = one(s).trim().to_string();
    if n.is_empty() { return Err("Name fehlt".into()) }
    if n.chars().count() > 64 || n.contains('⏎') { return Err("Name: eine Zeile, höchstens 64 Zeichen".into()) }
    Ok(n)
}

pub fn vote_args(id: u64, note: &str, tag: &str) -> Value {
    let n = clean(note).trim().to_string();
    if n.is_empty() { json!({"proposal_id":id}) } else { json!({"proposal_id":id,"note":crate::model::tag_body(&n, tag)}) }
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn kinds_roundtrip() {
        for k in ALL { assert_eq!(Kind::from_id(k.id()), Some(k)) }
        assert!(Kind::from_id("x").is_none());
    }
    #[test]
    fn wiki() {
        let p = plan(Kind::WikiNew, " notizen/a.md ", "# Hi\n\x1b[31mx").unwrap();
        assert_eq!((p.tool, p.args["edits"][0]["op"].as_str(), p.args["edits"][0]["path"].as_str()), ("wiki_edit", Some("create"), Some("notizen/a.md")));
        assert_eq!(p.args["edits"][0]["content"], "# Hi\nx");
        assert_eq!(plan(Kind::WikiEdit, "a.md", "x").unwrap().args["edits"][0]["op"], "content");
        for bad in ["/a.md", "../a.md", "a/../b.md", "a.txt", "a\\b.md", "a//b.md", "", "a\n.md"] { assert!(plan(Kind::WikiNew, bad, "x").is_err(), "{bad}") }
        assert!(plan(Kind::WikiNew, "a.md", "  \n").is_err());
        assert!(plan(Kind::WikiNew, "a.md", &"x".repeat(MAX_WIKI + 1)).is_err());
    }
    #[test]
    fn others() {
        assert_eq!(plan(Kind::Note, "Idee", "").unwrap().args["payload"], json!({"op":"add_note","title":"Idee"}));
        let id = "AB".repeat(16);
        assert_eq!(plan(Kind::Persist, &id, "").unwrap().args["payload"]["id"], id.to_lowercase());
        assert!(plan(Kind::Persist, "abc", "").is_err());
        assert_eq!(plan(Kind::Retention, "14", "").unwrap().args["payload"], json!({"op":"set_chat_retention","value":"14"}));
        assert!(plan(Kind::Retention, "0", "").is_err() && plan(Kind::Retention, "x", "").is_err());
        assert_eq!(plan(Kind::Name, "Neu", "").unwrap().args["surface"], "organization");
        assert!(plan(Kind::Name, &"x".repeat(65), "").is_err());
        assert_eq!(plan(Kind::Charter, "", "Text\nzwei").unwrap().args["payload"]["op"], "set_charter");
        assert_eq!(plan(Kind::Relays, "wss://a.example  ws://b.example", "").unwrap().args["payload"]["value"], "wss://a.example ws://b.example");
        assert!(plan(Kind::Relays, "http://a.example", "").is_err());
        assert_eq!(plan(Kind::Features, "Memory  quests", "").unwrap().args["payload"]["value"], "memory quests");
        assert!(plan(Kind::Features, "root", "").is_err());
        assert!(Kind::Relays.advanced() && Kind::Features.advanced() && !Kind::Note.advanced());
    }
    #[test]
    fn topics_and_notes() {
        assert_eq!(topic_name("  Ideen \x1b[31m ").unwrap(), "Ideen");
        assert!(topic_name("a\nb").is_err() && topic_name("  ").is_err() && topic_name(&"x".repeat(65)).is_err());
        assert_eq!(vote_args(4, "", "T"), json!({"proposal_id":4}));
        assert_eq!(vote_args(4, " grund ", "T")["note"], "[T] grund");
    }
}
