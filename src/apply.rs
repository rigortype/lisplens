//! End-to-end wiring: turn a batch of edits into a safe write (ADR-0005,
//! ADR-0006, ADR-0017).
//!
//! A caller produces edits — line ops (Line-hash mode) or byte-range edits
//! (Structural mode) — against a read snapshot; these functions re-read the
//! file, confirm it has not drifted, splice the edits, and hand the result to
//! [`verify_and_write`]. Library-level only: no CLI or patch syntax.

use std::path::Path;

use lispexp::Options;

use crate::edit::{apply_line_ops, splice, Edit, EditError, LineOp, SpliceError};
use crate::hash::file_hash;
use crate::write::{verify_and_write, WriteError};

/// Why an apply was refused.
#[derive(Debug)]
pub enum ApplyError {
    /// The file drifted since the read the edit was based on.
    Drift {
        /// The file hash the edit was based on.
        expected: String,
        /// The file's current hash.
        actual: String,
    },
    /// A line-op batch could not be mapped to edits.
    Edit(EditError),
    /// Byte-range edits could not be spliced (overlap / out of bounds).
    Splice(SpliceError),
    /// The safe write was refused (drift, new parse errors, I/O).
    Write(WriteError),
    /// A filesystem error.
    Io(std::io::Error),
}

impl From<std::io::Error> for ApplyError {
    fn from(err: std::io::Error) -> Self {
        ApplyError::Io(err)
    }
}

/// Apply a Line-hash [`LineOp`] batch to `path`, gated on `expected_file_hash`.
pub fn apply_line_ops_to_file(
    path: &Path,
    expected_file_hash: &str,
    ops: &[LineOp],
    options: &Options,
) -> Result<(), ApplyError> {
    let source = read_undrifted(path, expected_file_hash)?;
    let new_content = apply_line_ops(&source, ops).map_err(ApplyError::Edit)?;
    verify_and_write(path, expected_file_hash, &new_content, options).map_err(ApplyError::Write)
}

/// Apply a Structural byte-range [`Edit`] batch to `path`, gated on
/// `expected_file_hash`.
pub fn apply_edits_to_file(
    path: &Path,
    expected_file_hash: &str,
    edits: Vec<Edit>,
    options: &Options,
) -> Result<(), ApplyError> {
    let source = read_undrifted(path, expected_file_hash)?;
    let new_content = splice(&source, edits).map_err(ApplyError::Splice)?;
    verify_and_write(path, expected_file_hash, &new_content, options).map_err(ApplyError::Write)
}

/// Read `path`, confirming its current file hash matches `expected` (so edits
/// are spliced against the snapshot they were computed from). [`verify_and_write`]
/// re-checks drift at write time; this earlier check keeps the splice honest.
fn read_undrifted(path: &Path, expected: &str) -> Result<String, ApplyError> {
    let source = std::fs::read_to_string(path)?;
    let actual = file_hash(source.as_bytes());
    if actual != expected {
        return Err(ApplyError::Drift {
            expected: expected.to_string(),
            actual,
        });
    }
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::file_hash;

    fn temp_file(content: &str) -> (tempfile::TempDir, std::path::PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.scm");
        std::fs::write(&path, content).unwrap();
        let hash = file_hash(content.as_bytes());
        (dir, path, hash)
    }

    #[test]
    fn line_ops_apply_end_to_end() {
        let (_dir, path, hash) = temp_file("l1\nl2\nl3\n");
        apply_line_ops_to_file(
            &path,
            &hash,
            &[LineOp::Delete { start: 2, end: 2 }],
            &Options::scheme(),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "l1\nl3\n");
    }

    #[test]
    fn byte_edits_apply_end_to_end() {
        let (_dir, path, hash) = temp_file("(f x)\n");
        let edits = vec![Edit { range: 3..4, text: "y".into() }]; // x -> y
        apply_edits_to_file(&path, &hash, edits, &Options::scheme()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(f y)\n");
    }

    #[test]
    fn a_drifted_file_is_refused_before_splicing() {
        let (_dir, path, _hash) = temp_file("l1\nl2\n");
        let stale = file_hash("something else".as_bytes());
        let err = apply_line_ops_to_file(
            &path,
            &stale,
            &[LineOp::Delete { start: 1, end: 1 }],
            &Options::scheme(),
        )
        .unwrap_err();
        assert!(matches!(err, ApplyError::Drift { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "l1\nl2\n"); // untouched
    }

    #[test]
    fn an_edit_that_breaks_syntax_is_refused_by_the_writer() {
        let (_dir, path, hash) = temp_file("(f x)\n");
        // Delete the closing paren -> unbalanced -> verify_and_write refuses.
        let edits = vec![Edit { range: 4..5, text: "".into() }];
        let err = apply_edits_to_file(&path, &hash, edits, &Options::scheme()).unwrap_err();
        assert!(matches!(err, ApplyError::Write(WriteError::NewParseErrors { .. })));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(f x)\n"); // untouched
    }
}
