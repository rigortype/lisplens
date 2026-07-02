//! Project search (ADR-0010): a read-only, project-wide lookup that returns the
//! *locations* of definitions, so an agent can find a target across files
//! before editing it file by file. Decoupled from the edit machinery.
//!
//! This first cut finds definitions by name (aggregating per-file Outlines).
//! Symbol-occurrence search (code-vs-data aware) can follow.

use std::path::{Path, PathBuf};

use lispexp::{Class, DatumKind, Options, Walk};

use crate::hash::anchor_hash;
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

/// One symbol occurrence found by [`find_symbol`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// The file the symbol occurs in.
    pub file: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// The occurrence's anchor hash — directly usable as an edit anchor.
    pub hash: String,
    /// Whether the occurrence sits in code (`true`) or in quoted data
    /// (`false`), per lispexp's code-vs-data walk (ADR-0010).
    pub in_code: bool,
}

/// Find every occurrence of the symbol `symbol` under `root`, across recognized
/// Lisp files, tagging each as code or data (ADR-0010). Descends into both so
/// nothing is missed; the `in_code` flag surfaces the classification.
pub fn find_symbol(root: &Path, symbol: &str) -> std::io::Result<Vec<Occurrence>> {
    let mut occurrences = Vec::new();
    walk_files(root, &mut |path| {
        let Some(dialect) = recognized_dialect(path) else {
            return;
        };
        let Ok(source) = std::fs::read_to_string(path) else {
            return;
        };
        let parsed = lispexp::parse(&source, &Options::for_dialect(dialect));
        let mut local = Vec::new();
        lispexp::walk(&parsed.data, |datum, class| {
            if let DatumKind::Symbol(s) = &datum.kind {
                if *s == symbol {
                    let bytes =
                        &source.as_bytes()[datum.span.start as usize..datum.span.end as usize];
                    local.push(Occurrence {
                        file: path.to_path_buf(),
                        line: datum.line,
                        hash: anchor_hash(bytes),
                        in_code: class == Class::Code,
                    });
                }
            }
            Walk::Descend
        });
        occurrences.extend(local);
    })?;
    Ok(occurrences)
}

/// Render definition hits as `file:line:hash kind name` lines.
pub fn hits_text(hits: &[Hit]) -> String {
    hits.iter()
        .map(|h| format!("{}:{}:{} {} {}\n", h.file.display(), h.line, h.hash, h.kind, h.name))
        .collect()
}

/// Render symbol occurrences as `file:line:hash code|data name` lines.
pub fn occurrences_text(occurrences: &[Occurrence], name: &str) -> String {
    occurrences
        .iter()
        .map(|o| {
            let class = if o.in_code { "code" } else { "data" };
            format!("{}:{}:{} {class} {name}\n", o.file.display(), o.line, o.hash)
        })
        .collect()
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
    fn find_symbol_tags_code_vs_data_occurrences() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.scm"), "(foo bar)\n'(foo baz)\n(list foo)\n").unwrap();

        let occ = find_symbol(dir.path(), "foo").unwrap();
        assert_eq!(occ.len(), 3, "{occ:?}");
        assert_eq!(occ.iter().filter(|o| o.in_code).count(), 2);
        assert_eq!(occ.iter().filter(|o| !o.in_code).count(), 1);
    }

    #[test]
    fn skips_hidden_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/x.el"), "(defun target () 1)\n").unwrap();

        assert!(find_definitions(dir.path(), "target").unwrap().is_empty());
    }
}
