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
pub mod mcp;
pub mod patch;
pub mod resolve;
pub mod search;
pub mod structural;
pub mod write;

use std::path::Path;

use lispexp::annotate::{annotate_tree, bundled_registry, Annotated, Role};
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
    /// A method's Dispatch signature (ADR-0022) — qualifiers, a Clojure
    /// dispatch value, and specializer tokens — for readability. `None` for
    /// non-methods.
    pub signature: Option<String>,
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
                signature: dispatch_signature(source, form),
            }
        })
        .collect()
}

/// A method's Dispatch signature (ADR-0022): its verbatim qualifiers, a Clojure
/// dispatch value, and its specializer tokens. `None` if the form carries none
/// of these (i.e. it is not a method).
fn dispatch_signature(source: &str, form: &Annotated) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for qualifier in form.all(Role::Qualifier) {
        parts.push(span_text(source, qualifier).to_string());
    }
    if let Some(value) = form.first(Role::DispatchValue) {
        parts.push(span_text(source, value).to_string());
    }
    let specialized = form.specialized_params();
    if specialized.iter().any(|p| p.specializer.is_some()) {
        let tokens: Vec<String> = specialized
            .iter()
            .map(|p| match p.specializer {
                Some(s) => span_text(source, s).to_string(),
                None => "_".to_string(),
            })
            .collect();
        parts.push(format!("({})", tokens.join(" ")));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The verbatim source text of `datum`'s span.
fn span_text<'a>(source: &'a str, datum: &Datum) -> &'a str {
    &source[datum.span.start as usize..datum.span.end as usize]
}

/// One node of an [`expand`]: its start line, depth below the definition, an
/// anchor hash, and a one-line preview of its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeEntry {
    /// 1-based start line.
    pub line: u32,
    /// Depth below the expanded definition (0 = the definition itself).
    pub depth: u32,
    /// 4-hex anchor hash over the node's verbatim span (ADR-0008).
    pub hash: String,
    /// A one-line, truncated preview of the node's source.
    pub preview: String,
}

/// Render the Outline of `source` as terse text (`line hash kind name [sig]`,
/// nested names indented) — shared by the CLI and MCP surfaces (ADR-0013).
pub fn outline_text(source: &str, dialect: Dialect) -> String {
    let mut out = String::new();
    for entry in outline(source, dialect) {
        let name = entry.name.as_deref().unwrap_or("-");
        let indent = "  ".repeat(entry.depth as usize);
        let sig = entry
            .signature
            .as_deref()
            .map(|s| format!(" {s}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "{:>5}  {}  {}  {indent}{name}{sig}\n",
            entry.line, entry.hash, entry.kind
        ));
    }
    out
}

/// Render an [`expand`] as terse text (`line hash preview`, nested indented).
pub fn expand_text(source: &str, dialect: Dialect, name: &str) -> String {
    let mut out = String::new();
    for node in expand(source, dialect, name) {
        let indent = "  ".repeat(node.depth as usize);
        out.push_str(&format!("{:>5}  {}  {indent}{}\n", node.line, node.hash, node.preview));
    }
    out
}

/// Expand every definition named `name`, listing its subtree nodes in pre-order
/// with an anchor hash each — so inner nodes become addressable for Structural
/// edits (the expandable Lens, ADR-0013).
pub fn expand(source: &str, dialect: Dialect, name: &str) -> Vec<NodeEntry> {
    let parsed = parse(source, &Options::for_dialect(dialect));
    let registry = bundled_registry(dialect);
    let mut out = Vec::new();
    for form in annotate_tree(&parsed.data, &registry) {
        if form.first(Role::Name).and_then(name_text).as_deref() == Some(name) {
            walk_node(source, form.form, 0, &mut out);
        }
    }
    out
}

fn walk_node(source: &str, datum: &Datum, depth: u32, out: &mut Vec<NodeEntry>) {
    out.push(NodeEntry {
        line: datum.line,
        depth,
        hash: anchor_hash(span_text(source, datum).as_bytes()),
        preview: preview(span_text(source, datum)),
    });
    match &datum.kind {
        DatumKind::List { items, .. } => {
            for item in items {
                walk_node(source, item, depth + 1, out);
            }
        }
        DatumKind::Prefixed { inner, .. } => walk_node(source, inner, depth + 1, out),
        _ => {}
    }
}

/// Collapse whitespace and truncate to a short one-line preview.
fn preview(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let head: String = chars.by_ref().take(60).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
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
    recognized_dialect(path).unwrap_or(Dialect::SchemeSuperset)
}

/// The dialect for a path's extension, or `None` if it is not a recognized Lisp
/// file — used by search to skip non-Lisp files (ADR-0004).
pub fn recognized_dialect(path: &Path) -> Option<Dialect> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    Some(match ext.as_str() {
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
        _ => return None,
    })
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
    fn expand_lists_inner_nodes_with_hashes() {
        let src = "(define (square x) (* x x))\n";
        let nodes = expand(src, Dialect::Scheme, "square");

        assert!(nodes.len() >= 2, "{nodes:?}");
        assert_eq!(nodes[0].depth, 0);
        assert!(nodes[0].preview.contains("define"));
        assert!(nodes.iter().any(|n| n.preview.contains("* x x")));
        assert!(nodes.iter().all(|n| n.hash.len() == 4));
        // an inner node is deeper than the definition root
        assert!(nodes.iter().any(|n| n.depth > 0));
    }

    #[test]
    fn methods_carry_a_dispatch_signature() {
        let src = "(cl-defmethod area ((s square)) 1)\n(cl-defmethod area ((s circle)) 2)\n";
        let entries = outline(src, Dialect::EmacsLisp);

        assert_eq!(entries.len(), 2);
        assert!(entries[0].signature.as_deref().unwrap().contains("square"), "{:?}", entries[0]);
        assert!(entries[1].signature.as_deref().unwrap().contains("circle"), "{:?}", entries[1]);

        let plain = outline("(defun f () 1)\n", Dialect::EmacsLisp);
        assert_eq!(plain[0].signature, None);
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
