use crate::app::*;
use crate::model::*;
use crate::text::{fmt_ts, one, trunc, w as tw, wrap};
use ratatui::layout::{Constraint::*, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;
use std::time::Duration;

const DIM: Style = Style::new().fg(Color::DarkGray);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

fn blk(title: String, focus: bool) -> Block<'static> {
    Block::bordered().title(title).border_style(Style::new().fg(if focus { Color::Cyan } else { Color::DarkGray }))
}
fn sp(s: impl Into<String>, st: Style) -> Span<'static> { Span::styled(s.into(), st) }
fn who(name: &str, me: &str) -> Style {
    if name == me { return Style::new().fg(Color::Green).add_modifier(Modifier::BOLD) }
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let c = [Color::Cyan, Color::Magenta, Color::Yellow, Color::LightBlue, Color::LightRed, Color::LightMagenta][h as usize % 6];
    Style::new().fg(c).add_modifier(Modifier::BOLD)
}
fn diff_style(l: &str) -> Style {
    if l.starts_with("+++") || l.starts_with("---") || l.starts_with("diff ") || l.starts_with("index ") { DIM }
    else if l.starts_with('+') { Style::new().fg(Color::Green) }
    else if l.starts_with('-') { Style::new().fg(Color::Red) }
    else if l.starts_with("@@") { Style::new().fg(Color::Cyan) }
    else { Style::new() }
}

pub fn draw(f: &mut Frame, a: &mut App) {
    a.hits.clear();
    let area = f.area();
    let chat = a.tab == View::Chat;
    a.in_w = (area.width as usize).saturating_sub(4).max(2);
    let ih = if chat { a.ed.rows(a.in_w).len().clamp(1, 6) as u16 + 2 } else { 0 };
    let eh = if a.err.is_some() || !a.loaded { 1 } else { 0 };
    let [top, tabs, banner, body, input, foot] =
        Layout::vertical([Length(1), Length(1), Length(eh), Min(3), Length(ih), Length(1)]).areas(area);
    draw_top(f, a, top);
    draw_tabs(f, a, tabs);
    if eh > 0 { draw_banner(f, a, banner) }
    match a.tab {
        View::Chat => draw_chat(f, a, body),
        View::Props => draw_props(f, a, body),
        View::Wiki => draw_wiki(f, a, body),
    }
    if chat { draw_input(f, a, input) }
    draw_foot(f, a, foot);
    if let Some(m) = &a.modal { draw_modal(f, a, m) }
    if a.help { draw_help(f) }
}

fn draw_top(f: &mut Frame, a: &App, r: Rect) {
    let s = &a.snap.st;
    let mut v = vec![sp(format!(" {} ", if s.name.is_empty() { "MoltRepublic" } else { &s.name }), BOLD.fg(Color::Cyan)),
        sp(format!("· Sitz: {} ", s.member), Style::new())];
    let n = a.snap.acts.len();
    if n > 0 { v.push(sp(format!("· {n} Aktion(en) für dich "), BOLD.fg(Color::Green))) }
    v.push(if s.missing_relays > 0 { sp(format!("· Relays: {} fehlen ", s.missing_relays), Style::new().fg(Color::Yellow)) }
        else { sp(format!("· Relays: ok ({}) ", s.relays), Style::new().fg(Color::Green)) });
    if s.diverged { v.push(sp("· Chain divergiert ", Style::new().fg(Color::Red))) }
    else if s.lag > 0 { v.push(sp(format!("· Chain hinkt {} hinterher ", s.lag), Style::new().fg(Color::Yellow))) }
    v.push(sp(format!("· {} Mitglieder: {} (aktiv 1h: {}) ", s.members.len(), s.members.join(", "), s.active_1h), DIM));
    f.render_widget(Paragraph::new(Line::from(v)), r);
}

fn draw_tabs(f: &mut Frame, a: &mut App, r: Rect) {
    let mut x = r.x + 1;
    let mut v = vec![sp(" ", Style::new())];
    let unread: u64 = a.entries.iter().map(|e| e.unread).sum();
    let todo = a.snap.props.iter().filter(|p| p.todo()).count();
    for (t, l) in [(View::Chat, format!("1 Chat{}", if unread > 0 { format!(" ({unread})") } else { String::new() })),
        (View::Props, format!("2 Vorschläge{}", if todo > 0 { format!(" ({todo})") } else { String::new() })), (View::Wiki, "3 Wiki".into())] {
        let st = if a.tab == t { Style::new().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD) } else { Style::new() };
        let lab = format!(" {l} ");
        let wd = tw(&lab) as u16;
        a.hits.push((Rect::new(x, r.y, wd, 1), Hit::Tab(t)));
        x += wd + 1;
        v.push(sp(lab, st));
        v.push(sp(" ", Style::new()));
    }
    f.render_widget(Paragraph::new(Line::from(v)), r);
}

fn draw_banner(f: &mut Frame, a: &App, r: Rect) {
    let (t, st) = match &a.err {
        Some(e) => (format!(" ⚠ moltd nicht erreichbar: {e} — läuft moltd mit [mcp]-Token? prüfe MOLT_URL · Versuch {}, wiederhole alle 3 s", a.fails),
            Style::new().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD)),
        None => (" Lade Zustand von moltd …".into(), Style::new().fg(Color::Black).bg(Color::Yellow)),
    };
    let t = trunc(&t, r.width as usize);
    f.render_widget(Paragraph::new(Line::from(sp(format!("{t:<w$}", w = r.width as usize), st))), r);
}

fn side_rows(a: &App) -> Vec<(Option<usize>, String, bool, u64)> {
    let mut v = vec![];
    let mut last = 255;
    for (i, e) in a.entries.iter().enumerate() {
        let sec = match e.section { 0 => 0, 1 => 1, _ => 2 };
        if sec != last && sec > 0 {
            v.push((None, if sec == 1 { "Themen".to_string() } else { "Vorschläge".to_string() }, false, 0));
        }
        last = sec;
        v.push((Some(i), e.label.clone(), a.cur == e.chan, e.unread));
    }
    v
}

fn draw_side(f: &mut Frame, a: &mut App, r: Rect) {
    let b = blk(" Kanäle ".into(), a.focus == Focus::Side && a.tab == View::Chat);
    let inner = b.inner(r);
    f.render_widget(b, r);
    let rows = side_rows(a);
    let h = inner.height as usize;
    let selr = rows.iter().position(|r| r.2).unwrap_or(0);
    let off = if selr >= h { selr + 1 - h } else { 0 };
    let mut lines = vec![];
    for (k, (idx, label, sel, un)) in rows.into_iter().enumerate().skip(off).take(h) {
        let y = inner.y + (k - off) as u16;
        let Some(i) = idx else { lines.push(Line::from(sp(format!(" ── {label}"), DIM))); continue };
        a.hits.push((Rect::new(inner.x, y, inner.width, 1), Hit::Chan(i)));
        let badge = if un > 0 { format!(" {un}") } else { String::new() };
        let lab = trunc(&label, (inner.width as usize).saturating_sub(3 + tw(&badge)));
        let mut st = Style::new();
        if un > 0 { st = st.add_modifier(Modifier::BOLD) }
        if sel { st = st.bg(Color::Rgb(50, 56, 70)) }
        let pad = (inner.width as usize).saturating_sub(2 + tw(&lab) + tw(&badge));
        lines.push(Line::from(vec![sp(if sel { "▸ " } else { "  " }, st.fg(Color::Cyan)), sp(lab, st), sp(" ".repeat(pad), st),
            sp(badge, st.fg(Color::Yellow).add_modifier(Modifier::BOLD))]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

pub fn msg_lines(a: &App, wd: usize) -> (Vec<Line<'static>>, Vec<(String, usize, usize)>) {
    let mut lines: Vec<Line<'static>> = vec![];
    let mut ranges = vec![];
    for m in a.msgs() {
        let start = lines.len();
        let sel = a.sel.as_deref() == Some(m.id.as_str());
        let bg = if sel { Style::new().bg(Color::Rgb(40, 44, 58)) } else { Style::new() };
        let bar = |s: &str| sp(s.to_string(), if sel { Style::new().fg(Color::Cyan) } else { Style::new() });
        let mut h = vec![bar(if sel { "▌" } else { " " }), sp(format!("{} ", fmt_ts(m.ts, a.off)), DIM), sp(m.from.clone(), who(&m.from, a.me()))];
        for (e, n) in &m.reactions { h.push(sp(format!("  {e}{}", if *n > 1 { format!("×{n}") } else { String::new() }), DIM)) }
        lines.push(Line::from(h).style(bg));
        if let Some(q) = &m.quote {
            let t = match a.snap.msgs.iter().find(|x| &x.id == q) {
                Some(x) => format!("↪ {}: {}", x.from, one(&x.body)),
                None => "↪ (zitierte Nachricht nicht mehr vorhanden)".into(),
            };
            lines.push(Line::from(vec![bar(" "), sp(format!(" {}", trunc(&t, wd.saturating_sub(3))), DIM.add_modifier(Modifier::ITALIC))]).style(bg));
        }
        for l in wrap(&m.body, wd.saturating_sub(4).max(1)) { lines.push(Line::from(vec![bar(" "), sp(format!("  {l}"), Style::new())]).style(bg)) }
        ranges.push((m.id.clone(), start, lines.len()));
        lines.push(Line::default());
    }
    (lines, ranges)
}

fn draw_chat(f: &mut Frame, a: &mut App, area: Rect) {
    let sw = (area.width / 3).clamp(24, 44);
    let [side, main] = Layout::horizontal([Length(sw), Min(10)]).areas(area);
    draw_side(f, a, side);
    let b = blk(format!(" {} ", a.cur.name()), a.focus == Focus::Main);
    let inner = b.inner(main);
    f.render_widget(b, main);
    a.main_rect = inner;
    let (lines, ranges) = msg_lines(a, inner.width as usize);
    let (h, total) = (inner.height as usize, lines.len());
    if lines.is_empty() {
        let t = if matches!(a.cur, Chan::Topic(_)) && a.msgs().is_empty() { "Neues Thema: die erste Nachricht legt es an." } else { "Noch keine Nachrichten in diesem Kanal." };
        f.render_widget(Paragraph::new(Line::from(sp(format!(" {t}"), DIM))), inner);
        return;
    }
    let maxs = total.saturating_sub(h);
    if a.scroll > 0 && a.last_total > 0 && total > a.last_total { a.scroll += total - a.last_total }
    a.last_total = total;
    if a.reveal {
        if let Some((_, s, e)) = ranges.iter().find(|r| Some(&r.0) == a.sel.as_ref()) {
            let top = maxs.saturating_sub(a.scroll.min(maxs));
            if *s < top { a.scroll = maxs - s } else if *e > top + h { a.scroll = maxs.saturating_sub(e - h) }
        }
        a.reveal = false;
    }
    a.scroll = a.scroll.min(maxs);
    let top = maxs - a.scroll;
    for (id, s, e) in &ranges {
        for l in (*s).max(top)..(*e).min(top + h) {
            a.hits.push((Rect::new(inner.x, inner.y + (l - top) as u16, inner.width, 1), Hit::Msg(id.clone())));
        }
    }
    f.render_widget(Paragraph::new(lines[top..(top + h).min(total)].to_vec()), inner);
    if a.scroll > 0 {
        let t = format!(" ↓ {} Zeilen weiter unten (End) ", a.scroll);
        let wd = tw(&t) as u16;
        f.render_widget(Paragraph::new(sp(t, Style::new().fg(Color::Black).bg(Color::Yellow))), Rect::new(inner.x + inner.width.saturating_sub(wd), inner.y + inner.height - 1, wd.min(inner.width), 1));
    }
}

fn draw_input(f: &mut Frame, a: &mut App, r: Rect) {
    let focus = a.focus == Focus::Input;
    let mut title = format!(" Nachricht an {} ", a.cur.name());
    if let Some(q) = &a.quote {
        let t = a.snap.msgs.iter().find(|m| &m.id == q).map(|m| format!("{}: {}", m.from, one(&m.body))).unwrap_or_default();
        title = format!(" ↪ Antwort auf {} (Esc: Zitat entfernen) ", trunc(&t, 50));
    }
    let b = blk(title, focus).title_bottom(Line::from(sp(" Enter senden · Alt+Enter Zeilenumbruch ", DIM)).right_aligned());
    let inner = b.inner(r);
    f.render_widget(b, r);
    a.in_rect = inner;
    a.hits.push((inner, Hit::Input));
    let rows = a.ed.rows(a.in_w);
    let h = inner.height as usize;
    let (cr, cc) = a.ed.pos(&rows, a.ed.cur);
    if cr < a.in_top { a.in_top = cr } else if cr >= a.in_top + h { a.in_top = cr + 1 - h }
    a.in_top = a.in_top.min(rows.len().saturating_sub(h));
    if a.ed.t.is_empty() {
        f.render_widget(Paragraph::new(sp(if a.sending { "Sende …" } else { "Nachricht schreiben …  (F1: Hilfe)" }, DIM)), inner);
    } else {
        let sel = a.ed.sel();
        let selst = Style::new().bg(Color::Blue).fg(Color::White);
        let mut lines = vec![];
        for &(s, e) in rows.iter().skip(a.in_top).take(h) {
            let seg = |x: usize, y: usize, st: Style| sp(a.ed.t[x..y].iter().collect::<String>(), st);
            let mut v = vec![];
            match sel {
                Some((p, q)) if p < e && q > s => {
                    let (x, y) = (p.max(s), q.min(e));
                    v.push(seg(s, x, Style::new())); v.push(seg(x, y, selst)); v.push(seg(y, e, Style::new()));
                }
                _ => v.push(seg(s, e, Style::new())),
            }
            lines.push(Line::from(v));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }
    if focus && a.modal.is_none() && !a.help {
        f.set_cursor_position(Position::new(inner.x + (cc as u16).min(inner.width.saturating_sub(1)), inner.y + (cr - a.in_top) as u16));
    }
}

fn prop_lines(a: &App, p: &Prop, wd: usize) -> Vec<Line<'static>> {
    let d = a.details.get(&p.id);
    let p = d.unwrap_or(p);
    let mut v = vec![
        Line::from(vec![sp(format!("Vorschlag #{}", p.id), BOLD.fg(Color::Cyan)), sp(format!("  {}  [{}]", p.op, p.surface), Style::new())]),
        Line::from(format!("von {} · Status: {} · Zustimmung {}/{}", p.by, state_de(&p.state), p.approvals, p.threshold)),
    ];
    let mut vs = vec![sp("Stimmen: ", Style::new())];
    for (m, vt) in &p.votes {
        let (sym, st) = match vt.as_str() { "approved" => ("✓", Style::new().fg(Color::Green)), "declined" => ("✗", Style::new().fg(Color::Red)), _ => ("·", DIM) };
        vs.push(sp(format!("{m} "), Style::new())); vs.push(sp(format!("{sym}   "), st.add_modifier(Modifier::BOLD)));
    }
    v.push(Line::from(vs));
    if p.todo() { v.push(Line::from(sp("→ Deine Stimme fehlt: a = zustimmen, d = ablehnen (mit Bestätigung)", Style::new().fg(Color::Green)))) }
    else if p.open() { v.push(Line::from(sp("Du hast bereits abgestimmt.", DIM))) }
    if let Some(s) = &p.superseded { v.push(Line::from(sp(format!("superseded: {s}"), Style::new().fg(Color::Yellow)))) }
    if !p.paths.is_empty() { v.push(Line::from(format!("Pfade: {}", p.paths.join(", ")))) }
    if !p.summary.is_empty() { v.push(Line::from(format!("Zusammenfassung: {}", p.summary))) }
    v.push(Line::default());
    if d.is_none() { v.push(Line::from(sp("Lade Details …", DIM))); return v }
    if !p.value.is_empty() {
        for l in p.value.split('\n') { for w in wrap(l, wd) { v.push(Line::from(sp(w, diff_style(l)))) } }
    } else if !p.proposed.is_empty() || !p.current.is_empty() {
        for (t, x) in [("Vorher", &p.current), ("Nachher", &p.proposed)] {
            v.push(Line::from(sp(t, BOLD)));
            for l in wrap(x, wd) { v.push(Line::from(l)) }
            v.push(Line::default());
        }
    }
    v
}

fn list_pane(f: &mut Frame, a: &mut App, r: Rect, title: &str, items: Vec<(String, bool, Style)>, sel: usize, focus: bool, mk: fn(usize) -> Hit) {
    let b = blk(format!(" {title} "), focus);
    let inner = b.inner(r);
    f.render_widget(b, r);
    let h = inner.height as usize;
    let off = if sel >= h { sel + 1 - h } else { 0 };
    let mut lines = vec![];
    for (k, (t, _, st)) in items.iter().enumerate().skip(off).take(h) {
        let y = inner.y + (k - off) as u16;
        a.hits.push((Rect::new(inner.x, y, inner.width, 1), mk(k)));
        let s = if k == sel { st.bg(Color::Rgb(50, 56, 70)) } else { *st };
        let t = trunc(t, inner.width as usize - 2);
        lines.push(Line::from(vec![sp(if k == sel { "▸ " } else { "  " }, s.fg(Color::Cyan)), sp(format!("{t:<w$}", w = inner.width as usize - 2), s)]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn scroll_pane(f: &mut Frame, a: &mut App, r: Rect, title: String, lines: impl Fn(usize) -> Vec<Line<'static>>, wiki: bool) {
    let b = blk(title, a.focus == Focus::Main);
    let inner = b.inner(r);
    f.render_widget(b, r);
    a.main_rect = inner;
    let ls = lines(inner.width as usize - 1);
    let maxs = ls.len().saturating_sub(inner.height as usize);
    let sc = if wiki { &mut a.wscroll } else { &mut a.pscroll };
    *sc = (*sc).min(maxs);
    let top = *sc;
    f.render_widget(Paragraph::new(ls[top..(top + inner.height as usize).min(ls.len())].to_vec()), inner);
}

fn draw_props(f: &mut Frame, a: &mut App, area: Rect) {
    let [l, r] = Layout::horizontal([Length((area.width / 3).clamp(30, 46)), Min(10)]).areas(area);
    let ps: Vec<Prop> = a.sorted_props().into_iter().cloned().collect();
    let items = ps.iter().map(|p| (
        format!("#{} {} {}/{} {} {}", p.id, state_de(&p.state), p.approvals, p.threshold, p.op, p.summary),
        p.todo(), if p.todo() { BOLD.fg(Color::Green) } else if p.open() { Style::new() } else { DIM })).collect();
    let focus = a.focus == Focus::Side;
    let sel = a.psel;
    list_pane(f, a, l, "Vorschläge (offen zuerst)", items, sel, focus, Hit::Prop);
    match ps.get(sel) {
        Some(p) => { let a2: &App = a; let ls = move |wd: usize| prop_lines(a2, p, wd); 
            let lines = ls(r.width.saturating_sub(3) as usize);
            scroll_pane(f, a, r, format!(" #{} ", p.id), move |_| lines.clone(), false) }
        None => f.render_widget(Paragraph::new(sp(" Keine Vorschläge.", DIM)), r),
    }
}

fn draw_wiki(f: &mut Frame, a: &mut App, area: Rect) {
    let [l, r] = Layout::horizontal([Length((area.width / 3).clamp(26, 40)), Min(10)]).areas(area);
    let items = a.snap.docs.iter().map(|d| (format!("{}  — {}", d.path, d.title), false, Style::new())).collect();
    let (focus, sel) = (a.focus == Focus::Side, a.wsel);
    list_pane(f, a, l, "Wiki", items, sel, focus, Hit::Doc);
    let Some(d) = a.snap.docs.get(sel).cloned() else { f.render_widget(Paragraph::new(sp(" Wiki leer.", DIM)), r); return };
    let wd = r.width.saturating_sub(3) as usize;
    let lines: Vec<Line<'static>> = match a.docs.get(&d.path) {
        None => vec![Line::from(sp("Lade …", DIM))],
        Some(c) => {
            let mut fm = 0;
            let mut v = vec![];
            for l in c.split('\n') {
                if l.trim() == "---" { fm += 1 }
                let st = if fm == 1 || l.trim() == "---" { DIM } else if l.starts_with('#') { BOLD.fg(Color::Cyan) } else { Style::new() };
                for w in wrap(l, wd) { v.push(Line::from(sp(w, st))) }
            }
            v
        }
    };
    scroll_pane(f, a, r, format!(" {} · {} ", d.path, d.title), move |_| lines.clone(), true);
}

fn draw_foot(f: &mut Frame, a: &App, r: Rect) {
    if let Some((t, err, at)) = &a.toast {
        if at.elapsed() < Duration::from_secs(if *err { 8 } else { 4 }) {
            let st = if *err { Style::new().fg(Color::White).bg(Color::Red) } else { Style::new().fg(Color::Black).bg(Color::Green) };
            f.render_widget(Paragraph::new(sp(format!(" {}", trunc(t, r.width as usize - 1)), st)), r);
            return;
        }
    }
    let h = match (a.tab, a.focus) {
        (View::Chat, Focus::Input) => "Enter senden · Alt+Enter Zeile · Tab Fokus · Ctrl+N Thema · F1 Hilfe",
        (View::Chat, Focus::Main) => "↑↓ Nachricht · r antworten · c kopieren · m gelesen · i schreiben · ? Hilfe",
        (View::Chat, Focus::Side) => "↑↓ Kanal · Enter öffnen · n neues Thema · m gelesen · ? Hilfe",
        (View::Props, _) => "↑↓ wählen/scrollen · a zustimmen · d ablehnen · t Diskussion · Tab Fokus · ? Hilfe",
        (View::Wiki, _) => "↑↓ Dokument/scrollen · Enter lesen · Tab Fokus · ? Hilfe",
    };
    let m = format!(" Maus {}(F2) ", if a.mouse { "an " } else { "aus " });
    let wd = r.width as usize;
    let pad = wd.saturating_sub(tw(h) + 1 + tw(&m));
    f.render_widget(Paragraph::new(Line::from(vec![sp(format!(" {h}{}", " ".repeat(pad)), DIM), sp(m, if a.mouse { DIM } else { Style::new().fg(Color::Yellow) })])), r);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn draw_modal(f: &mut Frame, a: &App, m: &Modal) {
    let (title, body, col): (String, Vec<Line>, Color) = match m {
        Modal::Send => {
            let t = a.ed.text();
            (" Senden? ".into(), vec![
                Line::from(format!("Senden als {} in {}?", a.me(), a.cur.name())), Line::default(),
                Line::from(sp(trunc(&one(&t), 70), DIM)), Line::default(),
                Line::from(sp("Enter/J senden · A senden und nicht mehr fragen · Esc/N abbrechen", BOLD))], Color::Cyan)
        }
        Modal::Vote(id, ap) => {
            let s = a.snap.props.iter().find(|p| p.id == *id).map(|p| format!("{} {}", p.op, p.summary)).unwrap_or_default();
            let verb = if *ap { "ZUSTIMMEN" } else { "ABLEHNEN" };
            (format!(" ECHTE SIGNATUR · Vorschlag #{id} "), vec![
                Line::from(sp(format!("Vorschlag #{id} wirklich {verb}?"), BOLD.fg(Color::Yellow))), Line::from(sp(s, DIM)), Line::default(),
                Line::from(format!("Das ist eine echte, kryptografisch signierte Stimme von {} und geht an alle Mitglieder. Sie lässt sich nicht zurücknehmen.", a.me())),
                Line::default(),
                Line::from(sp(format!("J = ja, {} · N/Esc = abbrechen", verb.to_lowercase()), BOLD))], Color::Red)
        }
        Modal::NewTopic(t) => (" Neues Thema ".into(), vec![
            Line::from("Name des Themas (max. 64 Zeichen); es entsteht mit der ersten Nachricht:"), Line::default(),
            Line::from(vec![sp(format!("> {t}"), Style::new()), sp("▏", Style::new().fg(Color::Cyan))]), Line::default(),
            Line::from(sp("Enter anlegen · Esc abbrechen", BOLD))], Color::Cyan),
    };
    let area = centered(f.area(), 76, body.len() as u16 + 6);
    f.render_widget(Clear, area);
    let b = Block::bordered().title(title).border_style(Style::new().fg(col).add_modifier(Modifier::BOLD));
    f.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }).block(b), area);
}

fn draw_help(f: &mut Frame) {
    let k = |a: &str, b: &str| Line::from(vec![sp(format!(" {a:<22}"), BOLD.fg(Color::Cyan)), sp(b.to_string(), Style::new())]);
    let h = |t: &str| Line::from(sp(format!(" {t}"), BOLD.fg(Color::Yellow)));
    let lines = vec![
        h("Allgemein"),
        k("F1 / ?", "Hilfe"), k("F3 F4 F5 / 1 2 3", "Chat · Vorschläge · Wiki (Tabs auch klickbar)"),
        k("Tab / Shift+Tab", "Fokus: Kanäle → Nachrichten → Eingabe"), k("Ctrl+N", "neues Thema"),
        k("Ctrl+R", "jetzt aktualisieren"), k("F6", "Rückfrage vor dem Senden an/aus"),
        k("Ctrl+C / q", "beenden (q nur wenn du nicht tippst)"),
        h("Maus"),
        k("F2", "Maus an/aus. Bei an: Klicken/Scrollen; zum Markieren+Kopieren mit dem Terminal"),
        k("", "Option-Ziehen (iTerm/Terminal.app) oder Shift-Ziehen, oder F2 drücken"),
        k("Klick / Rechtsklick", "Kanal/Nachricht wählen / Nachricht kopieren"),
        k("Mausrad", "Nachrichten, Diff, Wiki scrollen"),
        h("Nachrichten (Fokus Mitte)"),
        k("↑ ↓ PgUp PgDn End", "wählen / scrollen / nach unten"), k("r oder Enter", "auf Nachricht antworten (Zitat)"),
        k("c / C", "Text / mit Absender nach pbcopy"), k("m", "Kanal als gelesen markieren"),
        h("Eingabe"),
        k("Enter", "senden (mit Rückfrage)"), k("Alt+Enter, Shift+Enter, Ctrl+J", "neue Zeile"),
        k("← → ↑ ↓ Home End", "Cursor (Shift = markieren)"), k("Alt+← → (Ctrl+← →)", "wortweise"),
        k("Ctrl+A / E", "Zeilenanfang / -ende"), k("Ctrl+K / U / W", "löschen bis Ende / Anfang / Wort zurück"),
        k("Ctrl+Z / Ctrl+Y", "rückgängig / wiederholen"), k("Ctrl+X, Alt+C, Ctrl+V", "ausschneiden, kopieren, einfügen (Cmd+V geht auch)"),
        k("Alt+A", "alles markieren"), k("Esc", "Zitat entfernen / Fokus nach oben"),
        h("Vorschläge"),
        k("↑ ↓", "Vorschlag wählen (links) / Diff scrollen (rechts)"), k("a / d", "zustimmen / ablehnen: echte Signatur, nur nach J"),
        k("t", "Diskussion des Vorschlags öffnen"),
        Line::default(), Line::from(sp(" Esc / F1 schließt", DIM)),
    ];
    let area = centered(f.area(), 92, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(Block::bordered().title(" Hilfe · molt-tui ").border_style(Style::new().fg(Color::Cyan))), area);
}

pub fn dump(a: &mut App, wd: u16, ht: u16) -> String {
    use ratatui::{backend::TestBackend, Terminal};
    use unicode_width::UnicodeWidthStr;
    let mut t = Terminal::new(TestBackend::new(wd, ht)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    let buf = t.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..ht {
        let mut line = String::new();
        let mut x = 0;
        while x < wd {
            let s = buf[(x, y)].symbol();
            line.push_str(s);
            x += (s.width() as u16).max(1);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod t {
    use super::*;
    use crate::model::fixtures;
    fn app() -> App { let mut a = App::new(0); a.on_ev(Ev::Snap(Ok(fixtures::snap()))); a }
    #[test]
    fn chat_frame() {
        let mut a = app();
        let s = dump(&mut a, 100, 30);
        assert!(s.contains("Demo Republic") && s.contains("Sitz: alice"), "{s}");
        assert!(s.contains("1 Aktion(en) für dich"));
        assert!(s.contains("Allgemein") && s.contains("Themen") && s.contains("Ideen") && s.contains("#4 (offen) wiki_patch"));
        assert!(s.contains("Hallo zusammen,") && s.contains("ROT evil"), "{s}");
        assert!(!s.contains('\x1b') && !s.contains('\u{202e}'));
        assert!(s.contains("↪ bob: Hallo zusammen, ⏎ wie läuft's?"), "{s}");
        assert!(s.contains("❤️"));
    }
    #[test]
    fn error_banner() {
        let mut a = app();
        a.on_ev(Ev::Snap(Err("Connection refused".into())));
        let s = dump(&mut a, 120, 20);
        assert!(s.contains("moltd nicht erreichbar: Connection refused") && s.contains("prüfe MOLT_URL"), "{s}");
        assert!(s.contains("Versuch 1"));
    }
    #[test]
    fn props_and_dialog() {
        let mut a = app();
        a.tab = View::Props;
        let mut p = a.snap.props[1].clone();
        p.value = "diff --git a/home.md b/home.md\n@@ -1 +1 @@\n-alt\n+neu".into();
        a.details.insert(4, p);
        let s = dump(&mut a, 110, 26);
        assert!(s.contains("Vorschlag #4") && s.contains("Zustimmung 1/2") && s.contains("+neu") && s.contains("-alt"), "{s}");
        assert!(s.contains("alice ·") && s.contains("carol ✗"), "{s}");
        a.modal = Some(Modal::Vote(4, true));
        let s = dump(&mut a, 110, 26);
        assert!(s.contains("ECHTE SIGNATUR") && s.contains("nicht zurücknehmen"), "{s}");
    }
    #[test]
    fn wiki_frame() {
        let mut a = app();
        a.tab = View::Wiki;
        a.docs.insert("home.md".into(), "---\ntitle: x\n---\n# Kopf\ntext".into());
        let s = dump(&mut a, 100, 20);
        assert!(s.contains("home.md  — Startseite") && s.contains("# Kopf"), "{s}");
    }
    #[test]
    fn help_and_wrap_input() {
        let mut a = app();
        a.help = true;
        let s = dump(&mut a, 110, 50);
        assert!(s.contains("Option-Ziehen") && s.contains("Alt+Enter"), "{s}");
        a.help = false;
        a.ed.insert_str(&"wort ".repeat(40));
        let s = dump(&mut a, 60, 24);
        assert!(s.matches("wort").count() > 10, "{s}");
    }
    #[test]
    fn scroll_sticks_bottom() {
        let mut a = app();
        let s = dump(&mut a, 100, 12);
        assert!(s.contains("noch weiter geht"), "{s}");
    }
}
