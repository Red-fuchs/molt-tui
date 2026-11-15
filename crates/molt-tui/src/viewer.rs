//! Full-screen image viewer state machine (no rendering, no I/O).
#![allow(dead_code, unused_variables)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VKey { Esc, Left, Right, Home, End, Other }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAct { None, Close, Moved }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewer { ids: Vec<String>, i: usize }

impl Viewer {
    /// Viewer over the image message ids (chat order) positioned on `start`; None if `ids` is empty or lacks `start`.
    /// qfill:test viewer
    pub fn new(ids: Vec<String>, start: &str) -> Option<Viewer> {
        let idx = ids.iter().position(|id| id == start)?;
        Some(Viewer { ids, i: idx })
    }
    /// Id of the image being shown.
    /// qfill:test viewer
    pub fn cur(&self) -> &str {
        self.ids.get(self.i).map(|s| s.as_str()).unwrap_or("")
    }
    /// 1-based position and total, e.g. (2, 5).
    /// qfill:test viewer
    pub fn pos(&self) -> (usize, usize) {
        (self.i + 1, self.ids.len())
    }
    /// Esc -> Close. Left/Right -> previous/next (no wrap-around); Moved if the index changed, else None. Home/End -> first/last, same rule. Other -> None.
    /// qfill:test viewer
    pub fn step(&mut self, k: VKey) -> VAct {
        match k {
            VKey::Esc => VAct::Close,
            VKey::Left => {
                if self.i == 0 {
                    VAct::None
                } else {
                    self.i = self.i.saturating_sub(1);
                    VAct::Moved
                }
            }
            VKey::Right => {
                if self.i + 1 < self.ids.len() {
                    self.i += 1;
                    VAct::Moved
                } else {
                    VAct::None
                }
            }
            VKey::Home => {
                if self.i == 0 {
                    VAct::None
                } else {
                    self.i = 0;
                    VAct::Moved
                }
            }
            VKey::End => {
                if self.i == self.ids.len() - 1 {
                    VAct::None
                } else {
                    self.i = self.ids.len() - 1;
                    VAct::Moved
                }
            }
            VKey::Other => VAct::None,
        }
    }
    /// Refresh the id list after new messages arrived. Keep showing the same id if still present (its index may change);
    /// if it vanished clamp the old index into the new list. Returns false (caller must close) when `ids` is empty, else true.
    /// qfill:test viewer
    pub fn update(&mut self, ids: Vec<String>) -> bool {
        if ids.is_empty() {
            return false;
        }
        let current_id = self.cur().to_string();
        let new_idx = ids.iter().position(|id| *id == current_id);
        self.ids = ids;
        self.i = match new_idx {
            Some(idx) => idx,
            None => self.i.min(self.ids.len() - 1),
        };
        true
    }
}

#[cfg(test)]
mod t {
    use super::*;
    fn v(ids: &[&str], at: &str) -> Viewer { Viewer::new(ids.iter().map(|s| s.to_string()).collect(), at).unwrap() }

    #[test]
    fn new_cases() {
        assert!(Viewer::new(vec![], "a").is_none());
        assert!(Viewer::new(vec!["a".into()], "b").is_none());
        let x = v(&["a", "b", "c"], "b");
        assert_eq!((x.cur(), x.pos()), ("b", (2, 3)));
    }

    #[test]
    fn navigation_clamps() {
        let mut x = v(&["a", "b", "c"], "a");
        assert_eq!(x.step(VKey::Left), VAct::None);
        assert_eq!(x.step(VKey::Right), VAct::Moved);
        assert_eq!(x.step(VKey::Right), VAct::Moved);
        assert_eq!(x.cur(), "c");
        assert_eq!(x.step(VKey::Right), VAct::None);
        assert_eq!(x.step(VKey::Left), VAct::Moved);
        assert_eq!(x.pos(), (2, 3));
        assert_eq!(x.step(VKey::Home), VAct::Moved);
        assert_eq!(x.step(VKey::Home), VAct::None);
        assert_eq!(x.step(VKey::End), VAct::Moved);
        assert_eq!(x.step(VKey::End), VAct::None);
        assert_eq!(x.cur(), "c");
    }

    #[test]
    fn esc_and_other() {
        let mut x = v(&["a", "b"], "a");
        assert_eq!(x.step(VKey::Other), VAct::None);
        assert_eq!(x.step(VKey::Esc), VAct::Close);
        assert_eq!(x.pos(), (1, 2));
    }

    #[test]
    fn single_image() {
        let mut x = v(&["a"], "a");
        for k in [VKey::Left, VKey::Right, VKey::Home, VKey::End] { assert_eq!(x.step(k), VAct::None) }
    }

    #[test]
    fn update_keeps_id() {
        let mut x = v(&["a", "b", "c"], "b");
        assert!(x.update(vec!["z".into(), "a".into(), "b".into(), "c".into()]));
        assert_eq!((x.cur(), x.pos()), ("b", (3, 4)));
    }

    #[test]
    fn update_vanished_clamps() {
        let mut x = v(&["a", "b", "c"], "c");
        assert!(x.update(vec!["a".into(), "b".into()]));
        assert_eq!((x.cur(), x.pos()), ("b", (2, 2)));
        let mut y = v(&["a", "b", "c"], "b");
        assert!(y.update(vec!["a".into(), "c".into(), "d".into()]));
        assert_eq!((y.cur(), y.pos()), ("c", (2, 3)));
        assert!(!y.update(vec![]));
    }
}
