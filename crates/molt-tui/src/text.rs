use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
pub use molt_client::text::{clean, one};

pub fn w(s: &str) -> usize { s.width() }
pub fn cw(c: char) -> usize { c.width().unwrap_or(0) }

pub fn trunc(s: &str, max: usize) -> String {
    if w(s) <= max { return s.to_string() }
    if max == 0 { return String::new() }
    let mut o = String::new();
    let mut n = 0;
    for c in s.chars() {
        let k = cw(c);
        if n + k + 1 > max { break }
        o.push(c);
        n += k;
    }
    o.push('…');
    o
}

pub fn wrap(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = vec![];
    for line in s.split('\n') {
        if line.is_empty() { out.push(String::new()); continue }
        let mut cur = String::new();
        let mut n = 0;
        for tok in line.split_inclusive(' ') {
            let t = tok.trim_end_matches(' ');
            let tw = w(t);
            if tw > width {
                if n > 0 { out.push(cur.trim_end().to_string()); cur.clear(); n = 0 }
                for ch in tok.chars() {
                    let k = cw(ch);
                    if n + k > width && !cur.is_empty() { out.push(std::mem::take(&mut cur)); n = 0 }
                    cur.push(ch);
                    n += k;
                }
                continue;
            }
            if n + tw > width && n > 0 {
                out.push(cur.trim_end().to_string());
                cur.clear();
                n = 0;
            }
            cur.push_str(tok);
            n += w(tok);
        }
        out.push(cur.trim_end().to_string());
    }
    out
}

pub fn tz_offset() -> i64 {
    let o = std::process::Command::new("date").arg("+%z").output().ok();
    let s = o.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    if s.len() != 5 { return 0 }
    let sign = if s.starts_with('-') { -1 } else { 1 };
    let h: i64 = s[1..3].parse().unwrap_or(0);
    let m: i64 = s[3..5].parse().unwrap_or(0);
    sign * (h * 3600 + m * 60)
}

pub fn fmt_ts(ts: i64, off: i64) -> String {
    let t = ts + off;
    let (d, s) = (t.div_euclid(86400), t.rem_euclid(86400));
    let z = d + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let mon = if mp < 10 { mp + 3 } else { mp - 9 };
    let _ = era;
    format!("{:02}.{:02} {:02}:{:02}", day, mon, s / 3600, s % 3600 / 60)
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn wrap_basic() {
        assert_eq!(wrap("hello world foo", 11), vec!["hello world", "foo"]);
        assert_eq!(wrap("a\n\nb", 5), vec!["a", "", "b"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("  - item eins zwei", 10), vec!["  - item", "eins zwei"]);
        assert!(wrap("日本語日本語", 4).iter().all(|l| w(l) <= 4));
    }
    #[test]
    fn trunc_w() { assert_eq!(trunc("abcdef", 4), "abc…"); assert_eq!(trunc("ab", 4), "ab") }
    #[test]
    fn ts() { assert_eq!(fmt_ts(1794410193, 0), "11.11 15:16"); assert_eq!(fmt_ts(0, 3600), "01.01 01:00") }
}
