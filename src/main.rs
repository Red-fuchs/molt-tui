mod app;
mod editor;
mod mcp;
mod model;
mod text;
mod ui;

use app::*;
use crossterm::event::{self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyEventKind,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use mcp::Mcp;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::sync::mpsc;
use std::time::Duration;

fn restore() {
    let mut o = std::io::stdout();
    let _ = execute!(o, PopKeyboardEnhancementFlags, DisableBracketedPaste, DisableMouseCapture, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

fn arg(args: &[String], k: &str) -> Option<String> { args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned()) }

fn dump(args: &[String]) -> i32 {
    let m = match Mcp::new() { Ok(m) => m, Err(e) => { eprintln!("molt-tui: {e}"); return 2 } };
    let snap = match model::fetch_snap(&m) { Ok(s) => s, Err(e) => { eprintln!("molt-tui: moltd nicht erreichbar: {e} (läuft moltd mit [mcp]-Token? prüfe MOLT_URL)"); return 2 } };
    let mut a = App::new(text::tz_offset());
    a.on_ev(Ev::Snap(Ok(snap)));
    match arg(args, "--tab").as_deref() {
        Some("props") => a.tab = View::Props,
        Some("wiki") => a.tab = View::Wiki,
        _ => {}
    }
    if let Some(c) = arg(args, "--chan") {
        let ch = match c.split_once(':') {
            Some(("patch", n)) => model::Chan::Patch(n.parse().unwrap_or(0)),
            Some(("topic", n)) => model::Chan::Topic(n.into()),
            _ => model::Chan::Group,
        };
        a.select_chan(ch);
    }
    if let Some(n) = arg(args, "--sel").and_then(|n| n.parse().ok()) { a.psel = n; a.wsel = n }
    match a.tab {
        View::Props => if let Some(id) = a.sorted_props().get(a.psel).map(|p| p.id) {
            if let Ok(v) = m.call("read_proposal", serde_json::json!({"id":id})) { a.on_ev(Ev::Reply(Tag::Detail(id), Ok(v))) }
        },
        View::Wiki => if let Some(p) = a.snap.docs.get(a.wsel).map(|d| d.path.clone()) {
            if let Ok(v) = m.call("wiki_get", serde_json::json!({"path":p})) { a.on_ev(Ev::Reply(Tag::Doc(p), Ok(v))) }
        },
        _ => {}
    }
    let (w, h) = arg(args, "--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().ok(), b.parse().ok()))).map_or((110, 36), |(w, h)| (w.unwrap_or(110), h.unwrap_or(36)));
    print!("{}", ui::dump(&mut a, w, h));
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // flags override env; single-threaded here, so set_var is sound
    if let Some(u) = arg(&args, "--url") { unsafe { std::env::set_var("MOLT_URL", u) } }
    if let Some(f) = arg(&args, "--token-file") {
        match std::fs::read_to_string(&f) { Ok(t) => unsafe { std::env::set_var("MOLT_TOKEN", t.trim()) }, Err(e) => { eprintln!("molt-tui: {f}: {e}"); std::process::exit(2) } }
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("molt-tui — Terminal-UI für den MoltRepublic-Sitz (moltd)\n\n  molt-tui                         interaktiv\n  molt-tui --dump|--once [--tab chat|props|wiki] [--chan group|patch:N|topic:NAME] [--sel N] [--size WxH]\n                                   einen Frame als Text ausgeben (nur lesende Aufrufe)\n\nConfig (in Reihenfolge): --url/--token-file, env MOLT_URL/MOLT_TOKEN, ~/.config/molt/url + token, Default http://127.0.0.1:4040/mcp\nHilfe in der App: F1 oder ?");
        return;
    }
    if args.iter().any(|a| a == "--dump" || a == "--once") { std::process::exit(dump(&args)) }
    let (poll, worker) = match (Mcp::new(), Mcp::new()) { (Ok(a), Ok(b)) => (a, b), (Err(e), _) | (_, Err(e)) => { eprintln!("molt-tui: {e}"); std::process::exit(2) } };

    let (tx, rx) = mpsc::channel::<Ev>();
    let (rtx, rrx) = mpsc::channel::<Req>();
    let (ntx, nrx) = mpsc::channel::<()>();
    let t1 = tx.clone();
    std::thread::spawn(move || loop {
        if t1.send(Ev::Snap(model::fetch_snap(&poll))).is_err() { break }
        let _ = nrx.recv_timeout(Duration::from_secs(3));
        while nrx.try_recv().is_ok() {}
    });
    std::thread::spawn(move || for r in rrx {
        let res = worker.call(r.name, r.args);
        if tx.send(Ev::Reply(r.tag, res)).is_err() { break }
    });

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |i| { restore(); prev(i) }));
    enable_raw_mode().expect("raw mode");
    let kb = supports_keyboard_enhancement().unwrap_or(false);
    let mut out = std::io::stdout();
    let _ = execute!(out, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste);
    if kb { let _ = execute!(out, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)); }
    let mut term = Terminal::new(CrosstermBackend::new(out)).expect("terminal");

    let mut a = App::new(text::tz_offset());
    a.tx = Some(rtx);
    a.nudge = Some(ntx);
    let mut dirty = true;
    while !a.quit {
        while let Ok(ev) = rx.try_recv() { a.on_ev(ev); dirty = true }
        a.pump();
        if dirty || a.toast.is_some() {
            let _ = term.draw(|f| ui::draw(f, &mut a));
            dirty = false;
        }
        if a.mouse_dirty {
            a.mouse_dirty = false;
            let mut o = std::io::stdout();
            let _ = if a.mouse { execute!(o, EnableMouseCapture) } else { execute!(o, DisableMouseCapture) };
        }
        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => a.on_key(k),
                Ok(Event::Mouse(m)) => a.on_mouse(m),
                Ok(Event::Paste(s)) => a.on_paste(s),
                _ => {}
            }
            dirty = true;
        }
    }
    restore();
}
