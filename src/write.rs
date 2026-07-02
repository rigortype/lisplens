//! Safe writes: drift check, validate-then-write, atomic replace
//! (ADR-0005, ADR-0008).
//!
//! The safety contract is simply: **never make a file's syntax worse.** An edit
//! is refused if the file drifted since the read it was based on, or if it would
//! introduce new parse errors; otherwise the new content is written atomically.
//! Shared by both edit modes and independent of how the edit was expressed.

use std::path::Path;

use lispexp::{parse, Options};

use crate::hash::file_hash;

/// Why a safe write was refused.
#[derive(Debug)]
pub enum WriteError {
    /// The file changed since the read the edit was based on: its current
    /// file-level hash does not match the expected one.
    Drift {
        /// The file hash the edit was based on.
        expected: String,
        /// The file's current hash.
        actual: String,
    },
    /// The edit would raise the parse-error count above the pre-edit baseline
    /// (ADR-0005). Edits that repair or preserve validity are allowed.
    NewParseErrors {
        /// Parse-error count before the edit.
        before: usize,
        /// Parse-error count the edit would produce.
        after: usize,
    },
    /// A filesystem error while reading or writing.
    Io(std::io::Error),
}

impl From<std::io::Error> for WriteError {
    fn from(err: std::io::Error) -> Self {
        WriteError::Io(err)
    }
}

/// Verify no drift, verify the edit introduces no new parse errors, then write
/// `new_content` to `path` atomically.
///
/// `expected_file_hash` is the [`file_hash`](crate::hash::file_hash) the edit
/// was based on. `options` selects the dialect for the parse-error baseline.
///
/// Note: the baseline comparison is by error *count* for now; ADR-0005's "new
/// errors" is ideally a set/position diff — a later refinement behind this same
/// signature.
pub fn verify_and_write(
    path: &Path,
    expected_file_hash: &str,
    new_content: &str,
    options: &Options,
) -> Result<(), WriteError> {
    let current = std::fs::read_to_string(path)?;

    let actual = file_hash(current.as_bytes());
    if actual != expected_file_hash {
        return Err(WriteError::Drift {
            expected: expected_file_hash.to_string(),
            actual,
        });
    }

    let before = parse(&current, options).errors.len();
    let after = parse(new_content, options).errors.len();
    if after > before {
        return Err(WriteError::NewParseErrors { before, after });
    }

    write_atomically(path, new_content)?;
    Ok(())
}

/// Write `content` to `path` atomically: write a sibling temp file, then rename
/// it over the target (atomic within one filesystem).
fn write_atomically(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = dir.unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out".to_string());
    let tmp = dir.join(format!(".{name}.lisplens.tmp"));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::file_hash;

    fn write_file(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn writes_a_valid_edit_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(dir.path(), "a.scm", "(f 1)\n");
        let expected = file_hash("(f 1)\n".as_bytes());

        verify_and_write(&path, &expected, "(f 2)\n", &Options::scheme()).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(f 2)\n");
    }

    #[test]
    fn refuses_a_drifted_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(dir.path(), "a.scm", "(f 1)\n");
        let stale = file_hash("(f 1)\n".as_bytes());
        std::fs::write(&path, "(f 999)\n").unwrap(); // someone else edited it

        let err = verify_and_write(&path, &stale, "(f 2)\n", &Options::scheme()).unwrap_err();

        assert!(matches!(err, WriteError::Drift { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(f 999)\n");
    }

    #[test]
    fn refuses_an_edit_that_introduces_parse_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(dir.path(), "a.scm", "(f 1)\n");
        let expected = file_hash("(f 1)\n".as_bytes());

        let err = verify_and_write(&path, &expected, "(f 1\n", &Options::scheme()).unwrap_err();

        assert!(matches!(err, WriteError::NewParseErrors { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(f 1)\n"); // unchanged
    }

    #[test]
    fn allows_an_edit_that_repairs_a_broken_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(dir.path(), "a.scm", "(f 1\n"); // already broken
        let expected = file_hash("(f 1\n".as_bytes());

        verify_and_write(&path, &expected, "(f 1)\n", &Options::scheme()).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(f 1)\n");
    }
}
