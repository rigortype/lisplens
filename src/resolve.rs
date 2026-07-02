//! Resolving a Structural anchor (`line:hash[:ordinal]`) to a node in the parse
//! tree (ADR-0018).
//!
//! A read emits, per node, its start line and a content hash; an edit hands the
//! same `line:hash` back. Resolution walks the tree in pre-order and matches by
//! `(line, hash)`, picking the `ordinal`-th match on the rare same-line
//! collision. The located node carries its parent and index so ops that need
//! sibling or parent context (raise, slurp, …) can be built.

use lispexp::{Datum, DatumKind};

use crate::hash::anchor_hash;
use crate::patch::Anchor;

/// A node found by [`resolve`], with the context ops may need.
pub struct Located<'a, 't> {
    /// The matched node.
    pub node: &'a Datum<'t>,
    /// Its enclosing list, if any (`None` for a top-level node).
    pub parent: Option<&'a Datum<'t>>,
    /// Its index within the parent's items, if any.
    pub index: Option<usize>,
}

/// Resolve `anchor` against the top-level `data`, returning the located node.
pub fn resolve<'a, 't>(
    source: &str,
    data: &'a [Datum<'t>],
    anchor: &Anchor,
) -> Option<Located<'a, 't>> {
    let mut matches = Vec::new();
    collect(source, data, None, anchor, &mut matches);
    let nth = anchor
        .ordinal
        .map(|o| o.saturating_sub(1) as usize)
        .unwrap_or(0);
    matches.into_iter().nth(nth)
}

fn collect<'a, 't>(
    source: &str,
    data: &'a [Datum<'t>],
    parent: Option<&'a Datum<'t>>,
    anchor: &Anchor,
    out: &mut Vec<Located<'a, 't>>,
) {
    for (index, node) in data.iter().enumerate() {
        if node.line == anchor.line {
            let bytes = &source.as_bytes()[node.span.start as usize..node.span.end as usize];
            if anchor_hash(bytes) == anchor.hash {
                out.push(Located {
                    node,
                    parent,
                    index: Some(index),
                });
            }
        }
        if let DatumKind::List { items, .. } = &node.kind {
            collect(source, items, Some(node), anchor, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::anchor_hash;
    use lispexp::{parse, Options};

    fn anchor_for(source: &str, node: &Datum) -> Anchor {
        let bytes = &source.as_bytes()[node.span.start as usize..node.span.end as usize];
        Anchor {
            line: node.line,
            hash: anchor_hash(bytes),
            ordinal: None,
        }
    }

    #[test]
    fn resolves_a_top_level_node_with_no_parent() {
        let src = "(define x 1)\n(define y 2)\n";
        let p = parse(src, &Options::scheme());
        let a = anchor_for(src, &p.data[1]);
        let located = resolve(src, &p.data, &a).unwrap();
        assert!(located.parent.is_none());
        assert_eq!(located.node.line, 2);
    }

    #[test]
    fn resolves_a_nested_node_with_its_parent() {
        let src = "(when cond body)";
        let p = parse(src, &Options::scheme());
        let DatumKind::List { items, .. } = &p.data[0].kind else {
            unreachable!()
        };
        let a = anchor_for(src, &items[2]); // `body`
        let located = resolve(src, &p.data, &a).unwrap();
        assert!(located.parent.is_some());
        assert_eq!(located.index, Some(2));
    }
}
