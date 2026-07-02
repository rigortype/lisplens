//! lisplens — token-efficient, polyglot Lisp editing for AI agents.
//!
//! Current surface: a structural [`outline`] and a Line-hash [`linehash::read`]
//! over the [`lispexp`] reader, plus the safe-write machinery in [`write`]. The
//! full design — Structural / Line-hash modes, Batch edits, drift detection,
//! the pluggable formatter — lives in `CONTEXT.md` and `docs/adr/`.

pub mod apply;
pub mod edit;
pub mod hash;
pub mod linehash;
pub mod patch;
pub mod structural;
pub mod write;

use std::path::Path;

use lispexp::annotate::{annotate_tree, bundled_registry, Role};
use lispexp::{parse, Datum, DatumKind, Dialect, Options};

use crate::hash::anchor_hash;

/// One entry in a file's [`outline`]: a definition's start line, a short
/// content hash of its whole form (its anchor — ADR-0008), its defining head
/// (the verbatim *kind*, e.g. `defun`), and the name it introduces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// 1-based start line of the definition.
    pub line: u32,
    /// Nesting depth: 0 for a top-level definition, +1 per enclosing
    /// definition (e.g. a Scheme internal `define`). Shown by indentation.
    pub depth: u32,
    /// 4-hex anchor hash over the form's verbatim span bytes.
    pub hash: String,
    /// The defining head symbol, verbatim (e.g. `define`, `cl-defun`).
    pub kind: String,
    /// The defined name, when one can be extracted.
    pub name: Option<String>,
}

/// Produce a structural Outline (ADR-0013) of `source`'s definitions for
/// `dialect`, using lispexp's polyglot definition registry — no bespoke
/// heuristic. Definitions are listed in source order (outer before nested);
/// nesting is not yet shown by indentation. Each entry carries a 4-hex anchor
/// hash (ADR-0008) over the form's verbatim span.
pub fn outline(source: &str, dialect: Dialect) -> Vec<OutlineEntry> {
    let parsed = parse(source, &Options::for_dialect(dialect));
    let registry = bundled_registry(dialect);
    // annotate_tree yields forms in pre-order (outer before the inner forms it
    // contains), so a stack of enclosing-definition end offsets gives each
    // form's nesting depth by span containment.
    let mut enclosing: Vec<u32> = Vec::new();
    annotate_tree(&parsed.data, &registry)
        .iter()
        .map(|form| {
            let (start, end) = (form.form.span.start, form.form.span.end);
            while enclosing.last().is_some_and(|&e| e <= start) {
                enclosing.pop();
            }
            let depth = enclosing.len() as u32;
            enclosing.push(end);
            OutlineEntry {
                line: form.form.line,
                depth,
                hash: anchor_hash(span_bytes(source, form.form)),
                kind: form.head.to_string(),
                name: form.first(Role::Name).and_then(name_text),
            }
        })
        .collect()
}

/// The verbatim source bytes of `datum`'s span.
fn span_bytes<'a>(source: &'a str, datum: &Datum) -> &'a [u8] {
    &source.as_bytes()[datum.span.start as usize..datum.span.end as usize]
}

/// A definition's name as text: a bare name symbol (`(define pi …)` → `pi`), or
/// the head of a `(name args…)` definition target (`(define (square x) …)` →
/// `square`).
fn name_text(datum: &Datum) -> Option<String> {
    match &datum.kind {
        DatumKind::Symbol(s) => Some((*s).to_string()),
        DatumKind::List { items, .. } => match &items.first()?.kind {
            DatumKind::Symbol(s) => Some((*s).to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// The lispexp [`Options`] for a path's guessed dialect ([`dialect_for_path`]).
pub fn options_for_path(path: &Path) -> Options {
    Options::for_dialect(dialect_for_path(path))
}

/// Zero-config dialect guess from a file extension (ADR-0004, first pass).
/// Unknown extensions fall back to a permissive Scheme superset.
pub fn dialect_for_path(path: &Path) -> Dialect {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "scm" | "ss" | "sls" | "sps" | "sld" => Dialect::Scheme,
        "el" => Dialect::EmacsLisp,
        "clj" | "cljs" | "cljc" => Dialect::Clojure,
        "edn" => Dialect::Edn,
        "lisp" | "lsp" | "cl" | "asd" => Dialect::CommonLisp,
        "rkt" => Dialect::Racket,
        "fnl" => Dialect::Fennel,
        "janet" => Dialect::Janet,
        "hy" => Dialect::Hy,
        "lfe" => Dialect::Lfe,
        "phel" => Dialect::Phel,
        _ => Dialect::SchemeSuperset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_definitions_with_kind_name_and_hash() {
        let src = "(define (square x) (* x x))\n(define pi 3.14)\n(+ 1 2)\n";
        let entries = outline(src, Dialect::Scheme);

        assert_eq!(entries.len(), 2, "the bare call (+ 1 2) is not a definition");
        assert_eq!(entries[0].kind, "define");
        assert_eq!(entries[0].name.as_deref(), Some("square"));
        assert_eq!(entries[0].line, 1);
        assert_eq!(entries[0].depth, 0);
        assert_eq!(entries[0].hash.len(), 4);
        assert_eq!(entries[1].name.as_deref(), Some("pi"));
    }

    #[test]
    fn nested_definitions_get_a_deeper_depth() {
        let src = "(define (outer)\n  (define inner 1)\n  inner)\n";
        let entries = outline(src, Dialect::Scheme);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name.as_deref(), Some("outer"));
        assert_eq!(entries[0].depth, 0);
        assert_eq!(entries[1].name.as_deref(), Some("inner"));
        assert_eq!(entries[1].depth, 1);
    }

    #[test]
    fn recognizes_prefixed_emacs_definitions_the_old_heuristic_missed() {
        // `cl-defun` / `ert-deftest` don't start with "def" — the previous
        // starts_with("def") heuristic dropped them; the registry catches them.
        let src = "(cl-defun foo () 1)\n(ert-deftest bar () (should t))\n";
        let kinds: Vec<String> = outline(src, Dialect::EmacsLisp)
            .into_iter()
            .map(|e| e.kind)
            .collect();

        assert!(kinds.iter().any(|k| k == "cl-defun"), "{kinds:?}");
        assert!(kinds.iter().any(|k| k == "ert-deftest"), "{kinds:?}");
    }

    #[test]
    fn does_not_mistake_an_ordinary_call_for_a_definition() {
        // `(defrobulate …)` looks defn-ish to a prefix heuristic but is not a
        // known definition form — the registry does not match it.
        let entries = outline("(defrobulate x)\n", Dialect::Scheme);
        assert!(entries.is_empty(), "{entries:?}");
    }
}
