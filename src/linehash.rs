//! Line-hash mode: line-oriented, dialect-agnostic reads and anchors
//! (ADR-0001, ADR-0013), in the style of hashline.
//!
//! This mode needs no parse tree — it works on raw text, so it is fully
//! decoupled from the lispexp reader.

use crate::hash::{anchor_hash, file_hash};

/// Render a file in the Line-hash mode read format: a `[path#FILEHASH]` header
/// followed by one `N:hash|content` line per source line (1-based `N`).
///
/// The per-line hash anchors that line for a later edit; the file-level hash in
/// the header guards the whole snapshot against drift (ADR-0008).
pub fn read(path_display: &str, source: &str) -> String {
    let mut out = format!("[{path_display}#{}]\n", file_hash(source.as_bytes()));
    for (i, line) in source.lines().enumerate() {
        let n = i + 1;
        let h = anchor_hash(line.as_bytes());
        out.push_str(&format!("{n}:{h}|{line}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_header_and_numbered_hashed_lines() {
        let rendered = read("a.el", "(defun f ())\n(defvar x 1)\n");
        let mut lines = rendered.lines();

        let header = lines.next().unwrap();
        assert!(header.starts_with("[a.el#"));
        assert!(header.ends_with(']'));

        let first = lines.next().unwrap();
        assert!(first.starts_with("1:"));
        assert!(first.ends_with("|(defun f ())"));

        let second = lines.next().unwrap();
        assert!(second.starts_with("2:"));
        assert!(second.ends_with("|(defvar x 1)"));

        assert!(lines.next().is_none());
    }

    #[test]
    fn identical_lines_share_a_line_hash() {
        let rendered = read("d.scm", "(x)\n(x)\n");
        let hashes: Vec<&str> = rendered
            .lines()
            .skip(1)
            .map(|l| l.split(':').nth(1).unwrap().split('|').next().unwrap())
            .collect();
        assert_eq!(hashes[0], hashes[1]);
    }
}
