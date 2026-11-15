//! Pure helpers for shared files: verification of cached bytes, cache naming, display-name sanitising, image sniffing.
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImgKind { Png, Jpeg, Gif, Webp }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check { Missing, Pending, Mismatch, Ok }

/// Lowercase hex sha256 of `b`.
pub fn sha256_hex(b: &[u8]) -> String { Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect() }

/// Verify the cached file at `p` against the share's declared `size` and `checksum` (hex sha256, any case; anything that is not
/// exactly 64 hex chars counts as "no checksum declared" and is not compared).
/// - `symlink_metadata` fails (no such file)            -> Missing
/// - not a regular file (symlink, dir) or len != size    -> Pending (the daemon may still be writing)
/// - read fails or bytes read != size                    -> Pending
/// - checksum declared and sha256 differs                -> Mismatch
/// - otherwise                                           -> Ok
///
/// Never follow symlinks.
/// qfill:test verified
pub fn verified(p: &Path, size: u64, checksum: &str) -> Check {
    let meta = match std::fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(_) => return Check::Missing,
    };
    if !meta.is_file() {
        return Check::Pending;
    }
    if meta.len() != size {
        return Check::Pending;
    }
    let bytes = match std::fs::read(p) {
        Ok(b) => b,
        Err(_) => return Check::Pending,
    };
    if bytes.len() as u64 != size {
        return Check::Pending;
    }
    if checksum.len() == 64 && checksum.chars().all(|c| c.is_ascii_hexdigit()) {
        let actual = sha256_hex(&bytes);
        if !actual.eq_ignore_ascii_case(checksum) {
            return Check::Mismatch;
        }
    }
    Check::Ok
}

/// The cache file name for a share id: the id itself if it is exactly 32 chars of [0-9a-f], else None (rejects path tricks, uppercase, other lengths).
/// qfill:test cache_name
pub fn cache_name(id: &str) -> Option<&str> {
    if id.len() == 32 && id.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)) {
        Some(id)
    } else {
        None
    }
}

/// Make a share's display name safe for the terminal and as a file name:
/// - keep only the part after the last '/' or '\\'
/// - drop control characters (char::is_control, including ESC, \n, \t, DEL and C1 controls) and the bidi overrides U+202A..=U+202E, U+2066..=U+2069
/// - trim whitespace, then strip leading '.' characters (and trim again)
/// - truncate to at most 80 chars (chars, not bytes)
/// - empty result -> "datei"
///
/// qfill:test safe_name
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    let trimmed = cleaned.trim();
    let stripped = trimmed.trim_start_matches('.').trim();
    let truncated: String = stripped.chars().take(80).collect();
    if truncated.is_empty() {
        "datei".to_string()
    } else {
        truncated
    }
}

/// Sniff the image format from the first bytes. PNG: 89 50 4E 47; JPEG: FF D8 FF; GIF: "GIF87a" or "GIF89a";
/// WebP: len > 12, bytes 0..4 == "RIFF" and bytes 8..12 == "WEBP". Anything else None.
/// qfill:test magic
pub fn magic(b: &[u8]) -> Option<ImgKind> {
    if b.len() >= 4 && b[0] == 0x89 && b[1] == b'P' && b[2] == b'N' && b[3] == b'G' {
        Some(ImgKind::Png)
    } else if b.len() >= 3 && b[0] == 0xFF && b[1] == 0xD8 && b[2] == 0xFF {
        Some(ImgKind::Jpeg)
    } else if b.len() >= 6 && (&b[0..6] == b"GIF87a" || &b[0..6] == b"GIF89a") {
        Some(ImgKind::Gif)
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(ImgKind::Webp)
    } else {
        None
    }
}

/// Does a message's file meta look like an image? `kind` equals "image" ignoring ASCII case, OR `name` ends (ignoring ASCII case)
/// with one of .png .jpg .jpeg .gif .webp. A name with no dot never matches by extension.
/// qfill:test is_image
pub fn is_image(kind: &str, name: &str) -> bool {
    if kind.eq_ignore_ascii_case("image") {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".gif") || lower.ends_with(".webp")
}

#[cfg(test)]
mod t {
    use super::*;
    use std::io::Write;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("molt-files-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn sha_known() { assert_eq!(sha256_hex(b"abc"), ABC); assert_eq!(sha256_hex(b"").len(), 64) }

    #[test]
    fn verified_cases() {
        let d = tmp("v");
        let p = d.join("a");
        assert_eq!(verified(&p, 3, ABC), Check::Missing);
        std::fs::File::create(&p).unwrap().write_all(b"abc").unwrap();
        assert_eq!(verified(&p, 3, ABC), Check::Ok);
        assert_eq!(verified(&p, 3, &ABC.to_uppercase()), Check::Ok);
        assert_eq!(verified(&p, 4, ABC), Check::Pending);
        assert_eq!(verified(&p, 2, ABC), Check::Pending);
        assert_eq!(verified(&p, 3, &"0".repeat(64)), Check::Mismatch);
        assert_eq!(verified(&p, 3, ""), Check::Ok);
        assert_eq!(verified(&p, 3, "zz"), Check::Ok);
        assert_eq!(verified(&p, 3, &"g".repeat(64)), Check::Ok);
        assert_eq!(verified(&p, 3, &ABC[..63]), Check::Ok);
        assert_eq!(verified(&d, 3, ABC), Check::Pending);
        let l = d.join("link");
        std::os::unix::fs::symlink(&p, &l).unwrap();
        assert_eq!(verified(&l, 3, ABC), Check::Pending);
        let dangling = d.join("dang");
        std::os::unix::fs::symlink(d.join("nope"), &dangling).unwrap();
        assert_eq!(verified(&dangling, 3, ABC), Check::Pending);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cache_name_cases() {
        let ok = "0123456789abcdef0123456789abcdef";
        assert_eq!(cache_name(ok), Some(ok));
        assert_eq!(cache_name(&ok[..31]), None);
        assert_eq!(cache_name(&format!("{ok}0")), None);
        assert_eq!(cache_name(&ok.to_uppercase()), None);
        assert_eq!(cache_name("../../etc/passwd0123456789abcdef"), None);
        assert_eq!(cache_name("0123456789abcdef0123456789abcdeg"), None);
        assert_eq!(cache_name(""), None);
    }

    #[test]
    fn safe_name_cases() {
        assert_eq!(safe_name("foto.png"), "foto.png");
        assert_eq!(safe_name("a/b/c.png"), "c.png");
        assert_eq!(safe_name("..\\..\\x.png"), "x.png");
        assert_eq!(safe_name("../../.bashrc"), "bashrc");
        assert_eq!(safe_name(".hidden"), "hidden");
        assert_eq!(safe_name("..."), "datei");
        assert_eq!(safe_name(""), "datei");
        assert_eq!(safe_name("   "), "datei");
        assert_eq!(safe_name("a\x1b[31mb\nc\td\x7f.png"), "a[31mbcd.png");
        assert_eq!(safe_name("x\u{202e}gnp.exe"), "xgnp.exe");
        assert_eq!(safe_name("x\u{2066}y\u{2069}"), "xy");
        assert_eq!(safe_name("a\u{85}b"), "ab");
        assert_eq!(safe_name("  . x .png "), "x .png");
        assert_eq!(safe_name("dir/"), "datei");
        assert_eq!(safe_name(&"ä".repeat(100)).chars().count(), 80);
        assert_eq!(safe_name(&"a".repeat(80)), "a".repeat(80));
        assert_eq!(safe_name(&"a".repeat(81)), "a".repeat(80));
    }

    #[test]
    fn magic_cases() {
        assert_eq!(magic(&[0x89, b'P', b'N', b'G', 1, 2]), Some(ImgKind::Png));
        assert_eq!(magic(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImgKind::Jpeg));
        assert_eq!(magic(b"GIF89a.."), Some(ImgKind::Gif));
        assert_eq!(magic(b"GIF87a.."), Some(ImgKind::Gif));
        assert_eq!(magic(b"GIF88a.."), None);
        assert_eq!(magic(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImgKind::Webp));
        assert_eq!(magic(b"RIFF\0\0\0\0WEBP"), None); // exactly 12 bytes: too short
        assert_eq!(magic(b"RIFF\0\0\0\0WAVEfmt "), None);
        assert_eq!(magic(&[0x89, b'P', b'N']), None);
        assert_eq!(magic(&[0xFF, 0xD8]), None);
        assert_eq!(magic(b""), None);
        assert_eq!(magic(b"<svg"), None);
    }

    #[test]
    fn is_image_cases() {
        assert!(is_image("Image", "x"));
        assert!(is_image("IMAGE", ""));
        assert!(is_image("File", "a.PNG"));
        assert!(is_image("", "a.jpeg"));
        assert!(is_image("", "a.b.JPG"));
        assert!(is_image("", "a.gif"));
        assert!(is_image("", "a.webp"));
        assert!(!is_image("File", "a.svg"));
        assert!(!is_image("File", "png"));
        assert!(!is_image("File", "a.png.exe"));
        assert!(!is_image("Images", "a.txt"));
        assert!(!is_image("", ""));
    }
}
