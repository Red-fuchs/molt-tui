//! Pure layout math for inline images: size in cells, scroll visibility, captions.
#![allow(dead_code, unused_variables)]

/// Cells an image should occupy: (cols, rows). Aspect ratio is preserved, the image is never scaled up.
/// Inputs: `img_w`/`img_h` image pixels, `fw`/`fh` pixels per terminal cell, `max_cols`/`max_rows` budget.
/// If any of the six inputs is 0 -> (0, 0).
/// Natural size in cells (floats): cx = img_w / fw, cy = img_h / fh. Scale s = min(1.0, max_cols / cx, max_rows / cy).
/// cols = ceil(cx * s), rows = ceil(cy * s), each computed as `((x - 1e-9).ceil() as u16)` then clamped to 1..=max (cols<=max_cols, rows<=max_rows).
/// qfill:test fit_cells
pub fn fit_cells(img_w: u32, img_h: u32, fw: u16, fh: u16, max_cols: u16, max_rows: u16) -> (u16, u16) {
    if img_w == 0 || img_h == 0 || fw == 0 || fh == 0 || max_cols == 0 || max_rows == 0 { return (0, 0); }
    let cx = img_w as f64 / fw as f64;
    let cy = img_h as f64 / fh as f64;
    let s = (max_cols as f64 / cx).min(max_rows as f64 / cy).min(1.0);
    let c = (((cx * s) - 1e-9).ceil() as u16).clamp(1, max_cols);
    let r = (((cy * s) - 1e-9).ceil() as u16).clamp(1, max_rows);
    (c, r)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vis { Full, Partial, Hidden }

/// Visibility of the transcript line range [start, end) inside the viewport [top, top + height).
/// start >= end (empty block) or height == 0 or no overlap -> Hidden; completely inside -> Full; otherwise Partial.
/// qfill:test vis
pub fn vis(start: usize, end: usize, top: usize, height: usize) -> Vis {
    if start >= end || height == 0 { return Vis::Hidden; }
    let bot = top.saturating_add(height);
    if end <= top || start >= bot { Vis::Hidden } else if start >= top && end <= bot { Vis::Full } else { Vis::Partial }
}

/// Byte count for humans. < 1024 -> "{n} B". Otherwise the largest unit among KB (1024), MB (1024^2), GB (1024^3) with value >= 1,
/// printed as "{whole}.{tenth} {unit}" where v10 = bytes * 10 / unit (integer division, truncating, computed in u128), whole = v10 / 10, tenth = v10 % 10.
/// qfill:test human_size
pub fn human_size(n: u64) -> String {
    if n < 1024 { return format!("{n} B"); }
    let (u, name) = if n >= 1 << 30 { (1u128 << 30, "GB") } else if n >= 1 << 20 { (1u128 << 20, "MB") } else { (1024, "KB") };
    let v10 = n as u128 * 10 / u;
    format!("{}.{} {}", v10 / 10, v10 % 10, name)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImgState { Ready, Loading, Off, Failed(String) }

/// One-line caption of an image message: "🖼 {name} ({size})" with size = human_size, then " GIF" if `gif`,
/// then a suffix by state: Loading -> " – wird geprüft", Failed(r) -> " – {r}", Ready/Off -> nothing. The dash is U+2013 with spaces around.
/// The name is used as given (callers sanitise it).
/// qfill:test caption
pub fn caption(name: &str, size: u64, gif: bool, st: &ImgState) -> String {
    let mut o = format!("🖼 {name} ({})", human_size(size));
    if gif { o.push_str(" GIF"); }
    match st {
        ImgState::Loading => o.push_str(" – wird geprüft"),
        ImgState::Failed(r) => { o.push_str(" – "); o.push_str(r); }
        _ => {}
    }
    o
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn fit_cells_cases() {
        assert_eq!(fit_cells(100, 100, 10, 20, 80, 10), (10, 5));      // fits: unchanged
        assert_eq!(fit_cells(800, 400, 10, 20, 100, 10), (40, 10));    // 80x20 cells, rows limit: s = 0.5
        assert_eq!(fit_cells(800, 400, 10, 20, 20, 10), (20, 5));      // cols limit: s = 0.25 -> 20 x 5
        assert_eq!(fit_cells(1600, 400, 10, 20, 40, 10), (40, 5));     // 160x20: s = 0.25 -> 40 x 5
    }

    #[test]
    fn fit_cells_more() {
        assert_eq!(fit_cells(400, 1600, 10, 20, 100, 10), (5, 10));
        assert_eq!(fit_cells(1, 1, 10, 20, 80, 10), (1, 1));           // tiny: clamp up to 1
        assert_eq!(fit_cells(105, 100, 10, 20, 80, 10), (11, 5));      // 10.5 x 5 -> ceil cols
        assert_eq!(fit_cells(0, 100, 10, 20, 80, 10), (0, 0));
        assert_eq!(fit_cells(100, 0, 10, 20, 80, 10), (0, 0));
        assert_eq!(fit_cells(100, 100, 0, 20, 80, 10), (0, 0));
        assert_eq!(fit_cells(100, 100, 10, 0, 80, 10), (0, 0));
        assert_eq!(fit_cells(100, 100, 10, 20, 0, 10), (0, 0));
        assert_eq!(fit_cells(100, 100, 10, 20, 80, 0), (0, 0));
        assert_eq!(fit_cells(4000, 20, 10, 20, 50, 10), (50, 1));      // 400x1 cells: s = 0.125 -> 50 x 0.125 -> rows clamp 1
    }

    #[test]
    fn vis_cases() {
        assert_eq!(vis(5, 10, 0, 20), Vis::Full);
        assert_eq!(vis(0, 20, 0, 20), Vis::Full);        // exactly the viewport
        assert_eq!(vis(5, 21, 0, 20), Vis::Partial);     // sticks out below
        assert_eq!(vis(5, 10, 6, 20), Vis::Partial);     // starts above
        assert_eq!(vis(5, 10, 9, 20), Vis::Partial);     // one line visible
        assert_eq!(vis(5, 10, 10, 20), Vis::Hidden);     // above (end == top)
        assert_eq!(vis(20, 25, 0, 20), Vis::Hidden);     // below (start == top + height)
        assert_eq!(vis(19, 25, 0, 20), Vis::Partial);
        assert_eq!(vis(7, 7, 0, 20), Vis::Hidden);       // empty
        assert_eq!(vis(9, 5, 0, 20), Vis::Hidden);       // inverted
        assert_eq!(vis(5, 10, 0, 0), Vis::Hidden);       // no viewport
        assert_eq!(vis(1, 3, 1, 2), Vis::Full);
    }

    #[test]
    fn human_size_cases() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(1500), "1.4 KB");          // truncating
        assert_eq!(human_size(1024 * 1024 - 1), "1023.9 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
        assert_eq!(human_size(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
        assert_eq!(human_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(human_size(u64::MAX), "17179869183.9 GB");
    }

    #[test]
    fn caption_cases() {
        assert_eq!(caption("a.png", 2048, false, &ImgState::Ready), "🖼 a.png (2.0 KB)");
        assert_eq!(caption("a.png", 2048, false, &ImgState::Off), "🖼 a.png (2.0 KB)");
        assert_eq!(caption("a.gif", 10, true, &ImgState::Ready), "🖼 a.gif (10 B) GIF");
        assert_eq!(caption("a.png", 10, false, &ImgState::Loading), "🖼 a.png (10 B) – wird geprüft");
        assert_eq!(caption("a.gif", 10, true, &ImgState::Loading), "🖼 a.gif (10 B) GIF – wird geprüft");
        assert_eq!(caption("a.png", 10, false, &ImgState::Failed("Zeitüberschreitung".into())), "🖼 a.png (10 B) – Zeitüberschreitung");
    }
}
