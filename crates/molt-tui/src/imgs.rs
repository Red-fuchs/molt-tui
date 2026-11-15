//! Inline image glue: state, background loader (download + verify + decode), protocol cache.
use crate::imglay::ImgState;
use crate::imglru::Lru;
use crate::viewer::Viewer;
use crate::model::{FileMeta, Msg};
use image::DynamicImage;
use molt_client::{fetch, files, mcp::Mcp};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;

pub const MAX_BYTES: u64 = 15 * 1024 * 1024;
pub const MAX_PX: u32 = 1600;
const WANT: usize = 12;

pub struct ImgReq { pub id: String, pub meta: FileMeta }
pub type Decoded = Result<(DynamicImage, bool), String>;

#[derive(Clone, Debug)]
pub enum Slot { Loading, Failed(String), Ready { w: u32, h: u32, gif: bool } }

pub struct Imgs {
    pub on: bool,
    pub picker: Option<Picker>,
    lru: Option<Lru<StatefulProtocol>>,
    pub px: bool,
    pix: Option<Lru<image::RgbImage>>,
    pub slots: HashMap<String, Slot>,
    pub tx: Option<Sender<ImgReq>>,
    pub viewer: Option<Viewer>,
}

impl Imgs {
    pub fn new() -> Self { Imgs { on: true, picker: None, lru: None, px: false, pix: None, slots: HashMap::new(), tx: None, viewer: None } }
    /// Images only exist when a picker was set up (interactive TTY, MOLT_IMAGES != off).
    pub fn proto(&mut self, id: &str) -> Option<&mut StatefulProtocol> { self.lru.as_mut()?.get(id) }
    pub fn has(&self, id: &str) -> bool { self.lru.as_ref().is_some_and(|l| l.contains(id)) || self.pix.as_ref().is_some_and(|l| l.contains(id)) }
    pub fn pixels(&mut self, id: &str) -> Option<&mut image::RgbImage> { self.pix.as_mut()?.get(id) }
    pub fn enabled(&self) -> bool { self.px || self.picker.is_some() }
    pub fn font(&self) -> (u16, u16) { self.picker.as_ref().map_or((1, 2), |p| p.font_size()) }
    pub fn state(&self, id: &str) -> ImgState {
        match self.slots.get(id) {
            Some(Slot::Ready { .. }) => ImgState::Ready,
            Some(Slot::Failed(e)) => ImgState::Failed(e.clone()),
            Some(Slot::Loading) => ImgState::Loading,
            None if self.enabled() => ImgState::Loading,
            None => ImgState::Off,
        }
    }
    /// Ask the worker for the newest image messages we do not have yet.
    pub fn want(&mut self, msgs: &[&Msg]) {
        let Some(tx) = self.tx.clone() else { return };
        if !self.on { return }
        for m in msgs.iter().rev().filter(|m| is_img(m)).take(WANT) { self.request(&tx, m) }
    }
    pub fn request(&mut self, tx: &Sender<ImgReq>, m: &Msg) {
        if self.slots.contains_key(&m.id) { return }
        let Some(meta) = m.file.clone() else { return };
        self.slots.insert(m.id.clone(), Slot::Loading);
        let _ = tx.send(ImgReq { id: m.id.clone(), meta });
    }
    /// Keep the viewer's id list in sync with the chat and fetch the shown image; false = close the viewer.
    pub fn sync_viewer(&mut self, msgs: &[&Msg]) {
        let Some(v) = self.viewer.as_mut() else { return };
        let ids: Vec<String> = msgs.iter().filter(|m| is_img(m)).map(|m| m.id.clone()).collect();
        if !v.update(ids) { self.viewer = None; return }
        let cur = v.cur().to_string();
        if let (Some(tx), Some(m)) = (self.tx.clone(), msgs.iter().find(|m| m.id == cur)) { self.request(&tx, m) }
    }
    pub fn on_loaded(&mut self, id: String, r: Decoded) {
        if self.px {
            match r {
                Ok((img, gif)) => {
                    let p = crate::pixel::prep(&img);
                    let slot = Slot::Ready { w: p.width(), h: p.height(), gif };
                    if let Some((old, _)) = self.pix.get_or_insert_with(|| Lru::new(48)).insert(id.clone(), p) { self.slots.remove(&old); }
                    self.slots.insert(id, slot);
                }
                Err(e) => { self.slots.insert(id, Slot::Failed(e)); }
            }
            return;
        }
        match (r, &self.picker) {
            (Ok((img, gif)), Some(p)) => {
                let slot = Slot::Ready { w: img.width(), h: img.height(), gif };
                let proto = p.new_resize_protocol(img);
                if let Some((old, _)) = self.lru.get_or_insert_with(|| Lru::new(24)).insert(id.clone(), proto) { self.slots.remove(&old); }
                self.slots.insert(id, slot);
            }
            (Err(e), _) => { self.slots.insert(id, Slot::Failed(e)); }
            _ => {}
        }
    }
}

pub fn is_img(m: &Msg) -> bool { m.file.as_ref().is_some_and(|f| files::is_image(&f.kind, &f.name)) }

pub fn cache_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(std::env::temp_dir, PathBuf::from).join(".cache/molt-tui/img")
}

/// Download (verified) and decode one image. Never panics on hostile bytes.
pub fn load(m: &Mcp, cache: &std::path::Path, r: &ImgReq) -> Decoded {
    let p = fetch::ensure(m, cache, &r.id, &r.meta, MAX_BYTES, Duration::from_secs(40))?;
    let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
    let kind = files::magic(&bytes).ok_or("kein unterstütztes Bildformat")?;
    let img = image::load_from_memory(&bytes).map_err(|_| "Bild nicht lesbar".to_string())?;
    let img = if img.width().max(img.height()) > MAX_PX { img.thumbnail(MAX_PX, MAX_PX) } else { img };
    Ok((img, kind == files::ImgKind::Gif))
}

pub fn spawn_worker(m: Mcp, rx: std::sync::mpsc::Receiver<ImgReq>, out: Sender<crate::app::Ev>) {
    std::thread::spawn(move || {
        let cache = cache_dir();
        for r in rx {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load(&m, &cache, &r))).unwrap_or_else(|_| Err("Bild nicht lesbar".into()));
            if out.send(crate::app::Ev::Img(r.id, res)).is_err() { break }
        }
    });
}

/// Image mode from MOLT_IMAGES: off | graphics (kitty/iTerm2/sixel via terminal query) | anything else = pixel art (default).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode { Off, Pixel, Graphics }
pub fn mode_of(v: Option<&str>) -> Mode {
    match v { Some("off" | "0" | "no") => Mode::Off, Some("graphics" | "gfx") => Mode::Graphics, _ => Mode::Pixel }
}
pub fn mode() -> Mode {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() { return Mode::Off }
    mode_of(std::env::var("MOLT_IMAGES").ok().as_deref())
}

/// Only for Mode::Graphics: query the terminal (raw mode on, alt screen entered), fall back to halfblocks.
pub fn make_picker() -> Picker {
    Picker::from_query_stdio().unwrap_or_else(|_| {
        let mut p = Picker::from_fontsize((8, 16));
        p.set_protocol_type(ratatui_image::picker::ProtocolType::Halfblocks);
        p
    })
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn modes() {
        assert_eq!(mode_of(None), Mode::Pixel);
        assert_eq!(mode_of(Some("graphics")), Mode::Graphics);
        assert_eq!(mode_of(Some("off")), Mode::Off);
        assert_eq!(mode_of(Some("x")), Mode::Pixel);
    }
    #[test]
    fn pixel_load_sets_ready() {
        let mut i = Imgs::new(); i.px = true;
        i.on_loaded("a".into(), Ok((image::DynamicImage::new_rgb8(4, 6), false)));
        assert!(i.has("a") && i.enabled() && i.font() == (1, 2));
        assert!(matches!(i.slots.get("a"), Some(Slot::Ready { w: 4, h: 6, .. })));
        assert!(i.pixels("a").is_some());
    }
}
