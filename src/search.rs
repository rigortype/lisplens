//! Project search (ADR-0010): a read-only, project-wide lookup that returns the
//! *locations* of definitions, so an agent can find a target across files
//! before editing it file by file. Decoupled from the edit machinery.
//!
//! This first cut finds definitions by name (aggregating per-file Outlines).
//! Symbol-occurrence search (code-vs-data aware) can follow.

use std::path::{Path, PathBuf};

use crate::{outline, recognized_dialect};

/// A definition found by [`find_definitions`], with the anchor an edit needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The file the definition is in.
    pub file: PathBuf,
    /// 1-based start line.
    pub line: u32,
    /// The definition's anchor hash (ADR-0008).
    pub hash: String,
    /// The defining head (kind), e.g. `defun`.
    pub kind: String,
    /// The defined name.
    pub name: String,
}

/// Find every definition named `name` under `root`, across recognized Lisp
/// files. Non-Lisp files and hidden directories (`.git`, `.qlot`, …) are
/// skipped. Unreadable files are ignored so one bad file cannot fail a search.
pub fn find_definitions(root: &Path, name: &str) -> std::io::Result<Vec<Hit>> {
    let mut hits = Vec::new();
    walk_files(root, &mut |path| {
        let Some(dialect) = recognized_dialect(path) else {
            return;
        };
        let Ok(source) = std::fs::read_to_string(path) else {
            return;
        };
        for entry in outline(&source, dialect) {
            if entry.name.as_deref() == Some(name) {
                hits.push(Hit {
                    file: path.to_path_buf(),
                    line: entry.line,
                    hash: entry.hash,
                    kind: entry.kind,
                    name: name.to_string(),
                });
            }
        }
    })?;
    Ok(hits)
}

/// Recurse `dir`, calling `visit` for each regular file. `DirEntry::file_type`
/// does not follow symlinks, so symlinked directories are not descended (no
/// cycles). Hidden directories are skipped.
fn walk_files(dir: &Path, visit: &mut impl FnMut(&Path)) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            let hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with('.'))
                .unwrap_or(false);
            if !hidden {
                walk_files(&path, visit)?;
            }
        } else if file_type.is_file() {
            visit(&path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_definition_across_files_and_skips_non_lisp() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.el"), "(defun target () 1)\n(defun other () 2)\n").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b.scm"), "(define target 3)\n").unwrap();
        std::fs::write(dir.path().join("readme.txt"), "(defun target () ignored)\n").unwrap();

        let mut hits = find_definitions(dir.path(), "target").unwrap();
        hits.sort_by_key(|h| h.file.clone());

        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().all(|h| h.name == "target"));
        assert!(hits.iter().any(|h| h.kind == "defun"));
        assert!(hits.iter().any(|h| h.kind == "define"));
    }

    #[test]
    fn skips_hidden_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/x.el"), "(defun target () 1)\n").unwrap();

        assert!(find_definitions(dir.path(), "target").unwrap().is_empty());
    }
}
