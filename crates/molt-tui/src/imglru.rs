//! Tiny least-recently-used map for decoded/encoded image state (keeps memory bounded).
#![allow(dead_code, unused_variables)]
use std::collections::HashMap;

pub struct Lru<V> { cap: usize, tick: u64, map: HashMap<String, (u64, V)> }

impl<V> Lru<V> {
    /// `cap` is at least 1 (0 is treated as 1).
    /// qfill:test lru
    pub fn new(cap: usize) -> Self { Self { cap: cap.max(1), tick: 0, map: HashMap::new() } }
    /// Lookup that marks the entry most recently used.
    /// qfill:test lru
    pub fn get(&mut self, k: &str) -> Option<&mut V> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| { e.0 = t; &mut e.1 })
    }
    /// Lookup without touching recency.
    /// qfill:test lru
    pub fn contains(&self, k: &str) -> bool { self.map.contains_key(k) }
    /// Insert as most recently used. Replacing an existing key never evicts and returns None.
    /// A new key at capacity evicts the least recently used entry and returns it as Some((key, value)). Recency: every get/insert bumps a counter.
    /// qfill:test lru
    pub fn insert(&mut self, k: String, v: V) -> Option<(String, V)> {
        self.tick += 1;
        let mut ev = None;
        if !self.map.contains_key(&k) && self.map.len() >= self.cap {
            let old = self.map.iter().min_by_key(|(_, e)| e.0).map(|(k, _)| k.clone());
            if let Some(o) = old { ev = self.map.remove(&o).map(|e| (o, e.1)); }
        }
        self.map.insert(k, (self.tick, v));
        ev
    }
    /// qfill:test lru
    pub fn remove(&mut self, k: &str) -> Option<V> { self.map.remove(k).map(|e| e.1) }
    /// qfill:test lru
    pub fn len(&self) -> usize { self.map.len() }
}

#[cfg(test)]
mod lru {
    use super::*;

    #[test]
    fn evicts_least_recent() {
        let mut l = Lru::new(2);
        assert!(l.insert("a".into(), 1).is_none());
        assert!(l.insert("b".into(), 2).is_none());
        assert_eq!(l.len(), 2);
        assert_eq!(l.insert("c".into(), 3), Some(("a".into(), 1)));
        assert!(!l.contains("a") && l.contains("b") && l.contains("c"));
    }

    #[test]
    fn get_bumps_recency() {
        let mut l = Lru::new(2);
        l.insert("a".into(), 1);
        l.insert("b".into(), 2);
        assert_eq!(l.get("a").copied(), Some(1));
        assert_eq!(l.insert("c".into(), 3), Some(("b".into(), 2)));
        assert!(l.get("zz").is_none());
    }

    #[test]
    fn contains_does_not_bump() {
        let mut l = Lru::new(2);
        l.insert("a".into(), 1);
        l.insert("b".into(), 2);
        assert!(l.contains("a"));
        assert_eq!(l.insert("c".into(), 3), Some(("a".into(), 1)));
    }

    #[test]
    fn replace_and_remove() {
        let mut l = Lru::new(2);
        l.insert("a".into(), 1);
        l.insert("b".into(), 2);
        assert!(l.insert("a".into(), 10).is_none());       // replace: no eviction
        assert_eq!(l.len(), 2);
        assert_eq!(l.get("a").copied(), Some(10));
        assert_eq!(l.insert("c".into(), 3), Some(("b".into(), 2))); // replace bumped "a"
        assert_eq!(l.remove("a"), Some(10));
        assert_eq!(l.remove("a"), None);
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn cap_zero_is_one() {
        let mut l = Lru::new(0);
        assert!(l.insert("a".into(), 1).is_none());
        assert_eq!(l.insert("b".into(), 2), Some(("a".into(), 1)));
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn get_mut_changes_value() {
        let mut l = Lru::new(3);
        l.insert("a".into(), 1);
        *l.get("a").unwrap() += 5;
        assert_eq!(l.get("a").copied(), Some(6));
    }
}
