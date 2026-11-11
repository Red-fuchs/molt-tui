use crate::text::{clean, cw};

#[derive(Default)]
pub struct Editor {
    pub t: Vec<char>,
    pub cur: usize,
    pub anc: Option<usize>,
    undo: Vec<(Vec<char>, usize)>,
    redo: Vec<(Vec<char>, usize)>,
    last: u8,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mv { L, R, WL, WR, Home, End, Up, Down, Top, Bottom }

pub type Row = (usize, usize);

impl Editor {
    pub fn text(&self) -> String { self.t.iter().collect() }
    pub fn is_empty(&self) -> bool { self.t.iter().all(|c| c.is_whitespace()) }
    pub fn clear(&mut self) { *self = Editor::default() }

    fn snap(&mut self, kind: u8) {
        if kind == 0 || kind != self.last {
            self.undo.push((self.t.clone(), self.cur));
            if self.undo.len() > 200 { self.undo.remove(0); }
            self.redo.clear();
        }
        self.last = kind;
    }
    pub fn undo(&mut self) {
        if let Some((t, c)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.t, t), self.cur));
            self.cur = c; self.anc = None; self.last = 0;
        }
    }
    pub fn redo(&mut self) {
        if let Some((t, c)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.t, t), self.cur));
            self.cur = c; self.anc = None; self.last = 0;
        }
    }

    pub fn sel(&self) -> Option<(usize, usize)> {
        self.anc.filter(|a| *a != self.cur).map(|a| (a.min(self.cur), a.max(self.cur)))
    }
    pub fn selected(&self) -> String { self.sel().map(|(a, b)| self.t[a..b].iter().collect()).unwrap_or_default() }
    fn del_sel(&mut self) -> bool {
        if let Some((a, b)) = self.sel() { self.t.drain(a..b); self.cur = a; self.anc = None; true } else { self.anc = None; false }
    }
    pub fn select_all(&mut self) { self.anc = Some(0); self.cur = self.t.len() }
    pub fn cut(&mut self) -> String {
        let s = self.selected();
        if !s.is_empty() { self.snap(0); self.del_sel(); }
        s
    }

    pub fn insert_str(&mut self, s: &str) {
        let s = clean(&s.replace("\r\n", "\n").replace('\r', "\n"));
        if s.is_empty() { return }
        let multi = s.chars().count() > 1 || self.sel().is_some();
        self.snap(if multi { 0 } else { 1 });
        self.del_sel();
        let n = s.chars().count();
        self.t.splice(self.cur..self.cur, s.chars());
        self.cur += n;
    }
    pub fn newline(&mut self) { self.snap(0); self.del_sel(); self.t.insert(self.cur, '\n'); self.cur += 1 }
    pub fn backspace(&mut self) {
        if self.sel().is_some() { self.snap(0); self.del_sel(); }
        else if self.cur > 0 { self.snap(2); self.cur -= 1; self.t.remove(self.cur); }
    }
    pub fn delete(&mut self) {
        if self.sel().is_some() { self.snap(0); self.del_sel(); }
        else if self.cur < self.t.len() { self.snap(3); self.t.remove(self.cur); }
    }
    pub fn del_word_back(&mut self) {
        if self.sel().is_some() { self.snap(0); self.del_sel(); return }
        let a = self.word_l(self.cur);
        if a < self.cur { self.snap(0); self.t.drain(a..self.cur); self.cur = a; }
    }
    pub fn kill_end(&mut self) {
        let e = self.line_end(self.cur);
        let e = if e == self.cur && e < self.t.len() { e + 1 } else { e };
        if e > self.cur { self.snap(0); self.t.drain(self.cur..e); }
    }
    pub fn kill_start(&mut self) {
        let s = self.line_start(self.cur);
        if s < self.cur { self.snap(0); self.t.drain(s..self.cur); self.cur = s; }
    }

    fn line_start(&self, i: usize) -> usize { self.t[..i].iter().rposition(|c| *c == '\n').map_or(0, |p| p + 1) }
    fn line_end(&self, i: usize) -> usize { self.t[i..].iter().position(|c| *c == '\n').map_or(self.t.len(), |p| i + p) }
    fn word_l(&self, mut i: usize) -> usize {
        while i > 0 && self.t[i - 1].is_whitespace() { i -= 1 }
        while i > 0 && !self.t[i - 1].is_whitespace() { i -= 1 }
        i
    }
    fn word_r(&self, mut i: usize) -> usize {
        let n = self.t.len();
        while i < n && self.t[i].is_whitespace() { i += 1 }
        while i < n && !self.t[i].is_whitespace() { i += 1 }
        i
    }

    /// Visual rows (start,end) after soft wrap at `w` columns; hard newlines split rows.
    pub fn rows(&self, w: usize) -> Vec<Row> {
        let w = w.max(2);
        let (t, n) = (&self.t, self.t.len());
        let mut rows = vec![];
        let mut s = 0;
        loop {
            let e = t[s..].iter().position(|c| *c == '\n').map_or(n, |p| s + p);
            let mut a = s;
            loop {
                let (mut j, mut k, mut sp) = (a, 0, None);
                while j < e {
                    let c = cw(t[j]);
                    if k + c > w { break }
                    k += c;
                    j += 1;
                    if t[j - 1] == ' ' { sp = Some(j) }
                }
                if j >= e { rows.push((a, e)); break }
                let b = match sp { Some(p) if p > a => p, _ => j.max(a + 1) };
                rows.push((a, b));
                a = b;
            }
            if e >= n { break }
            s = e + 1;
        }
        rows
    }
    fn col(&self, r: Row, i: usize) -> usize { self.t[r.0..i.min(r.1)].iter().map(|c| cw(*c)).sum() }
    pub fn pos(&self, rows: &[Row], i: usize) -> (usize, usize) {
        let r = rows.iter().rposition(|r| r.0 <= i).unwrap_or(0);
        (r, self.col(rows[r], i))
    }
    pub fn idx_at(&self, rows: &[Row], r: usize, col: usize) -> usize {
        let row = rows[r.min(rows.len() - 1)];
        let (mut i, mut k) = (row.0, 0);
        while i < row.1 && k + cw(self.t[i]) <= col { k += cw(self.t[i]); i += 1 }
        i
    }

    pub fn mv(&mut self, m: Mv, shift: bool, w: usize) {
        if shift { self.anc.get_or_insert(self.cur); }
        else if let (Some((a, b)), true) = (self.sel(), matches!(m, Mv::L | Mv::R | Mv::WL | Mv::WR)) {
            self.cur = if matches!(m, Mv::L | Mv::WL) { a } else { b };
            self.anc = None;
            return;
        } else { self.anc = None }
        let n = self.t.len();
        self.cur = match m {
            Mv::L => self.cur.saturating_sub(1),
            Mv::R => (self.cur + 1).min(n),
            Mv::WL => self.word_l(self.cur),
            Mv::WR => self.word_r(self.cur),
            Mv::Home => self.line_start(self.cur),
            Mv::End => self.line_end(self.cur),
            Mv::Top => 0,
            Mv::Bottom => n,
            Mv::Up | Mv::Down => {
                let rows = self.rows(w);
                let (r, c) = self.pos(&rows, self.cur);
                if m == Mv::Up { if r == 0 { 0 } else { self.idx_at(&rows, r - 1, c) } }
                else if r + 1 >= rows.len() { n } else { self.idx_at(&rows, r + 1, c) }
            }
        };
    }
    pub fn click(&mut self, w: usize, r: usize, c: usize, shift: bool) {
        if shift { if self.anc.is_none() { self.anc = Some(self.cur) } } else { self.anc = None }
        let rows = self.rows(w);
        self.cur = self.idx_at(&rows, r, c);
    }
}

#[cfg(test)]
mod t {
    use super::*;
    fn ed(s: &str) -> Editor { let mut e = Editor::default(); e.insert_str(s); e }
    #[test]
    fn insert_edit() {
        let mut e = ed("hello world");
        e.mv(Mv::Home, false, 20);
        assert_eq!(e.cur, 0);
        e.mv(Mv::WR, false, 20);
        assert_eq!(e.cur, 5);
        e.mv(Mv::WR, false, 20);
        assert_eq!(e.cur, 11);
        e.del_word_back();
        assert_eq!(e.text(), "hello ");
        e.backspace();
        assert_eq!(e.text(), "hello");
    }
    #[test]
    fn kill_and_undo() {
        let mut e = ed("ab\ncd");
        e.cur = 4;
        e.kill_start();
        assert_eq!(e.text(), "ab\nd");
        e.undo();
        assert_eq!(e.text(), "ab\ncd");
        e.redo();
        assert_eq!(e.text(), "ab\nd");
        e.cur = 1;
        e.kill_end();
        assert_eq!(e.text(), "a\nd");
        e.kill_end();
        assert_eq!(e.text(), "ad");
    }
    #[test]
    fn paste_sanitized_and_newlines() {
        let e = ed("a\r\nb\x1b[31mc\x07");
        assert_eq!(e.text(), "a\nbc");
    }
    #[test]
    fn wrap_rows_and_cursor() {
        let mut e = ed("aaa bbb ccc");
        let r = e.rows(5);
        assert_eq!(r, vec![(0, 4), (4, 8), (8, 11)]);
        assert_eq!(e.pos(&r, 11), (2, 3));
        e.mv(Mv::Up, false, 5);
        assert_eq!(e.pos(&e.rows(5), e.cur).0, 1);
        e.mv(Mv::Down, false, 5);
        e.mv(Mv::Down, false, 5);
        assert_eq!(e.cur, 11);
    }
    #[test]
    fn selection_and_replace() {
        let mut e = ed("hello");
        e.mv(Mv::Home, false, 9);
        e.mv(Mv::R, true, 9);
        e.mv(Mv::R, true, 9);
        assert_eq!(e.selected(), "he");
        e.insert_str("X");
        assert_eq!(e.text(), "Xllo");
        e.select_all();
        assert_eq!(e.cut(), "Xllo");
        assert!(e.text().is_empty());
    }
    #[test]
    fn click_maps() {
        let mut e = ed("abc\ndef");
        e.click(10, 1, 2, false);
        assert_eq!(e.cur, 6);
        e.click(10, 0, 99, false);
        assert_eq!(e.cur, 3);
    }
    #[test]
    fn trailing_newline_row() {
        let e = ed("a\n");
        assert_eq!(e.rows(5), vec![(0, 1), (2, 2)]);
        assert_eq!(e.pos(&e.rows(5), 2), (1, 0));
    }
}
