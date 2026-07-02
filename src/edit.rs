//! Applying a batch of edits as **non-overlapping byte-range replacements**
//! against a single snapshot (ADR-0006 Batch: one snapshot, all-or-nothing).
//!
//! This is the shared core of both edit modes and involves no format or CLI
//! decisions: Line-hash edits map line ranges to byte ranges (here), and
//! Structural edits will map node spans to byte ranges. The produced string is
//! then handed to [`crate::write::verify_and_write`].
//!
//! Line spans are computed **including their terminator** (unlike
//! `LineIndex::line_range`, which excludes it — see
//! `docs/lispexp-feedback/0001`), so edits are byte-exact and reversible.

use std::ops::Range;

use lispexp::LineIndex;

/// A single replacement: substitute `text` for the bytes in `range` of the
/// original source. An empty `range` (`start == end`) is a pure insertion; an
/// empty `text` is a deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Byte range in the original source to replace.
    pub range: Range<usize>,
    /// Replacement text.
    pub text: String,
}

/// Why a [`splice`] was rejected.
#[derive(Debug, PartialEq, Eq)]
pub enum SpliceError {
    /// Two edits target overlapping byte ranges.
    Overlap,
    /// An edit range is inverted, past end-of-source, or not on a char boundary.
    OutOfBounds,
}

/// Apply `edits` to `source` as non-overlapping byte-range replacements, all
/// resolved against the **original** offsets (ADR-0006). `edits` order is
/// irrelevant — they are sorted internally — and the whole batch is
/// all-or-nothing: any overlap or bad range rejects the entire batch.
pub fn splice(source: &str, mut edits: Vec<Edit>) -> Result<String, SpliceError> {
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for edit in &edits {
        let Range { start, end } = edit.range;
        if start > end
            || end > source.len()
            || !source.is_char_boundary(start)
            || !source.is_char_boundary(end)
        {
            return Err(SpliceError::OutOfBounds);
        }
        if start < cursor {
            return Err(SpliceError::Overlap);
        }
        out.push_str(&source[cursor..start]);
        out.push_str(&edit.text);
        cursor = end;
    }
    out.push_str(&source[cursor..]);
    Ok(out)
}

/// A line-oriented edit operation referencing 1-based line numbers from the
/// read snapshot (Line-hash mode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineOp {
    /// Replace lines `start..=end` (inclusive) with `text`.
    Replace {
        /// First line to replace (1-based).
        start: u32,
        /// Last line to replace (1-based, inclusive).
        end: u32,
        /// Replacement text (verbatim, including its own terminators).
        text: String,
    },
    /// Delete lines `start..=end` (inclusive).
    Delete {
        /// First line to delete (1-based).
        start: u32,
        /// Last line to delete (1-based, inclusive).
        end: u32,
    },
    /// Insert `text` after line `after`; `after == 0` inserts before line 1.
    InsertAfter {
        /// Line to insert after (1-based; 0 = before the first line).
        after: u32,
        /// Text to insert (verbatim).
        text: String,
    },
}

/// Why an [`apply_line_ops`] batch was rejected.
#[derive(Debug, PartialEq, Eq)]
pub enum EditError {
    /// A referenced line number is outside the file.
    LineOutOfRange,
    /// The resulting byte edits could not be spliced.
    Splice(SpliceError),
}

/// Apply a batch of [`LineOp`]s to `source`, resolving every line number against
/// the original snapshot, and return the new content.
pub fn apply_line_ops(source: &str, ops: &[LineOp]) -> Result<String, EditError> {
    let index = LineIndex::new(source);
    let mut edits = Vec::with_capacity(ops.len());
    for op in ops {
        edits.push(match op {
            LineOp::Replace { start, end, text } => Edit {
                range: line_bytes(source, &index, *start, *end)?,
                text: text.clone(),
            },
            LineOp::Delete { start, end } => Edit {
                range: line_bytes(source, &index, *start, *end)?,
                text: String::new(),
            },
            LineOp::InsertAfter { after, text } => {
                let pos = if *after == 0 {
                    0
                } else {
                    full_line_span(source, &index, *after)
                        .ok_or(EditError::LineOutOfRange)?
                        .end
                };
                Edit {
                    range: pos..pos,
                    text: text.clone(),
                }
            }
        });
    }
    splice(source, edits).map_err(EditError::Splice)
}

/// The byte range spanning lines `start..=end` (inclusive), terminators
/// included.
fn line_bytes(
    source: &str,
    index: &LineIndex,
    start: u32,
    end: u32,
) -> Result<Range<usize>, EditError> {
    if start > end {
        return Err(EditError::LineOutOfRange);
    }
    let first = full_line_span(source, index, start).ok_or(EditError::LineOutOfRange)?;
    let last = full_line_span(source, index, end).ok_or(EditError::LineOutOfRange)?;
    Ok(first.start..last.end)
}

/// The full byte span of 1-based line `n`, **including its terminator** (the
/// verbatim bytes). `None` if `n` is out of range.
fn full_line_span(source: &str, index: &LineIndex, n: u32) -> Option<Range<usize>> {
    let start = index.line_range(n)?.start;
    let end = index
        .line_range(n + 1)
        .map(|r| r.start)
        .unwrap_or(source.len());
    Some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splice_replaces_inserts_and_deletes() {
        let edits = vec![
            Edit { range: 0..1, text: "P".into() }, // replace 'a'
            Edit { range: 3..3, text: "Z".into() }, // insert before 'd'
            Edit { range: 4..5, text: "".into() },  // delete 'e'
        ];
        assert_eq!(splice("abcdef", edits).unwrap(), "PbcZdf");
    }

    #[test]
    fn splice_rejects_overlap() {
        let edits = vec![
            Edit { range: 0..3, text: "X".into() },
            Edit { range: 2..4, text: "Y".into() },
        ];
        assert_eq!(splice("abcdef", edits), Err(SpliceError::Overlap));
    }

    #[test]
    fn splice_rejects_out_of_bounds() {
        let edits = vec![Edit { range: 0..99, text: "X".into() }];
        assert_eq!(splice("abc", edits), Err(SpliceError::OutOfBounds));
    }

    #[test]
    fn replace_a_line_keeps_the_others() {
        let out = apply_line_ops(
            "l1\nl2\nl3\n",
            &[LineOp::Replace { start: 2, end: 2, text: "L2\n".into() }],
        )
        .unwrap();
        assert_eq!(out, "l1\nL2\nl3\n");
    }

    #[test]
    fn delete_a_line() {
        let out = apply_line_ops("l1\nl2\nl3\n", &[LineOp::Delete { start: 2, end: 2 }]).unwrap();
        assert_eq!(out, "l1\nl3\n");
    }

    #[test]
    fn insert_after_a_line_and_before_the_first() {
        let out = apply_line_ops(
            "l1\nl2\n",
            &[
                LineOp::InsertAfter { after: 1, text: "mid\n".into() },
                LineOp::InsertAfter { after: 0, text: "top\n".into() },
            ],
        )
        .unwrap();
        assert_eq!(out, "top\nl1\nmid\nl2\n");
    }

    #[test]
    fn all_ops_resolve_against_the_original_snapshot() {
        // Deleting line 1 and replacing line 3 both use original numbering; the
        // delete must not shift the replace.
        let out = apply_line_ops(
            "a\nb\nc\n",
            &[
                LineOp::Delete { start: 1, end: 1 },
                LineOp::Replace { start: 3, end: 3, text: "C\n".into() },
            ],
        )
        .unwrap();
        assert_eq!(out, "b\nC\n");
    }

    #[test]
    fn out_of_range_line_is_rejected() {
        let err = apply_line_ops("a\n", &[LineOp::Delete { start: 9, end: 9 }]).unwrap_err();
        assert_eq!(err, EditError::LineOutOfRange);
    }
}
