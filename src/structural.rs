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
        Edit {
            range: start..start,
            text: open,
        },
        Edit {
            range: end..end,
            text: ")".to_string(),
        },
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
    Some(vec![Edit {
        range: start..end,
        text: inner.to_string(),
    }])
}

/// Forward-slurp: extend `list` rightward to swallow its `next_sibling` —
/// `(foo (bar baz) quux zot)` → `(foo (bar baz quux) zot)`. Moves the list's
/// closing delimiter to just after `next_sibling`.
pub fn slurp_forward(list: &Datum, next_sibling: &Datum) -> Option<Vec<Edit>> {
    let (delim, _) = as_list(list)?;
    let close = list.span.end as usize;
    let after = next_sibling.span.end as usize;
    Some(vec![
        Edit {
            range: close - 1..close,
            text: String::new(),
        },
        Edit {
            range: after..after,
            text: close_glyph(delim).to_string(),
        },
    ])
}

/// Backward-slurp: extend `list` leftward to swallow its `prev_sibling` —
/// `(foo bar (baz quux) zot)` → `(foo (bar baz quux) zot)`. Moves the list's
/// opening delimiter to just before `prev_sibling`.
pub fn slurp_backward(list: &Datum, prev_sibling: &Datum) -> Option<Vec<Edit>> {
    let (delim, _) = as_list(list)?;
    let open = list.span.start as usize;
    let before = prev_sibling.span.start as usize;
    Some(vec![
        Edit {
            range: open..open + open_width(delim),
            text: String::new(),
        },
        Edit {
            range: before..before,
            text: open_glyph(delim).to_string(),
        },
    ])
}

/// Forward-barf: expel `list`'s last element out to the right —
/// `(foo (bar baz quux) zot)` → `(foo (bar baz) quux zot)`. `None` if the list
/// is empty.
pub fn barf_forward(list: &Datum) -> Option<Vec<Edit>> {
    let (delim, items) = as_list(list)?;
    if items.is_empty() {
        return None;
    }
    let close = list.span.end as usize;
    let insert_at = if items.len() >= 2 {
        items[items.len() - 2].span.end as usize
    } else {
        list.span.start as usize + open_width(delim)
    };
    Some(vec![
        Edit {
            range: insert_at..insert_at,
            text: close_glyph(delim).to_string(),
        },
        Edit {
            range: close - 1..close,
            text: String::new(),
        },
    ])
}

/// Backward-barf: expel `list`'s first element out to the left —
/// `(foo (bar baz quux) zot)` → `(foo bar (baz quux) zot)`. `None` if the list
/// is empty.
pub fn barf_backward(list: &Datum) -> Option<Vec<Edit>> {
    let (delim, items) = as_list(list)?;
    if items.is_empty() {
        return None;
    }
    let open = list.span.start as usize;
    let insert_at = if items.len() >= 2 {
        items[1].span.start as usize
    } else {
        list.span.end as usize - 1
    };
    Some(vec![
        Edit {
            range: open..open + open_width(delim),
            text: String::new(),
        },
        Edit {
            range: insert_at..insert_at,
            text: open_glyph(delim).to_string(),
        },
    ])
}

/// Split `list` into two after child `after_index` — `(a b c)` split after 0 →
/// `(a) (b c)`. `None` if there is no child after `after_index`.
pub fn split(list: &Datum, after_index: usize) -> Option<Vec<Edit>> {
    let (delim, items) = as_list(list)?;
    if after_index + 1 >= items.len() {
        return None;
    }
    let left_end = items[after_index].span.end as usize;
    let right_start = items[after_index + 1].span.start as usize;
    Some(vec![
        Edit {
            range: left_end..left_end,
            text: close_glyph(delim).to_string(),
        },
        Edit {
            range: right_start..right_start,
            text: open_glyph(delim).to_string(),
        },
    ])
}

/// Join two adjacent sibling lists into one — `(a) (b)` → `(a b)`. Removes
/// `first`'s closing delimiter and `second`'s opening delimiter. `None` if
/// either node is not a list.
pub fn join(first: &Datum, second: &Datum) -> Option<Vec<Edit>> {
    as_list(first)?;
    let (sdelim, _) = as_list(second)?;
    let fclose = first.span.end as usize;
    let sopen = second.span.start as usize;
    Some(vec![
        Edit {
            range: fclose - 1..fclose,
            text: String::new(),
        },
        Edit {
            range: sopen..sopen + open_width(sdelim),
            text: String::new(),
        },
    ])
}

/// Rename every occurrence of the symbol `from` to `to` within `node`'s subtree
/// — occurrence-based, **not** scope-aware (ADR-0003). Renames in both code and
/// quoted data within the subtree; it does not resolve bindings or shadowing.
pub fn rename(node: &Datum, from: &str, to: &str) -> Vec<Edit> {
    let mut edits = Vec::new();
    rename_walk(node, from, to, &mut edits);
    edits
}

fn rename_walk(datum: &Datum, from: &str, to: &str, edits: &mut Vec<Edit>) {
    match &datum.kind {
        DatumKind::Symbol(s) if *s == from => edits.push(Edit {
            range: datum.span.start as usize..datum.span.end as usize,
            text: to.to_string(),
        }),
        DatumKind::List { items, tail, .. } => {
            for item in items {
                rename_walk(item, from, to, edits);
            }
            if let Some(tail) = tail {
                rename_walk(tail, from, to, edits);
            }
        }
        DatumKind::Prefixed { inner, arg, .. } => {
            rename_walk(inner, from, to, edits);
            if let Some(arg) = arg {
                rename_walk(arg, from, to, edits);
            }
        }
        _ => {}
    }
}

/// The list's delimiter and its children, or `None` for a non-list.
fn as_list<'a, 't>(node: &'a Datum<'t>) -> Option<(&'a Delim, &'a [Datum<'t>])> {
    match &node.kind {
        DatumKind::List { delim, items, .. } => Some((delim, items)),
        _ => None,
    }
}

fn open_glyph(delim: &Delim) -> &'static str {
    match delim {
        Delim::Round => "(",
        Delim::Square => "[",
        Delim::Curly => "{",
        Delim::Set => "#{",
    }
}

fn close_glyph(delim: &Delim) -> &'static str {
    match delim {
        Delim::Round => ")",
        Delim::Square => "]",
        Delim::Curly | Delim::Set => "}",
    }
}

fn open_width(delim: &Delim) -> usize {
    match delim {
        Delim::Set => 2,
        Delim::Round | Delim::Square | Delim::Curly => 1,
    }
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

    fn items<'a, 't>(node: &'a Datum<'t>) -> &'a [Datum<'t>] {
        match &node.kind {
            DatumKind::List { items, .. } => items,
            _ => panic!("not a list"),
        }
    }

    #[test]
    fn slurp_forward_swallows_the_next_sibling() {
        let src = "(foo (bar baz) quux zot)";
        let p = parse(src, &Options::scheme());
        let outer = top(&p);
        let edits = slurp_forward(&items(outer)[1], &items(outer)[2]).unwrap();
        assert_eq!(apply(src, edits).unwrap(), "(foo (bar baz quux) zot)");
    }

    #[test]
    fn slurp_backward_swallows_the_previous_sibling() {
        let src = "(foo bar (baz quux) zot)";
        let p = parse(src, &Options::scheme());
        let outer = top(&p);
        let edits = slurp_backward(&items(outer)[2], &items(outer)[1]).unwrap();
        assert_eq!(apply(src, edits).unwrap(), "(foo (bar baz quux) zot)");
    }

    #[test]
    fn barf_forward_expels_the_last_element() {
        let src = "(foo (bar baz quux) zot)";
        let p = parse(src, &Options::scheme());
        let edits = barf_forward(&items(top(&p))[1]).unwrap();
        assert_eq!(apply(src, edits).unwrap(), "(foo (bar baz) quux zot)");
    }

    #[test]
    fn barf_backward_expels_the_first_element() {
        let src = "(foo (bar baz quux) zot)";
        let p = parse(src, &Options::scheme());
        let edits = barf_backward(&items(top(&p))[1]).unwrap();
        assert_eq!(apply(src, edits).unwrap(), "(foo bar (baz quux) zot)");
    }

    #[test]
    fn split_divides_a_list_after_a_child() {
        let src = "(hello world)";
        let p = parse(src, &Options::scheme());
        assert_eq!(
            apply(src, split(top(&p), 0).unwrap()).unwrap(),
            "(hello) (world)"
        );
    }

    #[test]
    fn rename_replaces_occurrences_in_the_subtree() {
        let src = "(let ((x 1)) (+ x x))";
        let p = parse(src, &Options::scheme());
        let edits = rename(top(&p), "x", "y");
        assert_eq!(apply(src, edits).unwrap(), "(let ((y 1)) (+ y y))");
    }

    #[test]
    fn join_merges_two_adjacent_lists() {
        let src = "(hello) (world)";
        let p = parse(src, &Options::scheme());
        let edits = join(&p.data[0], &p.data[1]).unwrap();
        assert_eq!(apply(src, edits).unwrap(), "(hello world)");
    }
}
