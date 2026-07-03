//! Semantic refactoring procedures (ADR-0032): atomic, parse-safe,
//! self-verifying operations that internalize the multi-step agent idiom
//! (`refs` → `line edit` batch → `refs` re-verify) into one call. First member:
//! whole-file symbol [`rename_symbol_in_file`]. Each is a *composition* over
//! [`crate::structural`] plus the safety pipeline (splice → reindent →
//! validate-then-write), so it adds surface without new edit machinery.

use std::path::Path;

use lispexp::{parse, Options};

use crate::edit::{splice_tracked, SpliceError};
use crate::format::{has_native_engine, reindent, Touched};
use crate::hash::file_hash;
use crate::write::{verify_and_write, WriteError};
use crate::Dialect;

/// The result of a successful [`rename_symbol_in_file`].
#[derive(Debug)]
pub struct RenameOutcome {
    /// How many occurrences were rewritten (the post-condition: exactly this
    /// many, and zero of `from` remain — the rename is exhaustive by
    /// construction).
    pub renamed: usize,
    /// The file hash after the rename, over the reindented content (ADR-0008).
    pub new_file_hash: String,
}

/// Why a rename was refused. No partial write ever happens on an error.
#[derive(Debug)]
pub enum RenameError {
    /// `from` and `to` are the same symbol — nothing to do.
    Unchanged,
    /// The symbol `from` does not occur in the file, so a typo'd name is
    /// reported rather than silently succeeding as a no-op.
    NoOccurrences(String),
    /// The edits could not be spliced.
    Splice(SpliceError),
    /// The safe write was refused (would introduce parse errors, or I/O).
    Write(WriteError),
    /// A filesystem error reading the file.
    Io(std::io::Error),
}

impl std::fmt::Display for RenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameError::Unchanged => write!(f, "`from` and `to` are the same symbol"),
            RenameError::NoOccurrences(s) => write!(f, "no occurrences of symbol `{s}`"),
            RenameError::Splice(e) => write!(f, "splice failed: {e:?}"),
            RenameError::Write(e) => write!(f, "{e:?}"),
            RenameError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Rename every exact occurrence of the symbol `from` to `to` across `path`
/// (ADR-0032).
///
/// Symbol-exact in code **and** data — never a substring, a keyword (`:from`),
/// or text inside a string or comment (that is [`crate::structural::rename`]'s
/// guarantee), so sibling symbols like `from-bar` are untouched with no
/// hand-built lookahead. The rewritten sites are spliced, the touched top-level
/// forms are reindented for dialects with a faithful native engine
/// ([`has_native_engine`] — a rename changes token width, so alignment under the
/// symbol shifts), and the result is validated (reject new parse errors,
/// ADR-0005) before an atomic write. Returns the site count and new file hash.
pub fn rename_symbol_in_file(
    path: &Path,
    from: &str,
    to: &str,
    dialect: Dialect,
) -> Result<RenameOutcome, RenameError> {
    if from == to {
        return Err(RenameError::Unchanged);
    }
    let source = std::fs::read_to_string(path).map_err(RenameError::Io)?;
    let options = Options::for_dialect(dialect);
    let parsed = parse(&source, &options);

    let mut edits = Vec::new();
    for datum in &parsed.data {
        edits.extend(crate::structural::rename(datum, from, to));
    }
    let renamed = edits.len();
    if renamed == 0 {
        return Err(RenameError::NoOccurrences(from.to_string()));
    }

    let expected = file_hash(source.as_bytes());
    let (spliced, spans) = splice_tracked(&source, edits).map_err(RenameError::Splice)?;

    // Reindent the touched top-level forms (native-engine dialects only); other
    // dialects stay verbatim (ADR-0027).
    let new_content = if has_native_engine(dialect) {
        let config = crate::config::resolve(path, &spliced);
        reindent(
            &spliced,
            &config,
            dialect,
            None,
            Touched {
                expand: &spans,
                exact: &[],
            },
        )
    } else {
        spliced
    };

    verify_and_write(path, &expected, &new_content, &options).map_err(RenameError::Write)?;
    Ok(RenameOutcome {
        renamed,
        new_file_hash: file_hash(new_content.as_bytes()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(name: &str, content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn renames_symbol_exactly_leaving_siblings_and_text() {
        // `foo` occurs twice as a symbol (the defun name and the call); the
        // sibling `foo-bar`, the string "foo", and the `; foo` comment must all
        // survive.
        let (_d, path) = write_temp(
            "a.el",
            "(defun foo (x)          ; foo comment\n  (foo-bar (foo x) \"foo\"))\n",
        );
        let out = rename_symbol_in_file(&path, "foo", "qux", Dialect::EmacsLisp).unwrap();
        assert_eq!(out.renamed, 2);
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("(defun qux (x)"));
        assert!(result.contains("(qux x)"));
        assert!(result.contains("foo-bar")); // sibling untouched
        assert!(result.contains("\"foo\"")); // string untouched
        assert!(result.contains("; foo comment")); // comment untouched
        assert!(!result.contains("(foo ")); // no stray old symbol
    }

    #[test]
    fn renames_across_dialects_including_quoted_data() {
        // Scheme: rename `x` — the definition, the body use, and the quoted datum
        // `'x` (data) all move; `xs` (a sibling) does not.
        let (_d, path) = write_temp("a.scm", "(define (f x xs)\n  (cons 'x (g x xs)))\n");
        let out = rename_symbol_in_file(&path, "x", "y", Dialect::Scheme).unwrap();
        assert_eq!(out.renamed, 3);
        let result = std::fs::read_to_string(&path).unwrap();
        assert_eq!(result, "(define (f y xs)\n  (cons 'y (g y xs)))\n");
    }

    #[test]
    fn missing_symbol_is_reported_not_a_silent_noop() {
        let (_d, path) = write_temp("a.el", "(defun foo () 1)\n");
        let err = rename_symbol_in_file(&path, "nope", "x", Dialect::EmacsLisp).unwrap_err();
        assert!(matches!(err, RenameError::NoOccurrences(_)));
        // file untouched
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(defun foo () 1)\n");
    }

    #[test]
    fn same_from_and_to_is_refused() {
        let (_d, path) = write_temp("a.el", "(defun foo () 1)\n");
        assert!(matches!(
            rename_symbol_in_file(&path, "foo", "foo", Dialect::EmacsLisp),
            Err(RenameError::Unchanged)
        ));
    }
}
