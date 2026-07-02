//! Safe writes: drift check, validate-then-write, atomic replace
//! (ADR-0005, ADR-0008, ADR-0017).
//!
//! The safety contract is simply: **never make a file's syntax worse.** An edit
//! is refused if the file drifted since the read it was based on (strict,
//! file-level; ADR-0017), or if it would introduce new parse errors; otherwise
//! the new content is written atomically, preserving the target's permissions
//! and following symlinks to their target. Shared by both edit modes and
//! independent of how the edit was expressed.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;

use lispexp::{parse, ErrorKind, Options, ParseError};
use tempfile::NamedTempFile;

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
    /// The edit would introduce parse errors not present before it (ADR-0005),
    /// compared by lispexp's position-stable [`ErrorKind`], not by count — an
    /// edit that swaps one error for another of the same total count is still
    /// refused. Each string is a newly-introduced error's human-readable form.
    NewParseErrors {
        /// The errors the edit would newly introduce.
        introduced: Vec<String>,
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
/// `expected_file_hash` is the [`file_hash`] the edit
/// was based on; a mismatch is [`WriteError::Drift`] (strict, file-level —
/// ADR-0017; per-anchor relaxed acceptance is a future opt-in). `options`
/// selects the dialect for the parse-error baseline.
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

    let before = parse(&current, options);
    let after = parse(new_content, options);
    let introduced = newly_introduced(&before.errors, &after.errors);
    if !introduced.is_empty() {
        return Err(WriteError::NewParseErrors { introduced });
    }

    write_atomically(path, new_content)?;
    Ok(())
}

/// The parse errors present in `after` beyond those already in `before`,
/// compared as a multiset of position-stable [`ErrorKind`]s so that edits which
/// merely shift the position of a pre-existing error are not flagged.
fn newly_introduced(before: &[ParseError], after: &[ParseError]) -> Vec<String> {
    let mut allowance: HashMap<&ErrorKind, i32> = HashMap::new();
    for err in before {
        *allowance.entry(&err.kind).or_insert(0) += 1;
    }
    let mut introduced = Vec::new();
    for err in after {
        let remaining = allowance.entry(&err.kind).or_insert(0);
        if *remaining > 0 {
            *remaining -= 1;
        } else {
            introduced.push(err.kind.to_string());
        }
    }
    introduced
}

/// Write `content` to `path` atomically: write a sibling temp file, copy the
/// target's permissions onto it, then rename it over the target (atomic within
/// one filesystem). `path` is canonicalized first, so a symlink is written
/// through to its target rather than being replaced by a regular file.
pub fn write_atomically(path: &Path, content: &str) -> std::io::Result<()> {
    let target = std::fs::canonicalize(path)?;
    let dir = target.parent().unwrap_or_else(|| Path::new("."));

    let mut tmp = NamedTempFile::new_in(dir)?;
    tmp.write_all(content.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.as_file()
        .set_permissions(std::fs::metadata(&target)?.permissions())?;
    tmp.persist(&target).map_err(|e| e.error)?;
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
    fn preserves_the_target_file_mode() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let path = write_file(dir.path(), "script.ros", "(f 1)\n");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            let expected = file_hash("(f 1)\n".as_bytes());

            verify_and_write(&path, &expected, "(f 2)\n", &Options::scheme()).unwrap();

            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "exec bit must survive the edit");
        }
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

        assert!(matches!(err, WriteError::NewParseErrors { introduced } if !introduced.is_empty()));
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

    #[test]
    fn allows_an_edit_that_only_shifts_an_existing_error() {
        // A pre-existing unclosed list stays; the edit adds a valid line above it,
        // shifting the error's position but not introducing a new one.
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(dir.path(), "a.scm", "(f 1\n");
        let expected = file_hash("(f 1\n".as_bytes());

        verify_and_write(&path, &expected, "(g 2)\n(f 1\n", &Options::scheme()).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(g 2)\n(f 1\n");
    }
}
