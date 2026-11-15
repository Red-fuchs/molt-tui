//! @mention helpers (pure): complete, token_at, insert, extract. Indices are CHAR indices (the TUI editor works on Vec<char>).
use crate::model::Member;
use std::ops::Range;

pub type Roster = [Member];

fn fc(c: char) -> char {
    match c { 'ä' | 'Ä' => 'a', 'ö' | 'Ö' => 'o', 'ü' | 'Ü' => 'u', 'ß' => 's', c => c.to_lowercase().next().unwrap_or(c) }
}
/// Case- and umlaut-insensitive, length-preserving fold.
pub fn fold(s: &str) -> Vec<char> { s.chars().map(fc).collect() }
fn word(c: char) -> bool { c.is_alphanumeric() || c == '_' }
fn find(h: &[char], n: &[char]) -> bool { n.is_empty() || h.windows(n.len()).any(|w| w == n) }

/// Matches for `prefix`: prefix hits before substring hits, online first within a rank, own seat excluded.
pub fn complete<'a>(prefix: &str, roster: &'a Roster, me: &str) -> Vec<&'a Member> {
    let (p, me) = (fold(prefix.trim_start_matches('{')), fold(me));
    let mut v: Vec<(u8, &Member)> = roster.iter().filter(|m| fold(&m.name) != me).filter_map(|m| {
        let n = fold(&m.name);
        if n.starts_with(&p) { Some((0, m)) } else if find(&n, &p) { Some((1, m)) } else { None }
    }).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0).then((b.1.presence > 0).cmp(&(a.1.presence > 0))).then_with(|| fold(&a.1.name).cmp(&fold(&b.1.name))));
    v.into_iter().map(|x| x.1).collect()
}

/// Active `@token` ending at `cursor`: (char range incl. '@', query without '@'/'{'). `@{...` may contain spaces/parens until '}'.
pub fn token_at(text: &str, cursor: usize) -> Option<(Range<usize>, String)> {
    let cs: Vec<char> = text.chars().collect();
    let cur = cursor.min(cs.len());
    let mut i = cur;
    while i > 0 {
        i -= 1;
        let c = cs[i];
        if c == '@' {
            if i > 0 && word(cs[i - 1]) { return None } // e-mail style
            let q = &cs[i + 1..cur];
            let braced = q.first() == Some(&'{');
            if braced && q.contains(&'}') { return None }
            if !braced && q.iter().any(|c| c.is_whitespace()) { return None }
            let q: String = q.iter().skip(braced as usize).collect();
            return Some((i..cur, q));
        }
        if c == '\n' || c == '}' { return None }
    }
    None
}

/// Replace `range` by canonical `@{name}` (+ a space unless one follows). Returns (new text, new cursor).
pub fn insert(text: &str, range: Range<usize>, name: &str) -> (String, usize) {
    let cs: Vec<char> = text.chars().collect();
    let (a, b) = (range.start.min(cs.len()), range.end.min(cs.len()));
    let ins = format!("@{{{name}}}");
    let sp = !cs.get(b).is_some_and(|c| c.is_whitespace());
    let mut o: String = cs[..a].iter().collect();
    o.push_str(&ins);
    if sp { o.push(' ') }
    o.extend(cs[b..].iter());
    (o, a + ins.chars().count() + sp as usize)
}

/// Result of scanning a body: resolved canonical names + ambiguous short names (typed token, candidates).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Mentions { pub names: Vec<String>, pub ambiguous: Vec<(String, Vec<String>)> }

/// Roster names whose first word (or part before " (") starts with the folded token.
fn short<'a>(tok: &[char], roster: &'a Roster) -> Vec<&'a str> {
    roster.iter().filter(|m| {
        let n = fold(&m.name);
        let w: Vec<char> = n.iter().take_while(|c| word(**c)).cloned().collect();
        let head = n.windows(2).position(|x| x == [' ', '(']).map(|p| &n[..p]);
        !m.name.is_empty() && (w.starts_with(tok) || head.is_some_and(|h| h.starts_with(tok)))
    }).map(|m| m.name.as_str()).collect()
}

/// Roster names mentioned in `body` (document order, deduped, canonical spelling). `@{Name}` exact; `@Name` at a word boundary
/// (longest full name wins, else a UNIQUE short name like `@Fren` -> "Fren (Peter)"; 2+ candidates -> reported ambiguous).
pub fn resolve(body: &str, roster: &Roster) -> Mentions {
    let cs: Vec<char> = body.chars().collect();
    let mut names: Vec<(Vec<char>, &str)> = roster.iter().filter(|m| !m.name.is_empty()).map(|m| (fold(&m.name), m.name.as_str())).collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.0.len()));
    let mut out = Mentions::default();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] != '@' || (i > 0 && word(cs[i - 1])) { i += 1; continue }
        let rest: Vec<char> = cs[i + 1..].iter().map(|c| fc(*c)).collect();
        let hit = if rest.first() == Some(&'{') {
            rest.iter().position(|c| *c == '}').and_then(|e| names.iter().find(|n| n.0 == rest[1..e]).map(|n| (n.1, e + 1)))
        } else if let Some(h) = names.iter().find(|n| rest.starts_with(&n.0) && !rest.get(n.0.len()).is_some_and(|c| word(*c) || *c == '-')) {
            Some((h.1, h.0.len()))
        } else {
            let l = rest.iter().take_while(|c| word(**c)).count();
            if l == 0 || rest.get(l) == Some(&'-') { None } else {
                let c = short(&rest[..l], roster);
                match c.len() {
                    0 => None,
                    1 => Some((c[0], l)),
                    _ => {
                        let t: String = cs[i + 1..i + 1 + l].iter().collect();
                        if !out.ambiguous.iter().any(|a| fold(&a.0) == fold(&t)) { out.ambiguous.push((t, c.iter().map(|s| s.to_string()).collect())) }
                        i += 1 + l; continue
                    }
                }
            }
        };
        match hit {
            Some((n, l)) => { if !out.names.iter().any(|o| o == n) { out.names.push(n.to_string()) } i += 1 + l }
            None => i += 1,
        }
    }
    out
}

pub fn extract(body: &str, roster: &Roster) -> Vec<String> { resolve(body, roster).names }

#[cfg(test)]
mod t {
    use super::*;
    fn r() -> Vec<Member> {
        let m = |n: &str, p| Member { name: n.into(), presence: p, ..Default::default() };
        vec![m("Fren (Peter)", 1), m("bob", 0), m("Bobby", 1), m("Jürgen", 0), m("alice", 1), m("Al", 0)]
    }
    #[test]
    fn complete_ranks_and_excludes() {
        let r = r();
        let n = |p: &str| complete(p, &r, "alice").iter().map(|m| m.name.clone()).collect::<Vec<_>>();
        assert_eq!(n("bo"), ["Bobby", "bob"]); // prefix, online first
        assert_eq!(n("b"), ["Bobby", "bob"]);
        assert_eq!(n("ob"), ["Bobby", "bob"]); // substring
        assert_eq!(n("jur"), ["Jürgen"]); // umlaut
        assert_eq!(n("PETER"), ["Fren (Peter)"]);
        assert!(!n("").contains(&"alice".to_string()));
        assert_eq!(n("")[0], "Bobby"); // online first overall
        assert!(n("ali").is_empty()); // own seat excluded
        assert_eq!(n("{fre"), ["Fren (Peter)"]);
    }
    #[test]
    fn complete_prefix_before_substring() {
        let r = r();
        let v: Vec<_> = complete("b", &r, "x").iter().map(|m| m.name.clone()).collect();
        assert_eq!(&v[..2], ["Bobby", "bob"]);
        let r2 = vec![Member { name: "abc".into(), presence: 1, ..Default::default() }, Member { name: "bcd".into(), presence: 0, ..Default::default() }];
        assert_eq!(complete("b", &r2, "x")[0].name, "bcd");
    }
    #[test]
    fn token() {
        assert_eq!(token_at("hi @bo", 6), Some((3..6, "bo".into())));
        assert_eq!(token_at("hi @", 4), Some((3..4, "".into())));
        assert_eq!(token_at("hi @{Fren (Pe", 13), Some((3..13, "Fren (Pe".into())));
        assert_eq!(token_at("hi @{Fren (Peter)} x", 20), None);
        assert_eq!(token_at("hi @bob x", 9), None); // space ended it
        assert_eq!(token_at("a@b.de", 6), None);
        assert_eq!(token_at("hi @bo\nzeile", 12), None);
        assert_eq!(token_at("@äb", 3), Some((0..3, "äb".into())));
        assert_eq!(token_at("hi @bob", 5), Some((3..5, "b".into()))); // cursor mid-word
    }
    #[test]
    fn ins() {
        assert_eq!(insert("hi @Fr", 3..6, "Fren (Peter)"), ("hi @{Fren (Peter)} ".into(), 19));
        assert_eq!(insert("@b x", 0..2, "bob"), ("@{bob} x".into(), 6));
        assert_eq!(insert("ä @b", 2..4, "bob").0, "ä @{bob} ");
    }
    #[test]
    fn extract_boundaries() {
        let r = r();
        assert_eq!(extract("hi @{Fren (Peter)} und @bob!", &r), ["Fren (Peter)", "bob"]);
        assert_eq!(extract("@bobby ist da", &r), ["Bobby"]); // not bob
        assert_eq!(extract("@bobo", &r), Vec::<String>::new());
        assert_eq!(extract("mail a@bob.de", &r), Vec::<String>::new());
        assert_eq!(extract("@fren (peter) hi @FREN (PETER)", &r), ["Fren (Peter)"]);
        assert_eq!(extract("@jurgen und @Jürgen", &r), ["Jürgen"]);
        assert_eq!(extract("@{nobody} @{bob}", &r), ["bob"]);
        assert_eq!(extract("@Al, @alice", &r), ["Al", "alice"]);
        assert!(extract("@alice-x", &r).is_empty());
    }
    #[test]
    fn short_names() {
        let r = r();
        let m = |n: &str, p| Member { name: n.into(), presence: p, ..Default::default() };
        assert_eq!(extract("hi @Fren,", &r), ["Fren (Peter)"]);
        assert_eq!(extract("@fren!", &r), ["Fren (Peter)"]);
        assert_eq!(resolve("@jur", &r).names, ["Jürgen"]);
        assert_eq!(resolve("@JÜRG ok", &r).names, ["Jürgen"]);
        assert!(extract("a@Fren.de x@fren", &r).is_empty()); // email / inside word
        assert!(extract("@Frenn @xyz", &r).is_empty());
        assert_eq!(extract("@{Fren (Peter)}", &r), ["Fren (Peter)"]);
        assert!(extract("@{Fren}", &r).is_empty()); // canonical stays exact
        let mut r2 = r.clone(); r2.push(m("Frank", 1));
        let x = resolve("@Fr, @Fren", &r2);
        assert!(x.names.is_empty() || x.names == ["Fren (Peter)"]);
        let x = resolve("hi @Fr,", &r2);
        assert!(x.names.is_empty());
        assert_eq!(x.ambiguous, vec![("Fr".to_string(), vec!["Fren (Peter)".to_string(), "Frank".to_string()])]);
        assert_eq!(resolve("@Fren", &r2).names, ["Fren (Peter)"]);
    }
}
