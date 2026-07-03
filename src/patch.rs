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

use lispexp::{Datum, DatumKind, Dialect, LineIndex, Options};

use crate::edit::{splice, splice_tracked, Edit, SpliceError};
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
    AnchorMismatch {
        line: u32,
        expected: String,
        actual: String,
    },
    /// No node matched a Structural anchor.
    AnchorNotFound { line: u32, hash: String },
    /// An op cannot apply to the resolved node (e.g. raise of a top-level node,
    /// splice of a non-list).
    NotApplicable(String),
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
    /// Validate-then-write warnings: definitions no longer recognized after the
    /// edit (ADR-0024). The edit still succeeded.
    pub warnings: Vec<String>,
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
        Some(s) => Some(
            s.parse::<u32>()
                .map_err(|_| PatchError::BadAnchor(token.to_string()))?,
        ),
        None => None,
    };
    Ok(Anchor {
        line,
        hash,
        ordinal,
    })
}

/// Parse a Line-hash Patch (ADR-0021).
/// Read the leading `@ <file-hash>` header (skipping blank lines).
fn parse_header<'a>(
    lines: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>,
) -> Result<String, PatchError> {
    loop {
        match lines.next() {
            None => return Err(PatchError::MissingHeader),
            Some(line) if line.trim().is_empty() => continue,
            Some(line) => {
                let rest = line
                    .strip_prefix('@')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or(PatchError::MissingHeader)?;
                return Ok(rest.to_string());
            }
        }
    }
}

pub fn parse_line_patch(input: &str) -> Result<LinePatch, PatchError> {
    let mut lines = input.lines().peekable();
    let file_hash = parse_header(&mut lines)?;
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
        let anchor = parse_anchor(
            tokens
                .next()
                .ok_or_else(|| PatchError::BadOp(line.to_string()))?,
        )?;

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
    dialect: Dialect,
) -> Result<Outcome, ApplyError> {
    let options = Options::for_dialect(dialect);
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
    verify_and_write(path, &patch.file_hash, &new_content, &options).map_err(ApplyError::Write)?;
    Ok(Outcome {
        new_file_hash: file_hash(new_content.as_bytes()),
        warnings: crate::disappeared_definitions(&source, &new_content, dialect),
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
        Verb::Delete => Edit {
            range: full,
            text: String::new(),
        },
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

// --- Structural patches (ADR-0021) -----------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SVerb {
    Replace,
    Delete,
    Wrap,
    Raise,
    Splice,
    SlurpFwd,
    SlurpBack,
    BarfFwd,
    BarfBack,
    Split,
    Join,
    Rename,
    Format,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SOpSpec {
    verb: SVerb,
    anchor: Anchor,
    text: Option<String>,
    /// Child index for `split`.
    index: Option<usize>,
    /// The second list for `join`.
    anchor2: Option<Anchor>,
    /// `(from, to)` symbols for `rename`.
    rename: Option<(String, String)>,
}

/// A parsed Structural Patch, covering the full op set (ADR-0012, ADR-0021):
/// `replace`, `delete`, `wrap`, `raise`, `splice`, `slurp-fwd`, `slurp-back`,
/// `barf-fwd`, `barf-back`, `split`, `join`, `rename`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructPatch {
    /// The snapshot the patch was built against.
    pub file_hash: String,
    ops: Vec<SOpSpec>,
}

/// Parse a Structural Patch (ADR-0021).
pub fn parse_struct_patch(input: &str) -> Result<StructPatch, PatchError> {
    let mut lines = input.lines().peekable();
    let file_hash = parse_header(&mut lines)?;
    let mut ops = Vec::new();
    while let Some(line) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let verb = match tokens.next() {
            Some("replace") => SVerb::Replace,
            Some("delete") => SVerb::Delete,
            Some("wrap") => SVerb::Wrap,
            Some("raise") => SVerb::Raise,
            Some("splice") => SVerb::Splice,
            Some("slurp-fwd") => SVerb::SlurpFwd,
            Some("slurp-back") => SVerb::SlurpBack,
            Some("barf-fwd") => SVerb::BarfFwd,
            Some("barf-back") => SVerb::BarfBack,
            Some("split") => SVerb::Split,
            Some("join") => SVerb::Join,
            Some("rename") => SVerb::Rename,
            Some("format") => SVerb::Format,
            _ => return Err(PatchError::BadOp(line.to_string())),
        };
        let bad = || PatchError::BadOp(line.to_string());
        let anchor = parse_anchor(tokens.next().ok_or_else(bad)?)?;

        let mut text = None;
        let mut index = None;
        let mut anchor2 = None;
        let mut rename = None;
        match verb {
            SVerb::Replace | SVerb::Wrap => {
                let tag = tokens
                    .next()
                    .and_then(|t| t.strip_prefix("<<"))
                    .filter(|t| !t.is_empty())
                    .ok_or_else(bad)?
                    .to_string();
                text = Some(read_heredoc(&mut lines, &tag)?);
            }
            SVerb::Split => {
                let n = tokens
                    .next()
                    .and_then(|t| t.strip_prefix('@'))
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or_else(bad)?;
                index = Some(n);
            }
            SVerb::Join => anchor2 = Some(parse_anchor(tokens.next().ok_or_else(bad)?)?),
            SVerb::Rename => {
                let from = tokens.next().ok_or_else(bad)?.to_string();
                let to = tokens.next().ok_or_else(bad)?.to_string();
                rename = Some((from, to));
            }
            _ => {}
        }
        ops.push(SOpSpec {
            verb,
            anchor,
            text,
            index,
            anchor2,
            rename,
        });
    }
    Ok(StructPatch { file_hash, ops })
}

/// Apply a parsed Structural Patch to `path` (ADR-0021, ADR-0023).
pub fn apply_struct_patch(
    path: &Path,
    patch: &StructPatch,
    dialect: Dialect,
) -> Result<Outcome, ApplyError> {
    let options = Options::for_dialect(dialect);
    let source = std::fs::read_to_string(path)?;
    let actual = file_hash(source.as_bytes());
    if actual != patch.file_hash {
        return Err(ApplyError::Drift {
            expected: patch.file_hash.clone(),
            actual,
        });
    }

    let parsed = lispexp::parse(&source, &options);
    let mut edits = Vec::new();
    // Indices in `edits` produced by `format` ops — their identity edits reindent
    // exactly the anchored form, not the whole enclosing top-level form.
    let mut format_edits: Vec<usize> = Vec::new();
    for op in &patch.ops {
        let located =
            crate::resolve::resolve(&source, &parsed.data, &op.anchor).ok_or_else(|| {
                ApplyError::AnchorNotFound {
                    line: op.anchor.line,
                    hash: op.anchor.hash.clone(),
                }
            })?;
        let op_edits = build_struct_edits(&source, &parsed.data, op, &located)?;
        if matches!(op.verb, SVerb::Format) {
            format_edits.extend(edits.len()..edits.len() + op_edits.len());
        }
        edits.extend(op_edits);
    }

    let (spliced, spans) = splice_tracked(&source, edits).map_err(ApplyError::Splice)?;
    // Auto-format the touched region (ADR-0025/0028): a content edit reindents
    // its whole enclosing top-level form (`expand`); a `format` op reindents
    // exactly its anchored form (`exact`, ADR-0028 point 3). Everything else stays
    // byte-identical. Structural only — Line-hash stays literal (ADR-0027). Gated
    // to dialects with a faithful native engine (Emacs Lisp, Common Lisp); the
    // generic fallback is not trusted to auto-reflow a dialect it doesn't model.
    let new_content = if crate::format::has_native_engine(dialect) {
        let config = crate::config::resolve(path, &spliced);
        let expand: Vec<_> = spans
            .iter()
            .enumerate()
            .filter(|(i, _)| !format_edits.contains(i))
            .map(|(_, r)| r.clone())
            .collect();
        let exact: Vec<_> = format_edits.iter().map(|&i| spans[i].clone()).collect();
        // Keep Nameless-indented files (e.g. php-mode/lisp) from being reflowed
        // to non-Nameless columns when a config signal marks them (ADR-0030).
        let nl = config.nameless.then(|| {
            let file_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            crate::nameless::Nameless::for_file(file_name)
        });
        crate::format::reindent(
            &spliced,
            &config,
            dialect,
            nl.as_ref(),
            crate::format::Touched {
                expand: &expand,
                exact: &exact,
            },
        )
    } else {
        spliced
    };
    verify_and_write(path, &patch.file_hash, &new_content, &options).map_err(ApplyError::Write)?;
    Ok(Outcome {
        new_file_hash: file_hash(new_content.as_bytes()),
        warnings: crate::disappeared_definitions(&source, &new_content, dialect),
    })
}

fn build_struct_edits(
    source: &str,
    data: &[Datum],
    op: &SOpSpec,
    located: &crate::resolve::Located,
) -> Result<Vec<Edit>, ApplyError> {
    use crate::structural as st;
    let node = located.node;
    let span = node.span.start as usize..node.span.end as usize;
    let na = |msg: &str| ApplyError::NotApplicable(msg.to_string());
    Ok(match op.verb {
        SVerb::Replace => vec![Edit {
            range: span,
            text: op.text.clone().unwrap_or_default(),
        }],
        SVerb::Delete => vec![Edit {
            range: span,
            text: String::new(),
        }],
        SVerb::Wrap => st::wrap(node, op.text.as_deref().unwrap_or_default()),
        SVerb::Raise => {
            let parent = located
                .parent
                .ok_or_else(|| na("raise: node has no parent"))?;
            st::raise(source, parent, node)
        }
        SVerb::Splice => {
            st::splice(source, node).ok_or_else(|| na("splice: node is not a list"))?
        }
        SVerb::SlurpFwd => {
            let next = sibling(located, 1).ok_or_else(|| na("slurp-fwd: no next sibling"))?;
            st::slurp_forward(node, next).ok_or_else(|| na("slurp-fwd: node is not a list"))?
        }
        SVerb::SlurpBack => {
            let prev = sibling(located, -1).ok_or_else(|| na("slurp-back: no previous sibling"))?;
            st::slurp_backward(node, prev).ok_or_else(|| na("slurp-back: node is not a list"))?
        }
        SVerb::BarfFwd => {
            st::barf_forward(node).ok_or_else(|| na("barf-fwd: empty or non-list"))?
        }
        SVerb::BarfBack => {
            st::barf_backward(node).ok_or_else(|| na("barf-back: empty or non-list"))?
        }
        SVerb::Split => {
            let index = op.index.ok_or_else(|| na("split: missing index"))?;
            st::split(node, index).ok_or_else(|| na("split: bad index or non-list"))?
        }
        SVerb::Join => {
            let a2 = op
                .anchor2
                .as_ref()
                .ok_or_else(|| na("join: missing second anchor"))?;
            let second = crate::resolve::resolve(source, data, a2).ok_or_else(|| {
                ApplyError::AnchorNotFound {
                    line: a2.line,
                    hash: a2.hash.clone(),
                }
            })?;
            st::join(node, second.node).ok_or_else(|| na("join: both must be lists"))?
        }
        SVerb::Rename => {
            let (from, to) = op
                .rename
                .as_ref()
                .ok_or_else(|| na("rename: missing from/to"))?;
            st::rename(node, from, to)
        }
        // An identity edit: it changes nothing, but records the node's span so
        // the post-splice reindent can format exactly this form (ADR-0028 point
        // 3). The actual reindent runs in `apply_struct_patch`.
        SVerb::Format => vec![Edit {
            range: span.clone(),
            text: source[span].to_string(),
        }],
    })
}

/// The sibling of the located node at `offset` (+1 next, -1 previous), or
/// `None` if there is no parent or no such sibling.
fn sibling<'a, 't>(
    located: &crate::resolve::Located<'a, 't>,
    offset: isize,
) -> Option<&'a Datum<'t>> {
    let DatumKind::List { items, .. } = &located.parent?.kind else {
        return None;
    };
    let target = located.index?.checked_add_signed(offset)?;
    items.get(target)
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
    fn struct_edit_auto_format_honors_nameless_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".dir-locals.el"),
            "((emacs-lisp-mode (nameless-mode . t)))\n",
        )
        .unwrap();
        let path = dir.path().join("php-mode.el"); // current-name → "php"
        let source = "(defun php-mode-x ()\n(php-mode-some-function arg1\nanother-arg))\n";
        std::fs::write(&path, source).unwrap();
        let fh = file_hash(source.as_bytes());
        let h = anchor_hash(
            "(defun php-mode-x ()\n(php-mode-some-function arg1\nanother-arg))".as_bytes(),
        );
        let patch = parse_struct_patch(&format!("@ {fh}\nformat 1:{h}\n")).unwrap();
        apply_struct_patch(&path, &patch, Dialect::EmacsLisp).unwrap();
        // `php-` composes to `:`, so `another-arg` aligns at column 23, not 26.
        let expected = format!(
            "(defun php-mode-x ()\n  (php-mode-some-function arg1\n{}another-arg))\n",
            " ".repeat(23)
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    }

    #[test]
    fn format_op_reindents_exactly_the_anchored_nested_form() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.el");
        // `(bar a / b)` nested in `progn`; both it and its sibling `(baz c / d)`
        // are misindented.
        let source = "(progn\n  (bar a\nb)\n(baz c\nd))\n";
        std::fs::write(&path, source).unwrap();
        let fh = file_hash(source.as_bytes());
        let h = anchor_hash("(bar a\nb)".as_bytes());
        let patch = parse_struct_patch(&format!("@ {fh}\nformat 2:{h}\n")).unwrap();
        apply_struct_patch(&path, &patch, Dialect::EmacsLisp).unwrap();
        // Only the anchored form is reindented; the sibling stays byte-identical.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "(progn\n  (bar a\n       b)\n(baz c\nd))\n"
        );
    }

    #[test]
    fn struct_edit_auto_formats_only_the_touched_form() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.el");
        let source = "(defun a ()\n  (x))\n(defun b ()\n  (y))\n";
        std::fs::write(&path, source).unwrap();
        let fh = file_hash(source.as_bytes());
        // Replace `(x)` (line 2) with a flat multi-line body.
        let h = anchor_hash("(x)".as_bytes());
        let patch = parse_struct_patch(&format!(
            "@ {fh}\nreplace 2:{h} <<END\n(when c\n(foo)\n(bar))\nEND\n"
        ))
        .unwrap();
        let out = apply_struct_patch(&path, &patch, Dialect::EmacsLisp).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        // Form `a` (touched) is reindented; form `b` stays byte-identical.
        assert_eq!(
            written,
            "(defun a ()\n  (when c\n    (foo)\n    (bar)))\n(defun b ()\n  (y))\n"
        );
        assert_eq!(out.new_file_hash, file_hash(written.as_bytes()));
    }

    #[test]
    fn missing_header_is_an_error() {
        assert_eq!(
            parse_line_patch("delete 1:aaaa\n"),
            Err(PatchError::MissingHeader)
        );
    }

    #[test]
    fn replace_substitutes_content_and_keeps_the_terminator() {
        let (_d, path, fh) = temp("(a)\n(b)\n(c)\n");
        let h = line_hash("(b)");
        let patch = parse_line_patch(&format!("@ {fh}\nreplace 2:{h} <<END\n(B)\nEND\n")).unwrap();
        let out = apply_line_patch(&path, &patch, Dialect::Scheme).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(a)\n(B)\n(c)\n");
        assert_eq!(out.new_file_hash, file_hash("(a)\n(B)\n(c)\n".as_bytes()));
    }

    #[test]
    fn delete_removes_the_whole_line() {
        let (_d, path, fh) = temp("(a)\n(b)\n(c)\n");
        let h = line_hash("(b)");
        let patch = parse_line_patch(&format!("@ {fh}\ndelete 2:{h}\n")).unwrap();
        apply_line_patch(&path, &patch, Dialect::Scheme).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(a)\n(c)\n");
    }

    #[test]
    fn insert_after_adds_a_line() {
        let (_d, path, fh) = temp("(a)\n(c)\n");
        let h = line_hash("(a)");
        let patch =
            parse_line_patch(&format!("@ {fh}\ninsert-after 1:{h} <<END\n(b)\nEND\n")).unwrap();
        apply_line_patch(&path, &patch, Dialect::Scheme).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(a)\n(b)\n(c)\n");
    }

    #[test]
    fn a_wrong_line_hash_is_refused() {
        let (_d, path, fh) = temp("(a)\n(b)\n");
        let patch = parse_line_patch(&format!("@ {fh}\ndelete 2:0000\n")).unwrap();
        let err = apply_line_patch(&path, &patch, Dialect::Scheme).unwrap_err();
        assert!(matches!(err, ApplyError::AnchorMismatch { line: 2, .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(a)\n(b)\n"); // untouched
    }

    #[test]
    fn a_drifted_file_is_refused() {
        let (_d, path, _fh) = temp("(a)\n");
        let patch = parse_line_patch("@ deadbeef\ndelete 1:0000\n").unwrap();
        assert!(matches!(
            apply_line_patch(&path, &patch, Dialect::Scheme),
            Err(ApplyError::Drift { .. })
        ));
    }

    fn node_hash(text: &str) -> String {
        anchor_hash(text.as_bytes())
    }

    #[test]
    fn an_edit_that_unrecognizes_a_definition_warns() {
        let (_d, path, fh) = temp("(defun foo () 1)\n");
        let h = line_hash("(defun foo () 1)");
        let patch =
            parse_line_patch(&format!("@ {fh}\nreplace 1:{h} <<END\n(defun)\nEND\n")).unwrap();
        let outcome = apply_line_patch(&path, &patch, Dialect::EmacsLisp).unwrap();
        assert!(
            outcome.warnings.iter().any(|w| w.contains("foo")),
            "{:?}",
            outcome.warnings
        );
    }

    #[test]
    fn struct_replace_swaps_a_definition_node() {
        let (_d, path, fh) = temp("(define x 1)\n(define y 2)\n");
        let h = node_hash("(define y 2)");
        let patch = parse_struct_patch(&format!(
            "@ {fh}\nreplace 2:{h} <<END\n(define y 22)\nEND\n"
        ))
        .unwrap();
        apply_struct_patch(&path, &patch, Dialect::Scheme).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "(define x 1)\n(define y 22)\n"
        );
    }

    #[test]
    fn struct_wrap_encloses_a_form() {
        let (_d, path, fh) = temp("body\n");
        let h = node_hash("body");
        let patch =
            parse_struct_patch(&format!("@ {fh}\nwrap 1:{h} <<END\nwhen cond\nEND\n")).unwrap();
        apply_struct_patch(&path, &patch, Dialect::Scheme).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "(when cond body)\n"
        );
    }

    #[test]
    fn struct_raise_replaces_the_parent_form() {
        let (_d, path, fh) = temp("(when cond (do-thing))\n");
        let h = node_hash("(do-thing)");
        let patch = parse_struct_patch(&format!("@ {fh}\nraise 1:{h}\n")).unwrap();
        apply_struct_patch(&path, &patch, Dialect::Scheme).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(do-thing)\n");
    }

    #[test]
    fn struct_splice_removes_inner_delimiters() {
        let (_d, path, fh) = temp("(foo (bar baz) quux)\n");
        let h = node_hash("(bar baz)");
        let patch = parse_struct_patch(&format!("@ {fh}\nsplice 1:{h}\n")).unwrap();
        apply_struct_patch(&path, &patch, Dialect::Scheme).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "(foo bar baz quux)\n"
        );
    }

    fn apply_struct(source: &str, op_line: &str, node_text: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.scm");
        std::fs::write(&path, source).unwrap();
        let fh = file_hash(source.as_bytes());
        let h = node_hash(node_text);
        let patch = parse_struct_patch(&format!("@ {fh}\n{op_line} 1:{h}\n")).unwrap();
        apply_struct_patch(&path, &patch, Dialect::Scheme).unwrap();
        std::fs::read_to_string(&path).unwrap()
    }

    #[test]
    fn struct_slurp_forward_swallows_the_next_sibling() {
        assert_eq!(
            apply_struct("(foo (bar) baz)\n", "slurp-fwd", "(bar)"),
            "(foo (bar baz))\n"
        );
    }

    #[test]
    fn struct_barf_forward_expels_the_last_element() {
        assert_eq!(
            apply_struct("(foo (bar baz))\n", "barf-fwd", "(bar baz)"),
            "(foo (bar) baz)\n"
        );
    }

    #[test]
    fn struct_split_divides_a_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.scm");
        std::fs::write(&path, "(a b)\n").unwrap();
        let fh = file_hash("(a b)\n".as_bytes());
        let h = node_hash("(a b)");
        let patch = parse_struct_patch(&format!("@ {fh}\nsplit 1:{h} @0\n")).unwrap();
        apply_struct_patch(&path, &patch, Dialect::Scheme).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(a) (b)\n");
    }

    #[test]
    fn struct_join_merges_two_lists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.scm");
        std::fs::write(&path, "(a) (b)\n").unwrap();
        let fh = file_hash("(a) (b)\n".as_bytes());
        let h1 = node_hash("(a)");
        let h2 = node_hash("(b)");
        let patch = parse_struct_patch(&format!("@ {fh}\njoin 1:{h1} 1:{h2}\n")).unwrap();
        apply_struct_patch(&path, &patch, Dialect::Scheme).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(a b)\n");
    }

    #[test]
    fn struct_rename_replaces_occurrences_in_the_anchored_subtree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.scm");
        let src = "(define (f x) (+ x x))\n";
        std::fs::write(&path, src).unwrap();
        let fh = file_hash(src.as_bytes());
        let h = node_hash("(define (f x) (+ x x))");
        let patch = parse_struct_patch(&format!("@ {fh}\nrename 1:{h} x y\n")).unwrap();
        apply_struct_patch(&path, &patch, Dialect::Scheme).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "(define (f y) (+ y y))\n"
        );
    }

    #[test]
    fn struct_raise_of_a_top_level_node_is_refused() {
        let (_d, path, fh) = temp("(define x 1)\n");
        let h = node_hash("(define x 1)");
        let patch = parse_struct_patch(&format!("@ {fh}\nraise 1:{h}\n")).unwrap();
        assert!(matches!(
            apply_struct_patch(&path, &patch, Dialect::Scheme),
            Err(ApplyError::NotApplicable(_))
        ));
    }
}
