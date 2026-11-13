fn bad(c: char) -> bool {
    c.is_control()
        || matches!(c as u32,
            0xAD | 0x61C | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF
            | 0xFFF9..=0xFFFB | 0x1D173..=0x1D17A | 0xE0000..=0xE007F)
}

/// Strip ANSI sequences, control, bidi and zero-width chars. Keeps '\n'; tab -> space; CRLF -> LF.
pub fn clean(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut o = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        match c {
            '\x1b' => match cs.get(i) {
                Some('[') => {
                    i += 1;
                    while i < cs.len() && (0x20..=0x3f).contains(&(cs[i] as u32)) { i += 1 }
                    if i < cs.len() && (0x40..=0x7e).contains(&(cs[i] as u32)) { i += 1 }
                }
                Some(']' | 'P' | '_' | '^' | 'X') => {
                    i += 1;
                    while i < cs.len() {
                        if cs[i] == '\x07' { i += 1; break }
                        if cs[i] == '\x1b' && cs.get(i + 1) == Some(&'\\') { i += 2; break }
                        i += 1;
                    }
                }
                Some(_) => i += 1,
                None => {}
            },
            '\n' => o.push('\n'),
            '\t' => o.push(' '),
            c if bad(c) => {}
            c => o.push(c),
        }
    }
    o
}

pub fn one(s: &str) -> String {
    let c = clean(s);
    c.split('\n').map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ⏎ ")
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn strips_ansi_and_ctrl() {
        assert_eq!(clean("a\x1b[31mred\x1b[0m b"), "ared b");
        assert_eq!(clean("x\x1b]0;title\x07y"), "xy");
        assert_eq!(clean("x\x1b]8;;http://e\x1b\\link\x1b]8;;\x1b\\"), "xlink");
        assert_eq!(clean("a\x07b\x00c\x7fd\u{85}e"), "abcde");
        assert_eq!(clean("a\r\nb\rc\td"), "a\nbc d");
    }
    #[test]
    fn strips_bidi_zw() {
        assert_eq!(clean("a\u{202e}b\u{200b}c\u{2066}d\u{feff}e\u{e0041}f"), "abcdef");
        assert_eq!(clean("ä😀"), "ä😀");
    }
    #[test]
    fn truncated_escape() {
        assert_eq!(clean("a\x1b"), "a");
        assert_eq!(clean("a\x1b[31"), "a");
    }
    #[test]
    fn one_line() { assert_eq!(one("a\n\n b \nc"), "a ⏎ b ⏎ c") }
}
