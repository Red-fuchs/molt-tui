//! Pixel-art renderer: 2 image pixels per cell via '▀' (fg = top, bg = bottom), nearest-neighbour resize,
//! colours snapped to the xterm 6x6x6 cube (indexed, works in every terminal incl. tmux, no queries).
use image::{imageops::FilterType, DynamicImage, RgbImage};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};

/// Keep a small master copy (smooth downscale once) so per-frame nearest resizes stay cheap.
pub fn prep(img: &DynamicImage) -> RgbImage {
    let (w, h) = (img.width(), img.height());
    let img = if w.max(h) > 192 { img.resize(192, 192, FilterType::Triangle) } else { img.clone() };
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

/// (top, bottom) cube indexes per cell, row-major c*r, from a nearest resize to c x 2r.
pub fn grid(p: &RgbImage, c: u16, r: u16) -> Vec<(u8, u8)> {
    if c == 0 || r == 0 || p.width() == 0 || p.height() == 0 { return vec![] }
    let s = image::imageops::resize(p, c as u32, r as u32 * 2, FilterType::Nearest);
    let mut v = Vec::with_capacity(c as usize * r as usize);
    for y in 0..r as u32 { for x in 0..c as u32 { v.push((idx(s.get_pixel(x, y * 2).0), idx(s.get_pixel(x, y * 2 + 1).0))) } }
    v
}

pub fn render(p: &RgbImage, area: Rect, buf: &mut Buffer) {
    let g = grid(p, area.width, area.height);
    for (i, (t, b)) in g.into_iter().enumerate() {
        let (x, y) = (area.x + (i % area.width as usize) as u16, area.y + (i / area.width as usize) as u16);
        if let Some(cell) = buf.cell_mut((x, y)) { cell.set_char('▀').set_fg(Color::Indexed(t)).set_bg(Color::Indexed(b)); }
    }
}

/// Colour-free preview for --dump: shade by luminance (' ' ░ ▒ ▓ █), 2 pixels per cell as ▀ ▄ █ ' '-style halves.
#[cfg(test)]
pub fn text(p: &RgbImage, c: u16, r: u16) -> Vec<String> {
    grid(p, c, r).chunks(c.max(1) as usize).map(|row| row.iter().map(|&(t, b)| half(t, b)).collect()).collect()
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
        assert_eq!(g[0], (231, 231)); // rows 0..1 of the 4x4 -> cell row 0 uses y0,y1 (both white)
        assert_eq!(g[4], (16, 16));
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
        assert_eq!((c.symbol(), c.fg, c.bg), ("▀", Color::Indexed(196), Color::Indexed(16)));
    }
    #[test]
    fn alpha_over_black() {
        let mut i = image::RgbaImage::new(1, 1); i.put_pixel(0, 0, image::Rgba([255, 255, 255, 0]));
        assert_eq!(prep(&DynamicImage::ImageRgba8(i)).get_pixel(0, 0).0, [0, 0, 0]);
    }
}
