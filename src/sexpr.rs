//! Shared structural-comparison primitives over the lispexp parse tree.
//!
//! The home of **Structural equality** (`struct_eq`, ADR-0033): equality of two
//! forms *modulo formatting*. This is the foundation both the semantic
//! refactoring procedures ([`crate::refactor`] — `rewrite`'s literal / non-linear
//! matching, `extract`'s anti-unification) and the Structural diff (ADR-0047 /
//! ADR-0048) build on, so it lives here rather than private to any one consumer.

use lispexp::{Datum, DatumKind};

/// Structural equality **modulo formatting** (ADR-0033): recursive `DatumKind`
/// comparison ignoring `span`/`line` (so whitespace and comments do not matter),
/// with leaf text compared literally and no sugar/number/case normalization.
/// (Distinct from `Datum`'s derived `==`, which compares spans.)
pub fn struct_eq(a: &Datum, b: &Datum) -> bool {
    match (&a.kind, &b.kind) {
        (DatumKind::Symbol(x), DatumKind::Symbol(y)) => x == y,
        (DatumKind::Keyword(x), DatumKind::Keyword(y)) => x == y,
        (DatumKind::Number(x), DatumKind::Number(y)) => x == y,
        (DatumKind::Str(x), DatumKind::Str(y)) => x == y,
        (DatumKind::Char(x), DatumKind::Char(y)) => x == y,
        (DatumKind::Bool(x), DatumKind::Bool(y)) => x == y,
        (DatumKind::LabelRef { id: x }, DatumKind::LabelRef { id: y }) => x == y,
        (
            DatumKind::List {
                delim: da,
                items: ia,
                tail: ta,
                ..
            },
            DatumKind::List {
                delim: db,
                items: ib,
                tail: tb,
                ..
            },
        ) => {
            da == db
                && ia.len() == ib.len()
                && ia.iter().zip(ib).all(|(p, q)| struct_eq(p, q))
                && opt_eq(ta.as_deref(), tb.as_deref())
        }
        (
            DatumKind::Prefixed {
                prefix: pa,
                notation: na,
                inner: inna,
                arg: aa,
            },
            DatumKind::Prefixed {
                prefix: pb,
                notation: nb,
                inner: innb,
                arg: ab,
            },
        ) => pa == pb && na == nb && struct_eq(inna, innb) && opt_eq(aa.as_deref(), ab.as_deref()),
        (
            DatumKind::HashLiteral { tag: ta, inner: ia },
            DatumKind::HashLiteral { tag: tb, inner: ib },
        ) => ta == tb && opt_eq(ia.as_deref(), ib.as_deref()),
        (DatumKind::Label { id: xa, inner: ia }, DatumKind::Label { id: xb, inner: ib }) => {
            xa == xb && struct_eq(ia, ib)
        }
        _ => false,
    }
}

/// [`struct_eq`] lifted over optional nodes (e.g. a list's dotted tail): both
/// absent is equal, both present compares structurally, a presence mismatch is
/// unequal.
pub fn opt_eq(a: Option<&Datum>, b: Option<&Datum>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => struct_eq(x, y),
        _ => false,
    }
}
