//! Structural diff (ADR-0047) — a read-only observation of how two versions of a
//! file differ, at top-level **definition** granularity: the "attention map" an
//! agent reads first to see *which* units changed before drilling into *how*
//! (the ADR-0048 tree diff, a later slice).
//!
//! A **unit** is a top-level definition (a named `outline` entry at depth 0).
//! Units are matched across the two versions by `(kind, name, dispatch?)` — plain
//! definitions carry no Dispatch signature, method-like forms add theirs so two
//! methods of one generic stay distinct. A matched pair is *changed* iff it is
//! not [`struct_eq`](crate::sexpr::struct_eq) (formatting-modulo), so reindent- or
//! comment-only churn never registers. No rename detection: a rename surfaces as
//! one removed + one added unit. Top-level non-definition forms (`require`,
//! `provide`, …) have no stable name key and are not diffed individually — only a
//! single "other top-level forms changed" summary flag reports them.

use std::collections::HashMap;

use lispexp::annotate::{annotate_tree, bundled_registry, Role};
use lispexp::{parse, Datum, Options};

use crate::hash::anchor_hash;
use crate::sexpr::struct_eq;
use crate::{dispatch_signature, name_text, span_bytes, Dialect};

/// How a definition unit differs between the two versions. `Unchanged` units are
/// not emitted (see [`FileDiff::units`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitStatus {
    /// Present only in the new version.
    Added,
    /// Present only in the old version.
    Removed,
    /// Present in both, but not [`struct_eq`](crate::sexpr::struct_eq).
    Changed,
}

impl UnitStatus {
    /// The one-char marker used in the text rendering (`+` / `-` / `~`).
    fn marker(self) -> char {
        match self {
            UnitStatus::Added => '+',
            UnitStatus::Removed => '-',
            UnitStatus::Changed => '~',
        }
    }

    /// The lowercase word used for the JSON key / section header.
    fn word(self) -> &'static str {
        match self {
            UnitStatus::Added => "added",
            UnitStatus::Removed => "removed",
            UnitStatus::Changed => "changed",
        }
    }
}

/// One changed/added/removed definition unit. The anchor (`line` + `hash`) is the
/// new version's for `Added`/`Changed` and the old version's for `Removed`, so an
/// agent can go straight from a change to editing the right file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitDiff {
    /// The defining head symbol, verbatim (e.g. `defun`, `cl-defmethod`).
    pub kind: String,
    /// The defined name.
    pub name: String,
    /// A method's Dispatch signature (ADR-0022), or `None` for a plain definition.
    pub signature: Option<String>,
    /// How the unit differs.
    pub status: UnitStatus,
    /// 1-based start line of the anchor side (new for Added/Changed, old for Removed).
    pub line: u32,
    /// 4-hex anchor hash of the anchor side's form span (ADR-0008).
    pub hash: String,
}

/// The result of [`diff_files`]: the changed/added/removed units (unchanged
/// omitted) plus whether the top-level non-definition forms changed at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// Changed/added/removed units. Unchanged units are not included.
    pub units: Vec<UnitDiff>,
    /// Whether the set of top-level *non-definition* forms differs between the
    /// versions (a single summary flag; these forms are not diffed individually).
    pub other_forms_changed: bool,
}

impl FileDiff {
    /// Whether there is nothing to report (the versions are identical modulo
    /// formatting).
    pub fn is_empty(&self) -> bool {
        self.units.is_empty() && !self.other_forms_changed
    }
}

/// The matching key for a unit: `(kind, name, dispatch-signature?)`.
type Key = (String, String, Option<String>);

/// A top-level named definition, with its parsed form kept for `struct_eq`.
struct Unit<'a> {
    key: Key,
    line: u32,
    hash: String,
    form: &'a Datum<'a>,
}

/// Split a parsed top level into its named definition units and its
/// non-definition forms. A top-level `Datum` is a unit iff it is a definition the
/// registry recognizes (matched by span start) *and* it introduces a name.
fn collect_units<'a>(
    source: &'a str,
    data: &'a [Datum<'a>],
    dialect: Dialect,
) -> (Vec<Unit<'a>>, Vec<&'a Datum<'a>>) {
    let registry = bundled_registry(dialect);
    let annotated = annotate_tree(data, &registry);
    // Map each annotated definition to its span start. Top-level forms are matched
    // by span start below; nested definitions never share a top-level start, so
    // this yields depth-0 units without an explicit depth walk.
    let mut by_start: HashMap<u32, usize> = HashMap::new();
    for (i, form) in annotated.iter().enumerate() {
        by_start.entry(form.form.span.start).or_insert(i);
    }

    let mut units = Vec::new();
    let mut others = Vec::new();
    for datum in data {
        let unit = by_start.get(&datum.span.start).and_then(|&i| {
            let form = &annotated[i];
            let name = form.first(Role::Name).and_then(name_text)?;
            Some(Unit {
                key: (
                    form.head.to_string(),
                    name,
                    dispatch_signature(source, form),
                ),
                line: datum.line,
                hash: anchor_hash(span_bytes(source, datum)),
                form: datum,
            })
        });
        match unit {
            Some(u) => units.push(u),
            None => others.push(datum),
        }
    }
    (units, others)
}

/// Group units by their matching key, preserving source order within each key.
fn bucket_by_key<'u, 'a>(units: &'u [Unit<'a>]) -> HashMap<&'u Key, Vec<&'u Unit<'a>>> {
    let mut map: HashMap<&Key, Vec<&Unit>> = HashMap::new();
    for u in units {
        map.entry(&u.key).or_default().push(u);
    }
    map
}

/// Compare two versions of a source at definition granularity (ADR-0047).
pub fn diff_files(old: &str, new: &str, dialect: Dialect) -> FileDiff {
    let old_parsed = parse(old, &Options::for_dialect(dialect));
    let new_parsed = parse(new, &Options::for_dialect(dialect));
    let (old_units, old_others) = collect_units(old, &old_parsed.data, dialect);
    let (new_units, new_others) = collect_units(new, &new_parsed.data, dialect);

    // Bucket units by key, preserving source order. A key can repeat within one
    // file — e.g. Emacs's `(defvar x)` forward declaration plus its later
    // `(defvar x nil)` — so each key maps to a *list*; matching a single instance
    // per key would mispair the duplicates and falsely report a change.
    let old_by_key = bucket_by_key(&old_units);
    let new_by_key = bucket_by_key(&new_units);
    let mut keys: Vec<&Key> = old_by_key
        .keys()
        .chain(new_by_key.keys())
        .copied()
        .collect();
    keys.sort_unstable();
    keys.dedup();

    let mut units = Vec::new();
    let empty: Vec<&Unit> = Vec::new();
    for key in keys {
        let olds = old_by_key.get(key).unwrap_or(&empty);
        let news = new_by_key.get(key).unwrap_or(&empty);
        // Consume exact (struct_eq) pairs first — these are unchanged, so an
        // untouched duplicate stays untouched regardless of order.
        let mut new_used = vec![false; news.len()];
        let mut old_unmatched: Vec<&Unit> = Vec::new();
        for ou in olds {
            let mut matched = false;
            for (j, nu) in news.iter().enumerate() {
                if !new_used[j] && struct_eq(ou.form, nu.form) {
                    new_used[j] = true;
                    matched = true;
                    break;
                }
            }
            if !matched {
                old_unmatched.push(ou);
            }
        }
        let new_unmatched: Vec<&Unit> = news
            .iter()
            .enumerate()
            .filter(|(j, _)| !new_used[*j])
            .map(|(_, nu)| *nu)
            .collect();
        // The remainder: pair positionally as Changed, the leftover tail as
        // Removed / Added.
        let common = old_unmatched.len().min(new_unmatched.len());
        for nu in &new_unmatched[..common] {
            units.push(unit_diff(nu, UnitStatus::Changed));
        }
        for ou in &old_unmatched[common..] {
            units.push(unit_diff(ou, UnitStatus::Removed));
        }
        for nu in &new_unmatched[common..] {
            units.push(unit_diff(nu, UnitStatus::Added));
        }
    }

    let other_forms_changed = old_others.len() != new_others.len()
        || old_others
            .iter()
            .zip(&new_others)
            .any(|(a, b)| !struct_eq(a, b));

    FileDiff {
        units,
        other_forms_changed,
    }
}

fn unit_diff(u: &Unit, status: UnitStatus) -> UnitDiff {
    UnitDiff {
        kind: u.key.0.clone(),
        name: u.key.1.clone(),
        signature: u.key.2.clone(),
        status,
        line: u.line,
        hash: u.hash.clone(),
    }
}

/// Render a [`FileDiff`] as terse text, grouped Changed / Added / Removed (empty
/// sections omitted), each line `<marker> <kind> <name>[ <sig>]  <line>:<hash>`.
/// Structurally identical versions render the empty string.
pub fn diff_text(diff: &FileDiff) -> String {
    let mut out = String::new();
    for status in [UnitStatus::Changed, UnitStatus::Added, UnitStatus::Removed] {
        let mut rows: Vec<&UnitDiff> = diff.units.iter().filter(|u| u.status == status).collect();
        if rows.is_empty() {
            continue;
        }
        rows.sort_by_key(|u| u.line);
        out.push_str(&format!("{}:\n", status.word()));
        for u in rows {
            let sig = u
                .signature
                .as_deref()
                .map(|s| format!(" {s}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "  {} {} {}{sig}  {}:{}\n",
                status.marker(),
                u.kind,
                u.name,
                u.line,
                u.hash
            ));
        }
    }
    if diff.other_forms_changed {
        out.push_str("other:\n  ! other top-level forms changed\n");
    }
    out
}

/// Render a [`FileDiff`] as a JSON object: `{changed, added, removed}` arrays of
/// `{kind, name, signature, line, hash}` plus `otherFormsChanged`.
pub fn diff_json(diff: &FileDiff) -> serde_json::Value {
    use serde_json::json;
    let section = |status: UnitStatus| -> serde_json::Value {
        let mut rows: Vec<&UnitDiff> = diff.units.iter().filter(|u| u.status == status).collect();
        rows.sort_by_key(|u| u.line);
        rows.into_iter()
            .map(|u| {
                json!({
                    "kind": u.kind,
                    "name": u.name,
                    "signature": u.signature,
                    "line": u.line,
                    "hash": u.hash,
                })
            })
            .collect()
    };
    json!({
        "changed": section(UnitStatus::Changed),
        "added": section(UnitStatus::Added),
        "removed": section(UnitStatus::Removed),
        "otherFormsChanged": diff.other_forms_changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statuses(diff: &FileDiff, status: UnitStatus) -> Vec<&str> {
        let mut names: Vec<&str> = diff
            .units
            .iter()
            .filter(|u| u.status == status)
            .map(|u| u.name.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn added_removed_changed_unchanged() {
        let old = "(defun keep () 1)\n(defun gone () 2)\n(defun edit () 3)\n";
        let new = "(defun keep () 1)\n(defun edit () 30)\n(defun fresh () 4)\n";
        let d = diff_files(old, new, Dialect::EmacsLisp);
        assert_eq!(statuses(&d, UnitStatus::Added), ["fresh"]);
        assert_eq!(statuses(&d, UnitStatus::Removed), ["gone"]);
        assert_eq!(statuses(&d, UnitStatus::Changed), ["edit"]);
        // `keep` is unchanged and therefore not emitted.
        assert_eq!(d.units.len(), 3);
        assert!(!d.other_forms_changed);
    }

    #[test]
    fn whitespace_and_comment_only_is_unchanged() {
        let old = "(defun f (x) (+ x 1))\n";
        let new = "(defun f (x)\n  ;; a new comment\n  (+   x   1))\n";
        let d = diff_files(old, new, Dialect::EmacsLisp);
        assert!(
            d.is_empty(),
            "formatting/comment churn must not be a change"
        );
    }

    #[test]
    fn rename_is_add_plus_remove() {
        let old = "(defun old-name (x) (* x 2))\n";
        let new = "(defun new-name (x) (* x 2))\n";
        let d = diff_files(old, new, Dialect::EmacsLisp);
        assert_eq!(statuses(&d, UnitStatus::Removed), ["old-name"]);
        assert_eq!(statuses(&d, UnitStatus::Added), ["new-name"]);
        assert_eq!(statuses(&d, UnitStatus::Changed), Vec::<&str>::new());
    }

    #[test]
    fn same_name_methods_keyed_by_dispatch() {
        // Two methods of the same generic differ only by specializer; editing one
        // must not read as "the other vanished".
        let old = concat!(
            "(cl-defmethod area ((s square)) (* s s))\n",
            "(cl-defmethod area ((c circle)) (* 3 c c))\n"
        );
        let new = concat!(
            "(cl-defmethod area ((s square)) (* s s s))\n",
            "(cl-defmethod area ((c circle)) (* 3 c c))\n"
        );
        let d = diff_files(old, new, Dialect::EmacsLisp);
        // Exactly the `square` method changed; the `circle` method is unchanged.
        assert_eq!(statuses(&d, UnitStatus::Changed), ["area"]);
        assert_eq!(d.units.len(), 1);
        let changed = &d.units[0];
        assert_eq!(changed.signature.as_deref(), Some("(square)"));
    }

    #[test]
    fn other_top_level_forms_summary() {
        let old = "(require 'foo)\n(defun f () 1)\n";
        let new = "(require 'bar)\n(defun f () 1)\n";
        let d = diff_files(old, new, Dialect::EmacsLisp);
        assert!(d.units.is_empty(), "no definition changed");
        assert!(d.other_forms_changed, "the require form changed");
    }

    #[test]
    fn identical_files_are_empty() {
        let src = "(defun f (x) (+ x 1))\n(defvar y 2)\n";
        assert!(diff_files(src, src, Dialect::EmacsLisp).is_empty());
    }

    #[test]
    fn duplicate_key_forward_decl_then_real_is_stable() {
        // Emacs's idiom: a `(defvar x)` forward declaration and a later
        // `(defvar x nil)` share the (kind, name) key. Diffing the file against
        // itself must be empty — the two instances must not mispair.
        let src = "(defvar c-x)\n(defun mid () 1)\n(defvar c-x nil)\n";
        assert!(
            diff_files(src, src, Dialect::EmacsLisp).is_empty(),
            "duplicate-key definitions must not read as changed against self"
        );
        // Changing only the real definition reports exactly one change; the
        // forward declaration stays unchanged.
        let new = "(defvar c-x)\n(defun mid () 1)\n(defvar c-x 42)\n";
        let d = diff_files(src, new, Dialect::EmacsLisp);
        assert_eq!(statuses(&d, UnitStatus::Changed), ["c-x"]);
        assert_eq!(d.units.len(), 1);
        // Dropping the forward declaration removes exactly it; the real one stays.
        let dropped = "(defun mid () 1)\n(defvar c-x nil)\n";
        let d = diff_files(src, dropped, Dialect::EmacsLisp);
        assert_eq!(statuses(&d, UnitStatus::Removed), ["c-x"]);
        assert_eq!(d.units.len(), 1);
    }
}
