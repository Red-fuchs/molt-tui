use molt_client::chat;
use crate::app::*;
use crate::model::*;
use crate::imglay::{self, ImgState};
use crate::imgs::Slot;
use ratatui_image::{protocol::StatefulProtocol, StatefulImage};
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
    if a.imgs.viewer.is_some() { draw_viewer(f, a) }
}

fn draw_viewer(f: &mut Frame, a: &mut App) {
    let area = f.area();
    f.render_widget(Clear, area);
    let [img, foot] = Layout::vertical([Min(1), Length(1)]).areas(area);
    let Some(v) = a.imgs.viewer.clone() else { return };
    let (n, tot) = v.pos();
    let name = a.snap.msgs.iter().find(|m| m.id == v.cur()).and_then(|m| m.file.as_ref()).map_or(String::new(), |f| molt_client::files::safe_name(&f.name));
    if a.imgs.px {
        let c = v.cur().to_string();
        let (iw, ih) = match a.imgs.slots.get(&c) { Some(Slot::Ready { w, h, .. }) => (*w, *h), _ => (0, 0) };
        let (cols, rows) = imglay::fit_cells(iw, ih, 1, 2, img.width, img.height);
        let r = Rect::new(img.x + (img.width - cols) / 2, img.y, cols, rows);
        match a.imgs.pixels(&c) { Some(p) => crate::pixel::render(p, r, f.buffer_mut()), None => f.render_widget(Paragraph::new(sp(" Bild wird geladen …", DIM)), img) }
    } else { match a.imgs.proto(v.cur()) {
        Some(p) => f.render_stateful_widget(StatefulImage::<StatefulProtocol>::default(), img, p),
        None => f.render_widget(Paragraph::new(sp(" Bild wird geladen …", DIM)), img),
    } }
    f.render_widget(Paragraph::new(sp(format!(" {n}/{tot}  {name}  ←/→ blättern  Esc schließen"), DIM)), foot);
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

fn draw_side(f: &mut Frame, a: &mut App, r: Rect) {
    let b = blk(" Kanäle ".into(), a.focus == Focus::Side && a.tab == View::Chat);
    let inner = b.inner(r);
    f.render_widget(b, r);
    let rows = a.side_rows();
    let h = inner.height as usize;
    let selr = rows.iter().position(|x| match (x, a.hdr) { (Row::Head(s), Some(h)) => *s == h, (Row::Ent(i), None) => a.entries[*i].chan == a.cur, _ => false }).unwrap_or(0);
    let off = if selr >= h { selr + 1 - h } else { 0 };
    let mut lines = vec![];
    for (k, row) in rows.into_iter().enumerate().skip(off).take(h) {
        let y = inner.y + (k - off) as u16;
        let w = inner.width as usize;
        let i = match row {
            Row::Head(sec) => {
                let col = a.is_col(sec);
                let un = if col { a.sec_unread(sec) } else { 0 };
                let badge = if un > 0 { format!(" +{un}") } else { String::new() };
                let on = a.hdr == Some(sec);
                let st = if on { DIM.bg(Color::Rgb(50, 56, 70)) } else { DIM };
                let lab = trunc(&format!("{} {}", if col { "▸" } else { "▾" }, crate::uistate::SECS[sec].1), w.saturating_sub(tw(&badge)));
                let pad = w.saturating_sub(tw(&lab) + tw(&badge));
                a.hits.push((Rect::new(inner.x, y, inner.width, 1), Hit::Head(sec)));
                lines.push(Line::from(vec![sp(lab, st), sp(" ".repeat(pad), st), sp(badge, st.fg(Color::Yellow).add_modifier(Modifier::BOLD))]));
                continue;
            }
            Row::Ent(i) => i,
        };
        let e = &a.entries[i];
        let (label, sel, un) = (e.label.clone(), a.cur == e.chan && a.hdr.is_none(), e.unread);
        a.hits.push((Rect::new(inner.x, y, inner.width, 1), Hit::Chan(i)));
        let badge = if un > 0 { format!(" {un}") } else { String::new() };
        let lab = trunc(&label, w.saturating_sub(3 + tw(&badge)));
        let mut st = Style::new();
        if un > 0 { st = st.add_modifier(Modifier::BOLD) }
        if sel { st = st.bg(Color::Rgb(50, 56, 70)) }
        let pad = w.saturating_sub(2 + tw(&lab) + tw(&badge));
        lines.push(Line::from(vec![sp(if sel { "▸ " } else { "  " }, st.fg(Color::Cyan)), sp(lab, st), sp(" ".repeat(pad), st),
            sp(badge, st.fg(Color::Yellow).add_modifier(Modifier::BOLD))]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

pub type Blk = (String, usize, usize, u16, u16);

pub fn msg_lines(a: &App, wd: usize) -> (Vec<Line<'static>>, Vec<(String, usize, usize)>, Vec<Blk>) {
    let mut lines: Vec<Line<'static>> = vec![];
    let mut ranges = vec![];
    let mut blks: Vec<Blk> = vec![];
    for m in a.msgs() {
        let start = lines.len();
        let sel = a.sel.as_deref() == Some(m.id.as_str());
        let bg = if sel { Style::new().bg(Color::Rgb(40, 44, 58)) } else { Style::new() };
        let bar = |s: &str| sp(s.to_string(), if sel { Style::new().fg(Color::Cyan) } else { Style::new() });
        let mut h = vec![bar(if sel { "▌" } else { " " }), sp(format!("{} ", fmt_ts(m.ts, a.off)), DIM), sp(m.from.clone(), who(&m.from, a.me()))];
        let mine = m.reacted(a.me());
        for (e, n) in &m.reactions { h.push(sp(format!("  {e}{}", if *n > 1 { format!("×{n}") } else { String::new() }), if mine.contains(&e.as_str()) { Style::new().fg(Color::Cyan) } else { DIM })) }
        if m.deleted { h.push(sp("  (gelöscht)", DIM)) }
        lines.push(Line::from(h).style(bg));
        if let Some(q) = &m.quote {
            let t = match a.snap.msgs.iter().find(|x| &x.id == q) {
                Some(x) => format!("↪ {}: {}", x.from, one(&x.body)),
                None => "↪ (zitierte Nachricht nicht mehr vorhanden)".into(),
            };
            lines.push(Line::from(vec![bar(" "), sp(format!(" {}", trunc(&t, wd.saturating_sub(3))), DIM.add_modifier(Modifier::ITALIC))]).style(bg));
        }
        for l in wrap(&m.body, wd.saturating_sub(4).max(1)) { lines.push(Line::from(vec![bar(" "), sp(format!("  {l}"), Style::new())]).style(bg)) }
        if let Some(f) = &m.file {
            let name = molt_client::files::safe_name(&f.name);
            if crate::imgs::is_img(m) {
                let slot = a.imgs.slots.get(&m.id);
                let gif = matches!(slot, Some(Slot::Ready { gif: true, .. })) || name.to_ascii_lowercase().ends_with(".gif");
                let st = if a.imgs.on { a.imgs.state(&m.id) } else { ImgState::Off };
                lines.push(Line::from(vec![bar(" "), sp(format!("  {}", imglay::caption(&name, f.size, gif, &st)), DIM)]).style(bg));
                if let (true, Some(Slot::Ready { w, h, .. })) = (a.imgs.on, slot) {
                    let (c, r) = imglay::fit_cells(*w, *h, a.imgs.font().0, a.imgs.font().1, wd.saturating_sub(4).min(if a.imgs.px { 100 } else { u16::MAX as usize }) as u16, if a.imgs.px { 22 } else { 10 });
                    if r > 0 {
                        let s = lines.len();
                        for _ in 0..r { lines.push(Line::from(bar(" ")).style(bg)) }
                        blks.push((m.id.clone(), s, lines.len(), c, r));
                    }
                }
            } else {
                lines.push(Line::from(vec![bar(" "), sp(format!("  📎 {name} ({})", imglay::human_size(f.size)), DIM)]).style(bg));
            }
        }
        ranges.push((m.id.clone(), start, lines.len()));
        lines.push(Line::default());
    }
    (lines, ranges, blks)
}

fn draw_chat(f: &mut Frame, a: &mut App, area: Rect) {
    let sw = if a.sidebar { (area.width / 3).clamp(24, 44) } else { 0 };
    let [side, main] = Layout::horizontal([Length(sw), Min(10)]).areas(area);
    if a.sidebar { draw_side(f, a, side) }
    let b = blk(format!(" {} ", a.cur.name()), a.focus == Focus::Main);
    let inner = b.inner(main);
    f.render_widget(b, main);
    a.main_rect = inner;
    let (lines, ranges, blks) = msg_lines(a, inner.width as usize);
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
    if a.modal.is_none() && !a.help && a.imgs.viewer.is_none() {
        for (id, s, e, c, r) in &blks {
            if imglay::vis(*s, *e, top, h) != imglay::Vis::Full { continue }
            let rect = Rect::new(inner.x + 3, inner.y + (*s - top) as u16, (*c).min(inner.width.saturating_sub(3)), *r);
            if a.imgs.px { if let Some(p) = a.imgs.pixels(id) { crate::pixel::render(p, rect, f.buffer_mut()) } }
            else if let Some(p) = a.imgs.proto(id) { f.render_stateful_widget(StatefulImage::<StatefulProtocol>::default(), rect, p) }
        }
    }
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
    if let (Some(p), true) = (&a.pop, focus) { draw_pop(f, p, r) }
}

/// @mention list directly above the input box.
fn draw_pop(f: &mut Frame, p: &Pop, input: Rect) {
    let h = (p.items.len() as u16 + 2).min(input.y.max(3));
    let wd = (p.items.iter().map(|i| tw(&i.0) + 4).max().unwrap_or(10) as u16 + 4).clamp(16, input.width);
    let r = Rect::new(input.x, input.y.saturating_sub(h), wd, h);
    f.render_widget(Clear, r);
    let b = Block::bordered().title(" @ Mitglied (↑↓ Tab/Enter, Esc) ").border_style(Style::new().fg(Color::Cyan));
    let inner = b.inner(r);
    f.render_widget(b, r);
    let lines: Vec<Line> = p.items.iter().enumerate().take(inner.height as usize).map(|(i, (n, on))| {
        let st = if i == p.sel { Style::new().bg(Color::Blue).fg(Color::White) } else { Style::new() };
        Line::from(vec![sp(if *on { "● " } else { "○ " }, if *on { st.fg(Color::Green) } else { DIM }), sp(n.clone(), st)])
    }).collect();
    f.render_widget(Paragraph::new(lines), inner);
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
        (View::Chat, Focus::Input) => "Enter senden · Alt+Enter Zeile · Tab Fokus · Ctrl+N Thema · Ctrl+B Liste · F1 Hilfe",
        (View::Chat, Focus::Main) => "↑↓ Nachricht · r antworten · e reagieren · x löschen · u Freigabe weg · m/M gelesen · i schreiben · ? Hilfe",
        (View::Chat, Focus::Side) => "↑↓ Kanal · Enter öffnen/auf-zuklappen · z alle ein/aus · Ctrl+B Liste · n neu · x schließen · o öffnen · F vergessen · m gelesen · ? Hilfe",
        (View::Props, _) => "↑↓ wählen · a zustimmen · d ablehnen · n neu · w zurückziehen · t Diskussion · ? Hilfe",
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
        Modal::Withdraw(id) => {
            let s = a.snap.props.iter().find(|p| p.id == *id).map(|p| format!("{} {}", p.op, p.summary)).unwrap_or_default();
            (format!(" Vorschlag #{id} zurückziehen "), vec![
                Line::from(sp(format!("Eigenen Vorschlag #{id} wirklich zurückziehen?"), BOLD.fg(Color::Yellow))), Line::from(sp(s, DIM)), Line::default(),
                Line::from("Er wird für alle Mitglieder beendet und lässt sich nicht wieder öffnen."), Line::default(),
                Line::from(sp("J = ja, zurückziehen · N/Esc = abbrechen", BOLD))], Color::Red)
        }
        Modal::React(_) => (" Reagieren ".into(), vec![
            Line::from(chat::REACTIONS.iter().enumerate().map(|(i, e)| sp(format!(" {} {e} ", i + 1), BOLD)).collect::<Vec<_>>()), Line::default(),
            Line::from(sp("Zahl wählt (nochmal dasselbe = zurücknehmen) · Esc abbrechen", DIM))], Color::Cyan),
        Modal::Delete(id) => {
            let t = a.snap.msgs.iter().find(|m| &m.id == id).map(|m| trunc(&one(&m.body), 70)).unwrap_or_default();
            (" Nachricht löschen ".into(), vec![
                Line::from(sp("Eigene Nachricht wirklich LÖSCHEN?", BOLD.fg(Color::Yellow))), Line::from(sp(t, DIM)), Line::default(),
                Line::from("Der Text wird für alle Mitglieder gelöscht und durch einen Hinweis ersetzt. Nicht rückgängig."), Line::default(),
                Line::from(sp("J = ja, löschen · N/Esc = abbrechen", BOLD))], Color::Red)
        }
        Modal::Unshare(id) => {
            let t = a.snap.msgs.iter().find(|m| &m.id == id).and_then(|m| m.file.as_ref().map(|f| f.name.clone())).unwrap_or_default();
            (" Freigabe zurückziehen ".into(), vec![
                Line::from(sp("Eigene Dateifreigabe wirklich zurückziehen?", BOLD.fg(Color::Yellow))), Line::from(sp(trunc(&one(&t), 70), DIM)), Line::default(),
                Line::from("Die Datei ist danach für niemanden mehr abrufbar."), Line::default(),
                Line::from(sp("J = ja, zurückziehen · N/Esc = abbrechen", BOLD))], Color::Red)
        }
        Modal::Thread(op, n) => {
            let (t, d) = if *op == chat::Thread::Close { ("schließen", "Das Thema nimmt keine Nachrichten mehr an und endet nach einem Tag; jedes Mitglied kann es vorher wieder öffnen (o).") } else { ("wieder öffnen", "Das Thema nimmt wieder Nachrichten an.") };
            (format!(" Thema {t} "), vec![
                Line::from(sp(format!("Thema \"{}\" für ALLE Mitglieder {t}?", trunc(&one(n), 50)), BOLD.fg(Color::Yellow))), Line::default(),
                Line::from(d), Line::default(), Line::from(sp("J = ja · N/Esc = abbrechen", BOLD))], Color::Red)
        }
        Modal::Forget(n, t) => (" Thema vergessen (nur dieser Sitz) ".into(), vec![
            Line::from(sp(format!("Thema \"{}\" auf DIESEM Sitz vergessen?", trunc(&one(n), 50)), BOLD.fg(Color::Yellow))), Line::default(),
            Line::from("Alle lokalen Nachrichten des Themas werden gelöscht. Die anderen Mitglieder behalten es."),
            Line::from("Zur Bestätigung den Namen des Themas eintippen:"),
            Line::from(vec![sp(format!("> {t}"), Style::new()), sp("▏", Style::new().fg(Color::Cyan))]), Line::default(),
            Line::from(sp("Enter bestätigen (Name muss stimmen) · Esc abbrechen", BOLD))], Color::Red),
        Modal::Propose(p) => { draw_propose(f, p); return }
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

fn draw_propose(f: &mut Frame, p: &PropForm) {
    let cur = |on: bool| if on { sp("▏", Style::new().fg(Color::Cyan)) } else { sp("", Style::new()) };
    let lbl = |on: bool, s: &str| Line::from(sp(s.to_string(), if on { BOLD.fg(Color::Cyan) } else { DIM }));
    let mut body: Vec<Line> = vec![];
    let title = if let Some(pl) = &p.plan {
        body.push(Line::from(sp("Vorschau: genau das wird gesendet", BOLD.fg(Color::Yellow))));
        body.push(Line::default());
        let pv: Vec<&str> = pl.preview.lines().collect();
        for l in pv.iter().take(12) { body.push(Line::from(trunc(l, 84))) }
        if pv.len() > 12 { body.push(Line::from(sp(format!("… (+{} Zeilen)", pv.len() - 12), DIM))) }
        body.push(Line::default());
        for w in &p.warns { body.push(Line::from(sp(format!("WARNUNG {}: {}", w.code, trunc(&w.text, 70)), Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)))) }
        body.push(Line::from("Vorschlag einreichen = signierte Aktion: echte Signatur, nicht rückgängig zu machen."));
        let key = if p.warns.is_empty() { "J = verbindlich einreichen" } else { "A = Warnungen akzeptieren und einreichen" };
        body.push(Line::from(sp(format!("{key} · Esc zurück zum Formular · N abbrechen"), BOLD)));
        " Vorschlag prüfen · ECHTE SIGNATUR "
    } else {
        let k = p.kind();
        let (la, lb) = k.fields();
        body.push(lbl(p.field == 0, "Typ (← → wählen)"));
        body.push(Line::from(vec![sp(format!("◀ {} ▶", k.label()), if p.field == 0 { BOLD } else { Style::new() }), sp(if k.advanced() { "  ERWEITERT: wirkt auf die ganze Republik" } else { "" }, Style::new().fg(Color::Yellow))]));
        if !la.is_empty() { body.push(lbl(p.field == 1, la)); body.push(Line::from(vec![sp(format!("> {}", p.a), Style::new()), cur(p.field == 1)])) }
        if !lb.is_empty() {
            body.push(lbl(p.field == 2, &format!("{lb} (Enter = neue Zeile)")));
            let mut s: Vec<char> = p.b.t.clone();
            s.insert(p.b.cur.min(s.len()), '▏');
            let txt: String = s.into_iter().collect();
            let ls: Vec<&str> = txt.split('\n').collect();
            let first = ls.iter().position(|l| l.contains('▏')).unwrap_or(0).saturating_sub(7);
            for l in ls.iter().skip(first).take(8) { body.push(Line::from(trunc(l, 84))) }
        }
        if !p.err.is_empty() { body.push(Line::default()); body.push(Line::from(sp(p.err.clone(), Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)))) }
        body.push(Line::default());
        body.push(Line::from(sp("Tab Feld · Ctrl+S Vorschau (noch nichts gesendet) · Esc abbrechen", BOLD)));
        " Vorschlag erstellen "
    };
    let area = centered(f.area(), 90, (body.len() as u16 + 2).min(f.area().height));
    f.render_widget(Clear, area);
    let b = Block::bordered().title(title).border_style(Style::new().fg(if p.plan.is_some() { Color::Red } else { Color::Cyan }).add_modifier(Modifier::BOLD));
    f.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }).block(b), area);
}

fn draw_help(f: &mut Frame) {
    let k = |a: &str, b: &str| Line::from(vec![sp(format!(" {a:<22}"), BOLD.fg(Color::Cyan)), sp(b.to_string(), Style::new())]);
    let h = |t: &str| Line::from(sp(format!(" {t}"), BOLD.fg(Color::Yellow)));
    let lines = vec![
        h("Allgemein"),
        k("F1 / ?", "Hilfe"), k("F3 F4 F5 / 1 2 3", "Chat · Vorschläge · Wiki (Tabs auch klickbar)"),
        k("Tab / Shift+Tab", "Fokus: Kanäle → Nachrichten → Eingabe"), k("Ctrl+N / Ctrl+P", "neues Thema / neuer Vorschlag"),
        k("Ctrl+B / F8", "Kanalliste aus-/einblenden"), k("z (Kanäle)", "alle Bereiche ein-/ausklappen; Enter/Space/Klick auf Kopf klappt einen"), k("Ctrl+R", "jetzt aktualisieren"), k("F6", "Rückfrage vor dem Senden an/aus"), k("F7", "Bilder im Chat an/aus (Enter/o = Vollbild)"),
        k("Ctrl+C / q", "beenden (q nur wenn du nicht tippst)"),
        h("Maus"),
        k("F2", "Maus an/aus. Bei an: Klicken/Scrollen; zum Markieren+Kopieren mit dem Terminal"),
        k("", "Option-Ziehen (iTerm/Terminal.app) oder Shift-Ziehen, oder F2 drücken"),
        k("Klick / Rechtsklick", "Kanal/Nachricht wählen / Nachricht kopieren"),
        k("Mausrad", "Nachrichten, Diff, Wiki scrollen"),
        h("Nachrichten (Fokus Mitte)"),
        k("↑ ↓ PgUp PgDn End", "wählen / scrollen / nach unten"), k("r oder Enter", "auf Nachricht antworten (Zitat)"),
        k("c / C", "Text / mit Absender nach pbcopy"), k("m / M", "Kanal / gewählte Nachricht als gelesen markieren"),
        k("e", "reagieren (1-6, nochmal = zurücknehmen)"), k("x", "eigene Nachricht löschen (J)"), k("u", "eigene Dateifreigabe zurückziehen (J)"),
        k("Kanäle: x / o / F", "Thema schließen / öffnen (J) / lokal vergessen (Name tippen)"),
        h("Eingabe"),
        k("Enter", "senden (mit Rückfrage)"), k("Alt+Enter, Shift+Enter, Ctrl+J", "neue Zeile"),
        k("← → ↑ ↓ Home End", "Cursor (Shift = markieren)"), k("Alt+← → (Ctrl+← →)", "wortweise"),
        k("Ctrl+A / E", "Zeilenanfang / -ende"), k("Ctrl+K / U / W", "löschen bis Ende / Anfang / Wort zurück"),
        k("Ctrl+Z / Ctrl+Y", "rückgängig / wiederholen"), k("Ctrl+X, Alt+C, Ctrl+V", "ausschneiden, kopieren, einfügen (Cmd+V geht auch)"),
        k("Alt+A", "alles markieren"), k("Esc", "Zitat entfernen / Fokus nach oben"),
        h("Vorschläge"),
        k("↑ ↓", "Vorschlag wählen (links) / Diff scrollen (rechts)"), k("a / d", "zustimmen / ablehnen: echte Signatur, nur nach J"),
        k("t", "Diskussion des Vorschlags öffnen"),
        k("n / Ctrl+P", "Vorschlag erstellen (Vorschau, dann J = signieren)"), k("w", "eigenen offenen Vorschlag zurückziehen (J)"),
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
            let c = &buf[(x, y)];
            let s = c.symbol();
            match (s, c.fg, c.bg) {
                ("▀", Color::Indexed(t), Color::Indexed(b)) => line.push(crate::pixel::half(t, b)),
                ("▀", Color::Rgb(a, b, c2), Color::Rgb(d, e, g)) => line.push(crate::pixel::half_rgb([a, b, c2], [d, e, g])),
                _ => line.push_str(s),
            }
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
    fn mention_popup_renders() {
        let mut a = app();
        for c in "@".chars() { a.ed.insert_str(&c.to_string()) }
        a.update_pop();
        let s = dump(&mut a, 100, 30);
        assert!(s.contains("@ Mitglied") && s.contains("● bob") && !s.contains("● alice"), "{s}");
    }
    #[test]
    fn image_caption_in_dump_and_no_fetch() {
        // needs files::is_image/safe_name (Qwen package) - red until filled
        let mut snap = fixtures::snap();
        let mut m = snap.msgs[0].clone();
        m.id = "9".repeat(32); m.ts += 99; m.body = String::new(); m.kind = "file".into();
        m.file = Some(FileMeta { name: "../x\u{1b}y.PNG".into(), kind: "image".into(), size: 2048, checksum: String::new() });
        snap.msgs.push(m);
        let mut a = App::new(0);
        a.on_ev(Ev::Snap(Ok(snap)));
        let s = dump(&mut a, 100, 30);
        assert!(s.contains("🖼 xy.PNG (2.0 KB)") && !s.contains("wird geprüft"), "{s}");
        assert!(a.imgs.slots.is_empty(), "dump never fetches");
    }
    #[test]
    fn pixel_art_preview_in_dump() {
        let mut snap = fixtures::snap();
        let mut m = snap.msgs[0].clone();
        m.id = "8".repeat(32); m.ts += 99; m.body = String::new(); m.kind = "file".into();
        m.file = Some(FileMeta { name: "p.png".into(), kind: "image".into(), size: 100, checksum: String::new() });
        snap.msgs.push(m.clone());
        let mut a = App::new(0);
        a.on_ev(Ev::Snap(Ok(snap)));
        a.imgs.px = true;
        let mut im = image::RgbImage::new(16, 8);
        for (x, _, p) in im.enumerate_pixels_mut() { *p = if x < 8 { image::Rgb([255, 255, 255]) } else { image::Rgb([0, 0, 0]) } }
        a.imgs.on_loaded(m.id.clone(), Ok((image::DynamicImage::ImageRgb8(im), false)));
        let s = dump(&mut a, 100, 40);
        assert!(s.contains("p.png") && s.contains("████████"), "{s}");
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
    #[test]
    fn propose_modal_frames() {
        use molt_client::compose;
        let mut a = app();
        a.modal = Some(Modal::Propose(Box::new(PropForm::new())));
        let d = dump(&mut a, 110, 36);
        assert!(d.contains("Vorschlag erstellen") && d.contains("Wiki: neue Seite") && d.contains("Ctrl+S"), "{d}");
        if let Some(Modal::Propose(f)) = &mut a.modal { f.a = "x/y.md".into(); f.b.insert_str("# Titel"); f.plan = compose::plan(f.kind(), &f.a, &f.b.text()).ok() }
        let d = dump(&mut a, 110, 36);
        assert!(d.contains("ECHTE SIGNATUR") && d.contains("x/y.md") && d.contains("J = verbindlich"), "{d}");
        a.modal = Some(Modal::Withdraw(4));
        assert!(dump(&mut a, 110, 36).contains("zurückziehen"));
    }
}
