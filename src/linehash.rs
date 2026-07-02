//! Line-hash mode: line-oriented, dialect-agnostic reads and anchors
//! (ADR-0001, ADR-0013), in the style of hashline.
//!
//! This mode does not parse — it works on raw text. It shares one line model
//! with the rest of the crate by using lispexp's [`LineIndex`] (a standalone
//! text utility, *not* the parse tree), so Line-hash line numbers agree with
//! the `Datum.line` that Structural mode reports — which Mode fallback
//! (ADR-0009) relies on.
//!
//! Line-ending policy (ADR-0008): a line's anchor hash is computed over the
//! line's content **excluding its terminator** ([`LineIndex::line_range`]), so
//! LF vs CRLF does not spuriously drift a line, and a lone `\r` is not a break.
//! A trailing newline is surfaced as an **empty final line**, making its
//! presence visible. The file-level hash still covers the whole verbatim byte
//! stream, so any line-ending change is caught there.

use lispexp::LineIndex;

use crate::hash::{anchor_hash, file_hash};

/// Render a file in the Line-hash mode read format: a `[path#FILEHASH]` header
/// followed by one `N:hash|content` line per source line (1-based `N`).
///
/// The per-line hash anchors that line for a later edit; the file-level hash in
/// the header guards the whole snapshot against drift (ADR-0008).
pub fn read(path_display: &str, source: &str) -> String {
    let index = LineIndex::new(source);
    let mut out = format!("[{path_display}#{}]\n", file_hash(source.as_bytes()));
    for n in 1..=index.line_count() as u32 {
        let content = &source[index.line_range(n).expect("n is within line_count")];
        out.push_str(&format!(
            "{n}:{}|{content}\n",
            anchor_hash(content.as_bytes())
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extract the hash column from a rendered `N:hash|content` body line.
    fn line_hash(rendered_line: &str) -> &str {
        rendered_line
            .split_once(':')
            .and_then(|(_, rest)| rest.split_once('|'))
            .map(|(h, _)| h)
            .unwrap()
    }

    #[test]
    fn renders_header_and_numbered_hashed_lines() {
        // No trailing newline: exactly two content lines.
        let rendered = read("a.el", "(defun f ())\n(defvar x 1)");

        let header = rendered.lines().next().unwrap();
        assert!(header.starts_with("[a.el#"));
        assert!(header.ends_with(']'));

        let mut body = rendered.lines().skip(1);
        let first = body.next().unwrap();
        assert!(first.starts_with("1:") && first.ends_with("|(defun f ())"));
        let second = body.next().unwrap();
        assert!(second.starts_with("2:") && second.ends_with("|(defvar x 1)"));
        assert!(body.next().is_none());
    }

    #[test]
    fn a_trailing_newline_is_shown_as_an_empty_final_line() {
        let rendered = read("d.scm", "(x)\n");
        let body: Vec<&str> = rendered.lines().skip(1).collect();
        assert_eq!(body.len(), 2);
        assert!(body[0].ends_with("|(x)"));
        assert!(body[1].starts_with("2:") && body[1].ends_with('|')); // empty content
    }

    #[test]
    fn crlf_and_lf_hash_the_same_line_content() {
        let lf = read("f", "(x)\n(y)");
        let crlf = read("f", "(x)\r\n(y)");
        let lf_hashes: Vec<&str> = lf.lines().skip(1).map(line_hash).collect();
        let crlf_hashes: Vec<&str> = crlf.lines().skip(1).map(line_hash).collect();
        assert_eq!(lf_hashes, crlf_hashes); // terminator excluded from the hash
    }

    #[test]
    fn identical_lines_share_a_line_hash() {
        let rendered = read("d.scm", "(x)\n(x)");
        let hashes: Vec<&str> = rendered.lines().skip(1).map(line_hash).collect();
        assert_eq!(hashes[0], hashes[1]);
    }
}
