mod app;
mod editor;
mod uistate;
mod imglay;
mod imgs;
mod pixel;
mod imglru;
mod viewer;
pub use molt_client::{mcp, model};
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

/// chat_send, then fire-and-forget pokes for @mentions (args carry the roster in "@poke"; it is stripped before the call).
fn send_poking(w: &Mcp, gate: &mut molt_client::poke::PokeGate, mut a: serde_json::Value) -> Result<serde_json::Value, String> {
    use molt_client::{model::Member, poke};
    let ctx = a.as_object_mut().and_then(|o| o.remove("@poke")).unwrap_or_default();
    let roster: Vec<Member> = ctx["roster"].as_array().into_iter().flatten()
        .map(|r| Member { name: r[0].as_str().unwrap_or("").into(), presence: r[1].as_u64().unwrap_or(0), ..Default::default() }).collect();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let o = poke::send_with_pokes(w, gate, &roster, ctx["me"].as_str().unwrap_or(""), ctx["enabled"].as_bool() == Some(true), false, a, now);
    let txt = poke::summary_de(&o);
    o.sent.map(|mut v| { if let (Some(t), Some(m)) = (txt, v.as_object_mut()) { m.insert("@pokes".into(), t.into()); } v })
}

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
    if a.tab == View::Chat && imgs::mode_of(std::env::var("MOLT_IMAGES").ok().as_deref()) != imgs::Mode::Off {
        // read-only: download_file only writes into the local cache; show pixel-art previews of the newest images
        a.imgs.px = true;
        let cache = imgs::cache_dir();
        let mut v: Vec<_> = a.msgs().into_iter().filter(|m| imgs::is_img(m)).cloned().collect();
        let n = v.len().saturating_sub(4);
        for msg in v.drain(n..) {
            if let Some(meta) = msg.file.clone() { let r = imgs::load(&m, &cache, &imgs::ImgReq { id: msg.id.clone(), meta }); a.imgs.on_loaded(msg.id, r) }
        }
    }
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
        println!("molt-tui — Terminal-UI für den MoltRepublic-Sitz (moltd)\n\n  molt-tui                         interaktiv\n  molt-tui --dump|--once [--tab chat|props|wiki] [--chan group|patch:N|topic:NAME] [--sel N] [--size WxH]\n                                   einen Frame als Text ausgeben (nur lesende Aufrufe)\n  molt-tui --diag chain|mirror|wake|log [Filter]|upload ID\n                                   Diagnose (nur lesend; log enthält Peer-IPs)\n\nAbsender-Tag: jede Nachricht beginnt mit [MOLT_ID] (Default Redfuchs, MOLT_ID=- aus)\nConfig (in Reihenfolge): --url/--token-file, env MOLT_URL/MOLT_TOKEN, ~/.config/molt/url + token, Default http://127.0.0.1:4040/mcp\nHilfe in der App: F1 oder ?");
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--diag") {
        let m = match Mcp::new() { Ok(m) => m, Err(e) => { eprintln!("molt-tui: {e}"); std::process::exit(2) } };
        let (w, q) = (args.get(i + 1).map_or("", String::as_str), args.get(i + 2).map_or("", String::as_str));
        match molt_client::diag::view(&m, w, q) { Ok(t) => println!("{t}"), Err(e) => { eprintln!("molt-tui: {e}"); std::process::exit(2) } }
        return;
    }
    if args.iter().any(|a| a == "--dump" || a == "--once") { std::process::exit(dump(&args)) }
    let (poll, worker) = match (Mcp::new(), Mcp::new()) { (Ok(a), Ok(b)) => (a, b), (Err(e), _) | (_, Err(e)) => { eprintln!("molt-tui: {e}"); std::process::exit(2) } };

    let (tx, rx) = mpsc::channel::<Ev>();
    let (rtx, rrx) = mpsc::channel::<Req>();
    let (ntx, nrx) = mpsc::channel::<()>();
    let t1 = tx.clone();
    let (itx, irx) = mpsc::channel::<imgs::ImgReq>();
    match Mcp::new() { Ok(m) => imgs::spawn_worker(m, irx, tx.clone()), Err(_) => {} }
    std::thread::spawn(move || loop {
        if t1.send(Ev::Snap(model::fetch_snap(&poll))).is_err() { break }
        let _ = nrx.recv_timeout(Duration::from_secs(3));
        while nrx.try_recv().is_ok() {}
    });
    let mut gate = molt_client::poke::PokeGate::default();
    std::thread::spawn(move || for r in rrx {
        let res = if r.name == "chat_send" && r.args.get("@poke").is_some() {
            send_poking(&worker, &mut gate, r.args)
        } else if r.name == "@check" {
            let plan = molt_client::compose::Plan { tool: "wiki_edit", args: r.args, preview: String::new() };
            molt_client::compose::check(&worker, plan).map(|c| serde_json::json!({"args":c.plan.args,"warnings":c.warnings.iter().map(|w| serde_json::json!({"code":w.code,"text":w.text})).collect::<Vec<_>>()}))
        } else { worker.call(r.name, r.args) };
        if tx.send(Ev::Reply(r.tag, res)).is_err() { break }
    });

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |i| { restore(); prev(i) }));
    enable_raw_mode().expect("raw mode");
    let kb = supports_keyboard_enhancement().unwrap_or(false);
    let mut out = std::io::stdout();
    let _ = execute!(out, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste);
    let imode = imgs::mode();
    let picker = if imode == imgs::Mode::Graphics { Some(imgs::make_picker()) } else { None };
    if kb { let _ = execute!(out, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)); }
    let mut term = Terminal::new(CrosstermBackend::new(std::io::stdout())).expect("terminal");

    let mut a = App::new(text::tz_offset());
    if let Some(p) = uistate::default_path() { a.load_ui(p) }
    a.tx = Some(rtx);
    a.nudge = Some(ntx);
    if imode != imgs::Mode::Off { a.imgs.tx = Some(itx) }
    a.imgs.px = imode == imgs::Mode::Pixel;
    a.imgs.picker = picker;
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
