//! lisplens — token-efficient, polyglot Lisp editing for AI agents.
//!
//! Skeleton stage: this crate currently exposes a minimal [`outline`] over the
//! [`lispexp`] reader. The full design — Structural / Line-hash modes, Batch
//! edits, drift detection, the pluggable formatter — lives in `CONTEXT.md` and
//! `docs/adr/`.

use std::path::Path;

use lispexp::{parse, Datum, DatumKind, Options};

/// One entry in a file's [`outline`]: a top-level definition's start line, its
/// defining head (e.g. `defun`, `define`), and the name it introduces.
///
/// A first, heuristic slice of the Outline from ADR-0013 — it keys off the head
/// symbol and does not yet use lispexp's `annotate` form roles or carry a hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// 1-based start line of the definition.
    pub line: u32,
    /// The defining head symbol, e.g. `define` or `defun`.
    pub kind: String,
    /// The defined name, when one can be extracted.
    pub name: Option<String>,
}

/// Parse `source` and return a heuristic Outline of its top-level definitions.
pub fn outline(source: &str, options: &Options) -> Vec<OutlineEntry> {
    parse(source, options)
        .data
        .iter()
        .filter_map(outline_entry)
        .collect()
}

fn outline_entry(datum: &Datum) -> Option<OutlineEntry> {
    let DatumKind::List { items, .. } = &datum.kind else {
        return None;
    };
    let head = symbol_text(items.first()?)?;
    // Heuristic first pass: treat `def*` / `define*` heads as definitions.
    if !head.starts_with("def") {
        return None;
    }
    Some(OutlineEntry {
        line: datum.line,
        kind: head.to_string(),
        name: items.get(1).and_then(defined_name),
    })
}

/// The name a definition introduces: `(defvar x …)` → `x`;
/// `(define (square n) …)` → `square`.
fn defined_name(datum: &Datum) -> Option<String> {
    match &datum.kind {
        DatumKind::Symbol(s) => Some((*s).to_string()),
        DatumKind::List { items, .. } => symbol_text(items.first()?).map(str::to_string),
        _ => None,
    }
}

fn symbol_text<'a>(datum: &Datum<'a>) -> Option<&'a str> {
    match &datum.kind {
        DatumKind::Symbol(s) => Some(*s),
        _ => None,
    }
}

/// Zero-config dialect guess from a file extension (ADR-0004, first pass).
/// Unknown extensions fall back to a permissive Scheme superset.
pub fn options_for_path(path: &Path) -> Options {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "scm" | "ss" | "sls" | "sps" | "sld" => Options::scheme(),
        "el" => Options::emacs_lisp(),
        "clj" | "cljs" | "cljc" | "edn" => Options::clojure(),
        "lisp" | "lsp" | "cl" | "asd" => Options::common_lisp(),
        "rkt" => Options::racket(),
        "fnl" => Options::fennel(),
        "janet" => Options::janet(),
        "hy" => Options::hy(),
        "lfe" => Options::lfe(),
        "phel" => Options::phel(),
        _ => Options::scheme_superset(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_scheme_and_elisp_definitions() {
        let src = "(define (square x) (* x x))\n(defvar answer 42)\n(+ 1 2)\n";
        let entries = outline(src, &Options::scheme());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, "define");
        assert_eq!(entries[0].name.as_deref(), Some("square"));
        assert_eq!(entries[1].kind, "defvar");
        assert_eq!(entries[1].name.as_deref(), Some("answer"));
    }
}
