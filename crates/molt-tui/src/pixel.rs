//! Pixel renderer: 2 image pixels per cell via '▀' (fg = top, bg = bottom), smooth (Catmull-Rom) resize.
//! Colours: 24-bit when COLORTERM says truecolor/24bit, else snapped to the xterm 6x6x6 cube. No terminal queries, works in tmux.
use image::{imageops::FilterType, DynamicImage, RgbImage};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};

/// Master copy edge in pixels (a cell shows 1 x 2 of them): 480 px covers ~120 cols x 120 rows sharply.
pub const MASTER: u32 = 480;

/// 24-bit colour available? (COLORTERM=truecolor|24bit; override with MOLT_IMAGES_COLORS=256|true)
pub fn truecolor() -> bool {
    match std::env::var("MOLT_IMAGES_COLORS").as_deref() { Ok("256") => false, Ok("true") => true, _ => matches!(std::env::var("COLORTERM").as_deref(), Ok("truecolor" | "24bit")) }
}

/// Keep a small master copy (smooth downscale once) so per-frame nearest resizes stay cheap.
pub fn prep(img: &DynamicImage) -> RgbImage {
    let (w, h) = (img.width(), img.height());
    let img = if w.max(h) > MASTER { img.resize(MASTER, MASTER, FilterType::CatmullRom) } else { img.clone() };
    let mut rgb = RgbImage::new(img.width(), img.height());
    for (x, y, p) in img.to_rgba8().enumerate_pixels() {
        let a = p[3] as u32;
        rgb.put_pixel(x, y, image::Rgb([0, 1, 2].map(|i| (p[i] as u32 * a / 255) as u8)));
    }
    rgb
}

/// xterm-256 cube index for an rgb triple (6 levels per channel).
pub fn idx(p: [u8; 3]) -> u8 {
    let l = |v: u8| ((v as u32 * 5 + 127) / 255) as u8;
    16 + 36 * l(p[0]) + 6 * l(p[1]) + l(p[2])
}

/// (top, bottom) rgb per cell, row-major c*r, from a smooth resize to c x 2r.
pub fn grid(p: &RgbImage, c: u16, r: u16) -> Vec<([u8; 3], [u8; 3])> {
    if c == 0 || r == 0 || p.width() == 0 || p.height() == 0 { return vec![] }
    let s = image::imageops::resize(p, c as u32, r as u32 * 2, FilterType::CatmullRom);
    let mut v = Vec::with_capacity(c as usize * r as usize);
    for y in 0..r as u32 { for x in 0..c as u32 { v.push((s.get_pixel(x, y * 2).0, s.get_pixel(x, y * 2 + 1).0)) } }
    v
}

pub fn render(p: &RgbImage, area: Rect, buf: &mut Buffer) {
    let g = grid(p, area.width, area.height);
    let tc = truecolor();
    let col = |v: [u8; 3]| if tc { Color::Rgb(v[0], v[1], v[2]) } else { Color::Indexed(idx(v)) };
    for (i, (t, b)) in g.into_iter().enumerate() {
        let (x, y) = (area.x + (i % area.width as usize) as u16, area.y + (i / area.width as usize) as u16);
        if let Some(cell) = buf.cell_mut((x, y)) { cell.set_char('▀').set_fg(col(t)).set_bg(col(b)); }
    }
}

/// Colour-free preview for --dump: shade by luminance (' ' ░ ▒ ▓ █), 2 pixels per cell as ▀ ▄ █ ' '-style halves.
#[cfg(test)]
pub fn text(p: &RgbImage, c: u16, r: u16) -> Vec<String> {
    grid(p, c, r).chunks(c.max(1) as usize).map(|row| row.iter().map(|&(t, b)| half_rgb(t, b)).collect()).collect()
}

/// Luminance-thresholded glyph for a (top, bottom) rgb pair.
pub fn half_rgb(t: [u8; 3], b: [u8; 3]) -> char {
    let lum = |p: [u8; 3]| (p[0] as u32 * 30 + p[1] as u32 * 59 + p[2] as u32 * 11) / 100 > 50;
    match (lum(t), lum(b)) { (true, true) => '█', (true, false) => '▀', (false, true) => '▄', _ => ' ' }
}

/// Luminance-thresholded glyph for a (top, bottom) cube pair.
pub fn half(t: u8, b: u8) -> char {
    let lum = |i: u8| { let i = (i as u32).saturating_sub(16); (i / 36 * 30 + i / 6 % 6 * 59 + i % 6 * 11) / 5 > 50 };
    match (lum(t), lum(b)) { (true, true) => '█', (true, false) => '▀', (false, true) => '▄', _ => ' ' }
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn cube() { assert_eq!(idx([0, 0, 0]), 16); assert_eq!(idx([255, 255, 255]), 231); assert_eq!(idx([255, 0, 0]), 196); }
    #[test]
    fn halves_and_text() {
        let mut im = RgbImage::new(4, 4);
        for y in 0..2 { for x in 0..4 { im.put_pixel(x, y, image::Rgb([255, 255, 255])) } }
        let g = grid(&im, 4, 2);
        assert_eq!(g.len(), 8);
        assert_eq!(g[0], ([255; 3], [255; 3])); // rows 0..1 of the 4x4 -> cell row 0 uses y0,y1 (both white)
        assert_eq!(g[4], ([0; 3], [0; 3]));
        assert_eq!(text(&im, 4, 2), vec!["████".to_string(), "    ".to_string()]);
        assert!(grid(&im, 0, 2).is_empty());
    }
    #[test]
    fn render_sets_cells() {
        let mut im = RgbImage::new(1, 2);
        im.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        let mut b = Buffer::empty(Rect::new(0, 0, 3, 3));
        render(&im, Rect::new(1, 1, 1, 1), &mut b);
        let c = &b[(1, 1)];
        let want = if truecolor() { (Color::Rgb(255, 0, 0), Color::Rgb(0, 0, 0)) } else { (Color::Indexed(196), Color::Indexed(16)) };
        assert_eq!((c.symbol(), c.fg, c.bg), ("▀", want.0, want.1));
    }
    #[test]
    fn alpha_over_black() {
        let mut i = image::RgbaImage::new(1, 1); i.put_pixel(0, 0, image::Rgba([255, 255, 255, 0]));
        assert_eq!(prep(&DynamicImage::ImageRgba8(i)).get_pixel(0, 0).0, [0, 0, 0]);
    }
}
