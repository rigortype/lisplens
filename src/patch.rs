//! Parsing and applying the Line-hash Patch DSL (ADR-0021).
//!
//! A Patch is `@ <file-hash>` then one op per line, with heredoc payloads:
//!
//! ```text
//! @ 568dfe9533e9302f
//! replace 12:a3f2 <<END
//! (new content)
//! END
//! delete 20:b7e1
//! insert-after 25:c3d0 <<END
//! ;; a note
//! END
//! ```
//!
//! Replace substitutes a line's **content** (its terminator is preserved);
//! delete removes the whole line; insert adds text at a line boundary. Line
//! numbers and per-op hashes are verified against the snapshot before anything
//! is written, then the batch goes through [`verify_and_write`] (ADR-0005).

use std::path::Path;

use lispexp::{LineIndex, Options};

use crate::edit::{splice, Edit, SpliceError};
use crate::hash::{anchor_hash, file_hash};
use crate::write::{verify_and_write, WriteError};

/// A `line:hash` anchor, with an optional collision ordinal `line:hash:N`
/// (ADR-0018, ADR-0021).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// 1-based line.
    pub line: u32,
    /// The expected content hash.
    pub hash: String,
    /// Same-line collision ordinal, if present.
    pub ordinal: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Replace,
    Delete,
    InsertAfter,
    InsertBefore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpSpec {
    verb: Verb,
    anchor: Anchor,
    text: Option<String>,
}

/// A parsed Line-hash Patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinePatch {
    /// The snapshot the patch was built against.
    pub file_hash: String,
    ops: Vec<OpSpec>,
}

/// Why a Patch could not be parsed.
#[derive(Debug, PartialEq, Eq)]
pub enum PatchError {
    /// The leading `@ <file-hash>` header is missing.
    MissingHeader,
    /// An op line could not be understood.
    BadOp(String),
    /// An anchor was not `line:hash[:ordinal]`.
    BadAnchor(String),
    /// A heredoc payload was never closed.
    UnterminatedHeredoc,
}

/// Why a parsed Patch could not be applied.
#[derive(Debug)]
pub enum ApplyError {
    /// The file drifted from the patch's snapshot.
    Drift { expected: String, actual: String },
    /// An op referenced a line outside the file.
    LineOutOfRange(u32),
    /// An op's hash did not match the current line content.
    AnchorMismatch { line: u32, expected: String, actual: String },
    /// The edits could not be spliced.
    Splice(SpliceError),
    /// The safe write was refused.
    Write(WriteError),
    /// A filesystem error.
    Io(std::io::Error),
}

impl From<std::io::Error> for ApplyError {
    fn from(err: std::io::Error) -> Self {
        ApplyError::Io(err)
    }
}

/// The result of a successful Patch application (ADR-0023).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The file's hash after the write — the gate for a subsequent batch.
    pub new_file_hash: String,
}

fn parse_anchor(token: &str) -> Result<Anchor, PatchError> {
    let mut parts = token.split(':');
    let line = parts
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| PatchError::BadAnchor(token.to_string()))?;
    let hash = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| PatchError::BadAnchor(token.to_string()))?
        .to_string();
    let ordinal = match parts.next() {
        Some(s) => Some(s.parse::<u32>().map_err(|_| PatchError::BadAnchor(token.to_string()))?),
        None => None,
    };
    Ok(Anchor { line, hash, ordinal })
}

/// Parse a Line-hash Patch (ADR-0021).
pub fn parse_line_patch(input: &str) -> Result<LinePatch, PatchError> {
    let mut lines = input.lines().peekable();

    // Header: the first non-blank line must be `@ <file-hash>`.
    let file_hash = loop {
        match lines.next() {
            None => return Err(PatchError::MissingHeader),
            Some(line) if line.trim().is_empty() => continue,
            Some(line) => {
                let rest = line
                    .strip_prefix('@')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or(PatchError::MissingHeader)?;
                break rest.to_string();
            }
        }
    };

    let mut ops = Vec::new();
    while let Some(line) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let verb = match tokens.next() {
            Some("replace") => Verb::Replace,
            Some("delete") => Verb::Delete,
            Some("insert-after") => Verb::InsertAfter,
            Some("insert-before") => Verb::InsertBefore,
            _ => return Err(PatchError::BadOp(line.to_string())),
        };
        let anchor = parse_anchor(tokens.next().ok_or_else(|| PatchError::BadOp(line.to_string()))?)?;

        let needs_text = matches!(verb, Verb::Replace | Verb::InsertAfter | Verb::InsertBefore);
        let text = if needs_text {
            let tag = tokens
                .next()
                .and_then(|t| t.strip_prefix("<<"))
                .filter(|t| !t.is_empty())
                .ok_or_else(|| PatchError::BadOp(line.to_string()))?
                .to_string();
            Some(read_heredoc(&mut lines, &tag)?)
        } else {
            None
        };

        ops.push(OpSpec { verb, anchor, text });
    }

    Ok(LinePatch { file_hash, ops })
}

fn read_heredoc<'a>(
    lines: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>,
    tag: &str,
) -> Result<String, PatchError> {
    // Payload is content only — lines joined with `\n`, no trailing newline.
    // lisplens owns terminator placement (ADR-0011): a Replace keeps the line's
    // own terminator; an Insert has one added.
    let mut payload_lines = Vec::new();
    for line in lines.by_ref() {
        if line == tag {
            return Ok(payload_lines.join("\n"));
        }
        payload_lines.push(line);
    }
    Err(PatchError::UnterminatedHeredoc)
}

/// Apply a parsed Line-hash Patch to `path` (ADR-0021, ADR-0023).
pub fn apply_line_patch(
    path: &Path,
    patch: &LinePatch,
    options: &Options,
) -> Result<Outcome, ApplyError> {
    let source = std::fs::read_to_string(path)?;
    let actual = file_hash(source.as_bytes());
    if actual != patch.file_hash {
        return Err(ApplyError::Drift {
            expected: patch.file_hash.clone(),
            actual,
        });
    }

    let index = LineIndex::new(&source);
    let mut edits = Vec::with_capacity(patch.ops.len());
    for op in &patch.ops {
        let line = op.anchor.line;
        let content_range = index
            .line_range(line)
            .ok_or(ApplyError::LineOutOfRange(line))?;
        let content = &source[content_range.clone()];
        let found = anchor_hash(content.as_bytes());
        if found != op.anchor.hash {
            return Err(ApplyError::AnchorMismatch {
                line,
                expected: op.anchor.hash.clone(),
                actual: found,
            });
        }
        edits.push(build_edit(&source, &index, op, content_range));
    }

    let new_content = splice(&source, edits).map_err(ApplyError::Splice)?;
    verify_and_write(path, &patch.file_hash, &new_content, options).map_err(ApplyError::Write)?;
    Ok(Outcome {
        new_file_hash: file_hash(new_content.as_bytes()),
    })
}

fn build_edit(
    source: &str,
    index: &LineIndex,
    op: &OpSpec,
    content_range: std::ops::Range<usize>,
) -> Edit {
    let full = full_line_span(source, index, op.anchor.line);
    match op.verb {
        // Replace the line's content; its terminator is preserved.
        Verb::Replace => Edit {
            range: content_range,
            text: op.text.clone().unwrap_or_default(),
        },
        // Delete the whole line, terminator included.
        Verb::Delete => Edit { range: full, text: String::new() },
        // Insert a new line; lisplens supplies the terminator (ADR-0011).
        Verb::InsertAfter => Edit {
            range: full.end..full.end,
            text: format!("{}\n", op.text.clone().unwrap_or_default()),
        },
        Verb::InsertBefore => Edit {
            range: full.start..full.start,
            text: format!("{}\n", op.text.clone().unwrap_or_default()),
        },
    }
}

fn full_line_span(source: &str, index: &LineIndex, n: u32) -> std::ops::Range<usize> {
    let start = index.line_range(n).map(|r| r.start).unwrap_or(source.len());
    let end = index
        .line_range(n + 1)
        .map(|r| r.start)
        .unwrap_or(source.len());
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{anchor_hash, file_hash};

    fn temp(content: &str) -> (tempfile::TempDir, std::path::PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.scm");
        std::fs::write(&path, content).unwrap();
        (dir, path, file_hash(content.as_bytes()))
    }

    fn line_hash(content: &str) -> String {
        anchor_hash(content.as_bytes())
    }

    #[test]
    fn parses_header_and_ops() {
        let patch = parse_line_patch("@ abc123\ndelete 3:9999\n").unwrap();
        assert_eq!(patch.file_hash, "abc123");
        assert_eq!(patch.ops.len(), 1);
    }

    #[test]
    fn missing_header_is_an_error() {
        assert_eq!(parse_line_patch("delete 1:aaaa\n"), Err(PatchError::MissingHeader));
    }

    #[test]
    fn replace_substitutes_content_and_keeps_the_terminator() {
        let (_d, path, fh) = temp("(a)\n(b)\n(c)\n");
        let h = line_hash("(b)");
        let patch = parse_line_patch(&format!("@ {fh}\nreplace 2:{h} <<END\n(B)\nEND\n")).unwrap();
        let out = apply_line_patch(&path, &patch, &Options::scheme()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(a)\n(B)\n(c)\n");
        assert_eq!(out.new_file_hash, file_hash("(a)\n(B)\n(c)\n".as_bytes()));
    }

    #[test]
    fn delete_removes_the_whole_line() {
        let (_d, path, fh) = temp("(a)\n(b)\n(c)\n");
        let h = line_hash("(b)");
        let patch = parse_line_patch(&format!("@ {fh}\ndelete 2:{h}\n")).unwrap();
        apply_line_patch(&path, &patch, &Options::scheme()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(a)\n(c)\n");
    }

    #[test]
    fn insert_after_adds_a_line() {
        let (_d, path, fh) = temp("(a)\n(c)\n");
        let h = line_hash("(a)");
        let patch =
            parse_line_patch(&format!("@ {fh}\ninsert-after 1:{h} <<END\n(b)\nEND\n")).unwrap();
        apply_line_patch(&path, &patch, &Options::scheme()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(a)\n(b)\n(c)\n");
    }

    #[test]
    fn a_wrong_line_hash_is_refused() {
        let (_d, path, fh) = temp("(a)\n(b)\n");
        let patch = parse_line_patch(&format!("@ {fh}\ndelete 2:0000\n")).unwrap();
        let err = apply_line_patch(&path, &patch, &Options::scheme()).unwrap_err();
        assert!(matches!(err, ApplyError::AnchorMismatch { line: 2, .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(a)\n(b)\n"); // untouched
    }

    #[test]
    fn a_drifted_file_is_refused() {
        let (_d, path, _fh) = temp("(a)\n");
        let patch = parse_line_patch("@ deadbeef\ndelete 1:0000\n").unwrap();
        assert!(matches!(
            apply_line_patch(&path, &patch, &Options::scheme()),
            Err(ApplyError::Drift { .. })
        ));
    }
}
