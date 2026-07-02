//! Structural (paredit/lispy) operations as byte-range edits (ADR-0012).
//!
//! Each operation is a pure function from a node (and, where needed, its parent
//! or the source) to a batch of [`Edit`]s applied via [`crate::edit::splice`].
//! No addressing or CLI syntax is involved here — a caller supplies the target
//! node; resolving a [`Structural address`](../CONTEXT.md) to a node is a
//! separate concern.
//!
//! Implemented so far: `wrap`, `raise`, `splice`. The boundary-moving ops
//! (`slurp` / `barf`) and `split` / `join` are not yet built.

use lispexp::{Datum, DatumKind, Delim};

use crate::edit::Edit;

/// Wrap `node` in a new enclosing list: `x` → `(prefix x)`.
///
/// `prefix` is the leading text placed inside the new parens before the node
/// (e.g. `"when cond"`); empty `prefix` yields a bare `(x)`. Emitted as two
/// insertions, so the node's own bytes are untouched.
pub fn wrap(node: &Datum, prefix: &str) -> Vec<Edit> {
    let start = node.span.start as usize;
    let end = node.span.end as usize;
    let open = if prefix.is_empty() {
        "(".to_string()
    } else {
        format!("({prefix} ")
    };
    vec![
        Edit { range: start..start, text: open },
        Edit { range: end..end, text: ")".to_string() },
    ]
}

/// Raise `child` into its `parent`'s place, discarding the parent's other
/// children: `(when cond x)` → `x`.
///
/// Replaces the parent's whole span with the child's verbatim bytes.
pub fn raise(source: &str, parent: &Datum, child: &Datum) -> Vec<Edit> {
    let inner = &source[child.span.start as usize..child.span.end as usize];
    vec![Edit {
        range: parent.span.start as usize..parent.span.end as usize,
        text: inner.to_string(),
    }]
}

/// Splice a list: remove its delimiters, keeping all contents in the parent —
/// `(foo (bar baz) quux)` on the inner list → `(foo bar baz quux)`.
///
/// Returns `None` if `node` is not a list. Distinct from [`raise`], which drops
/// siblings.
pub fn splice(source: &str, node: &Datum) -> Option<Vec<Edit>> {
    let DatumKind::List { delim, .. } = &node.kind else {
        return None;
    };
    let open_width = match delim {
        Delim::Set => 2, // `#{`
        Delim::Round | Delim::Square | Delim::Curly => 1,
    };
    let start = node.span.start as usize;
    let end = node.span.end as usize;
    let inner = &source[start + open_width..end - 1];
    Some(vec![Edit { range: start..end, text: inner.to_string() }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::splice as apply;
    use lispexp::{parse, DatumKind, Options};

    fn top<'a>(parsed: &'a lispexp::Parsed<'a>) -> &'a Datum<'a> {
        &parsed.data[0]
    }

    #[test]
    fn wrap_encloses_a_node_with_a_prefix() {
        let parsed = parse("x", &Options::scheme());
        let edits = wrap(top(&parsed), "when cond");
        assert_eq!(apply("x", edits).unwrap(), "(when cond x)");
    }

    #[test]
    fn wrap_with_empty_prefix_is_bare_parens() {
        let parsed = parse("x", &Options::scheme());
        let edits = wrap(top(&parsed), "");
        assert_eq!(apply("x", edits).unwrap(), "(x)");
    }

    #[test]
    fn raise_replaces_the_parent_and_drops_siblings() {
        let src = "(when cond x)";
        let parsed = parse(src, &Options::scheme());
        let parent = top(&parsed);
        let DatumKind::List { items, .. } = &parent.kind else {
            unreachable!()
        };
        let child = &items[2]; // `x`
        assert_eq!(apply(src, raise(src, parent, child)).unwrap(), "x");
    }

    #[test]
    fn splice_removes_delimiters_keeping_contents() {
        let src = "(foo (bar baz) quux)";
        let parsed = parse(src, &Options::scheme());
        let DatumKind::List { items, .. } = &top(&parsed).kind else {
            unreachable!()
        };
        let inner = &items[1]; // `(bar baz)`
        let edits = splice(src, inner).unwrap();
        assert_eq!(apply(src, edits).unwrap(), "(foo bar baz quux)");
    }

    #[test]
    fn splice_of_a_non_list_is_none() {
        let parsed = parse("sym", &Options::scheme());
        assert!(splice("sym", top(&parsed)).is_none());
    }
}
