//! Structural diff (ADR-0047) — a read-only observation of how two versions of a
//! file differ, at top-level **definition** granularity: the "attention map" an
//! agent reads first to see *which* units changed before drilling into *how*
//! (the ADR-0048 tree diff, a later slice).
//!
//! A **unit** is a top-level definition (a named `outline` entry at depth 0).
//! Units are matched across the two versions by `(kind, name, dispatch?)` — plain
//! definitions carry no Dispatch signature, method-like forms add theirs so two
//! methods of one generic stay distinct. A matched pair is *changed* iff it is
//! not [`struct_eq`] (formatting-modulo), so reindent- or
//! comment-only churn never registers. No rename detection: a rename surfaces as
//! one removed + one added unit. Top-level non-definition forms (`require`,
//! `provide`, …) have no stable name key and are not diffed individually — only a
//! single "other top-level forms changed" summary flag reports them.

use std::collections::HashMap;

use lispexp::annotate::{annotate_tree, bundled_registry, Role};
use lispexp::{parse, Datum, DatumKind, Options};

use crate::hash::anchor_hash;
use crate::sexpr::{opt_eq, struct_eq};
use crate::{dispatch_signature, name_text, node_lens, span_bytes, Dialect, NodeEntry};

/// How a definition unit differs between the two versions. `Unchanged` units are
/// not emitted (see [`FileDiff::units`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitStatus {
    /// Present only in the new version.
    Added,
    /// Present only in the old version.
    Removed,
    /// Present in both, but not [`struct_eq`].
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

/// The matched result of two unit lists: changed pairs (`old`, `new`) plus the
/// units present on only one side.
struct Matched<'u, 'a> {
    changed: Vec<(&'u Unit<'a>, &'u Unit<'a>)>,
    added: Vec<&'u Unit<'a>>,
    removed: Vec<&'u Unit<'a>>,
}

/// Match two unit lists by key (ADR-0047). A key can repeat within one file —
/// e.g. Emacs's `(defvar x)` forward declaration plus its later `(defvar x nil)`
/// — so each key maps to a *list*; within a key, exact `struct_eq` instances are
/// consumed first (unchanged), the remainder paired positionally (changed), and
/// the leftover tail is added/removed. Matching a single instance per key would
/// mispair the duplicates and falsely report a change.
fn match_units<'u, 'a>(old_units: &'u [Unit<'a>], new_units: &'u [Unit<'a>]) -> Matched<'u, 'a> {
    let old_by_key = bucket_by_key(old_units);
    let new_by_key = bucket_by_key(new_units);
    let mut keys: Vec<&Key> = old_by_key
        .keys()
        .chain(new_by_key.keys())
        .copied()
        .collect();
    keys.sort_unstable();
    keys.dedup();

    let mut matched = Matched {
        changed: Vec::new(),
        added: Vec::new(),
        removed: Vec::new(),
    };
    let empty: Vec<&Unit> = Vec::new();
    for key in keys {
        let olds = old_by_key.get(key).unwrap_or(&empty);
        let news = new_by_key.get(key).unwrap_or(&empty);
        let mut new_used = vec![false; news.len()];
        let mut old_unmatched: Vec<&Unit> = Vec::new();
        for ou in olds {
            let mut hit = false;
            for (j, nu) in news.iter().enumerate() {
                if !new_used[j] && struct_eq(ou.form, nu.form) {
                    new_used[j] = true;
                    hit = true;
                    break;
                }
            }
            if !hit {
                old_unmatched.push(ou);
            }
        }
        let new_unmatched: Vec<&Unit> = news
            .iter()
            .enumerate()
            .filter(|(j, _)| !new_used[*j])
            .map(|(_, nu)| *nu)
            .collect();
        let common = old_unmatched.len().min(new_unmatched.len());
        for k in 0..common {
            matched.changed.push((old_unmatched[k], new_unmatched[k]));
        }
        matched.removed.extend_from_slice(&old_unmatched[common..]);
        matched.added.extend_from_slice(&new_unmatched[common..]);
    }
    matched
}

/// Compare two versions of a source at definition granularity (ADR-0047).
pub fn diff_files(old: &str, new: &str, dialect: Dialect) -> FileDiff {
    let old_parsed = parse(old, &Options::for_dialect(dialect));
    let new_parsed = parse(new, &Options::for_dialect(dialect));
    let (old_units, old_others) = collect_units(old, &old_parsed.data, dialect);
    let (new_units, new_others) = collect_units(new, &new_parsed.data, dialect);

    let matched = match_units(&old_units, &new_units);
    let mut units = Vec::new();
    for (_, nu) in &matched.changed {
        units.push(unit_diff(nu, UnitStatus::Changed));
    }
    for nu in &matched.added {
        units.push(unit_diff(nu, UnitStatus::Added));
    }
    for ou in &matched.removed {
        units.push(unit_diff(ou, UnitStatus::Removed));
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

// ===========================================================================
// Tree diff within a unit (ADR-0048)
// ===========================================================================

/// A short, one-line source fragment of a form plus its editing anchor — what a
/// `Replace`/`Added`/`Removed` node carries for display and for jumping to an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frag {
    /// A one-line, whitespace-collapsed, truncated preview of the verbatim form.
    pub text: String,
    /// 1-based start line of the form.
    pub line: u32,
    /// 4-hex anchor hash over the form's full span (ADR-0008).
    pub hash: String,
}

/// How two aligned forms differ (ADR-0048). `Descend` recursed into a same-shape
/// container (same-delimiter list, or same-notation prefix); `Replace` is an
/// opaque leaf/category change carrying both sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormDiff {
    /// Recursed into a container: its changed child positions (unchanged children
    /// are omitted; `child_count` is the new side's total so a renderer can show
    /// elision).
    Descend {
        /// A short label for the container — its head symbol (or a delimiter hint).
        label: String,
        /// Total number of children on the new side.
        child_count: usize,
        /// The changed children, in new-side order.
        children: Vec<ChildDiff>,
    },
    /// Not the same category (differing leaves, list↔atom, delimiter or notation
    /// mismatch): the whole form is replaced.
    Replace {
        /// The old form.
        old: Frag,
        /// The new form.
        new: Frag,
    },
}

/// One changed child within a [`FormDiff::Descend`] (ADR-0048's four statuses at
/// the child level; unchanged children are not emitted). `index` is the child's
/// position on its own side, so a renderer can place it and infer elision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildDiff {
    /// A child present only in the new version.
    Added { index: usize, frag: Frag },
    /// A child present only in the old version.
    Removed { index: usize, frag: Frag },
    /// A child present in both but differing — recursed.
    Paired { index: usize, diff: FormDiff },
}

/// Verbatim source of `d`'s span.
fn slice<'a>(src: &'a str, d: &Datum) -> &'a str {
    &src[d.span.start as usize..d.span.end as usize]
}

/// One-line, whitespace-collapsed, truncated preview.
fn one_line(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let head: String = chars.by_ref().take(60).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn frag(d: &Datum, src: &str) -> Frag {
    Frag {
        text: one_line(slice(src, d)),
        line: d.line,
        hash: anchor_hash(span_bytes(src, d)),
    }
}

/// A container's short label: its head symbol if it has one, else a delimiter hint.
fn container_label(kind: &DatumKind) -> String {
    match kind {
        DatumKind::List { items, .. } => match items.first().map(|d| &d.kind) {
            Some(DatumKind::Symbol(s)) => s.to_string(),
            _ => "(…)".to_string(),
        },
        DatumKind::Prefixed { .. } => "prefix".to_string(),
        _ => "…".to_string(),
    }
}

/// Structural diff of two forms (ADR-0048). `None` when they are [`struct_eq`] —
/// i.e. there is no change modulo formatting.
pub fn diff_forms(old: &Datum, new: &Datum, old_src: &str, new_src: &str) -> Option<FormDiff> {
    if struct_eq(old, new) {
        return None;
    }
    Some(diff_forms_inner(old, new, old_src, new_src))
}

fn diff_forms_inner(old: &Datum, new: &Datum, os: &str, ns: &str) -> FormDiff {
    match (&old.kind, &new.kind) {
        // Same-delimiter list with equal dotted tails → recurse over children.
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
        ) if da == db && opt_eq(ta.as_deref(), tb.as_deref()) => FormDiff::Descend {
            label: container_label(&new.kind),
            child_count: ib.len(),
            children: align_children(ia, ib, os, ns),
        },
        // Same-notation prefix with an equal auxiliary arg → recurse into the inner.
        (
            DatumKind::Prefixed {
                notation: na,
                inner: ina,
                arg: aa,
                ..
            },
            DatumKind::Prefixed {
                notation: nb,
                inner: inb,
                arg: ab,
                ..
            },
        ) if na == nb && opt_eq(aa.as_deref(), ab.as_deref()) => {
            // The inner must differ (the whole form is not `struct_eq`).
            let children = diff_forms(ina, inb, os, ns)
                .map(|d| vec![ChildDiff::Paired { index: 0, diff: d }])
                .unwrap_or_default();
            FormDiff::Descend {
                label: container_label(&new.kind),
                child_count: 1,
                children,
            }
        }
        // Everything else: an opaque replace carrying both sides.
        _ => FormDiff::Replace {
            old: frag(old, os),
            new: frag(new, ns),
        },
    }
}

/// The struct_eq LCS of two child sequences as `(old_index, new_index)` anchor
/// pairs, increasing on both axes — the unchanged children the alignment pins.
fn lcs_matches(a: &[Datum], b: &[Datum]) -> Vec<(usize, usize)> {
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0u32; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if struct_eq(&a[i], &b[j]) {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < m && j < n {
        if struct_eq(&a[i], &b[j]) {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Align two child sequences (ADR-0048): `struct_eq` LCS anchors the unchanged
/// children, and each divergent gap pairs old/new positionally (recursing) with
/// the count difference as added/removed. Anchored (unchanged) children are not
/// emitted.
fn align_children(a: &[Datum], b: &[Datum], os: &str, ns: &str) -> Vec<ChildDiff> {
    let mut children = Vec::new();
    let (mut pi, mut pj) = (0usize, 0usize);
    let anchors = lcs_matches(a, b)
        .into_iter()
        .chain(std::iter::once((a.len(), b.len())));
    for (ai, bj) in anchors {
        let old_gap = &a[pi..ai];
        let new_gap = &b[pj..bj];
        let common = old_gap.len().min(new_gap.len());
        for k in 0..common {
            if let Some(diff) = diff_forms(&old_gap[k], &new_gap[k], os, ns) {
                children.push(ChildDiff::Paired {
                    index: pj + k,
                    diff,
                });
            }
        }
        // Removed sit at the current new-side cursor; added follow the paired run.
        for od in &old_gap[common..] {
            children.push(ChildDiff::Removed {
                index: pj + common,
                frag: frag(od, os),
            });
        }
        for (k, nd) in new_gap.iter().enumerate().skip(common) {
            children.push(ChildDiff::Added {
                index: pj + k,
                frag: frag(nd, ns),
            });
        }
        pi = ai + 1;
        pj = bj + 1;
    }
    children
}

/// A changed unit together with its intra-unit [`FormDiff`] — the deep diff of one
/// definition (ADR-0048), the output of [`diff_files_deep`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitTreeDiff {
    /// The defining head symbol (e.g. `defun`).
    pub kind: String,
    /// The defined name.
    pub name: String,
    /// A method's Dispatch signature, or `None`.
    pub signature: Option<String>,
    /// New-version start line of the definition.
    pub line: u32,
    /// New-version anchor hash of the definition form.
    pub hash: String,
    /// How the definition's body changed.
    pub diff: FormDiff,
}

/// An added or removed definition rendered as its expandable **Lens** (#44): the
/// definition's subtree in pre-order with an anchor + preview per node, so an
/// agent reading a deep diff can see *what a new/gone definition contains* — not
/// just its name — while staying token-conscious (previews, not verbatim source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitLens {
    /// The defining head symbol (e.g. `defun`).
    pub kind: String,
    /// The defined name.
    pub name: String,
    /// A method's Dispatch signature, or `None`.
    pub signature: Option<String>,
    /// Start line of the definition (new side for added, old side for removed).
    pub line: u32,
    /// Anchor hash of the definition form (same side as `line`).
    pub hash: String,
    /// The definition's expandable Lens (ADR-0013): node per line, `depth` 0 is
    /// the definition itself.
    pub lens: Vec<NodeEntry>,
    /// Full-verbatim source of the definition — populated only in `verbose` mode
    /// (`--verbose`), where the token-bounded Lens previews are not enough and the
    /// caller wants the added/removed body's exact text. `None` otherwise.
    pub verbatim: Option<String>,
}

/// The result of [`diff_files_deep`] (ADR-0048, #44): changed definitions with
/// their intra-unit tree diff, plus added/removed definitions with their Lens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepDiff {
    /// Changed definitions and how their bodies changed.
    pub changed: Vec<UnitTreeDiff>,
    /// Added definitions (new side) with their Lens.
    pub added: Vec<UnitLens>,
    /// Removed definitions (old side) with their Lens.
    pub removed: Vec<UnitLens>,
}

impl DeepDiff {
    /// Whether nothing changed at the definition level.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }
}

/// Deep diff (ADR-0048, #44): for every *changed* definition the intra-unit
/// [`FormDiff`], and for every *added*/*removed* definition its expandable Lens —
/// all optionally filtered to a `name`. So a deep view shows both how changed
/// definitions changed *and* what added/removed ones contain.
///
/// With `verbose`, each added/removed definition additionally carries its
/// full-verbatim source ([`UnitLens::verbatim`]); the renderers then show the
/// exact body instead of the token-bounded Lens previews (ADR-0048's sanctioned
/// `--verbose` opt-in). Changed definitions are unaffected — their tree diff is
/// already exact.
pub fn diff_files_deep(
    old: &str,
    new: &str,
    dialect: Dialect,
    name: Option<&str>,
    verbose: bool,
) -> DeepDiff {
    let old_parsed = parse(old, &Options::for_dialect(dialect));
    let new_parsed = parse(new, &Options::for_dialect(dialect));
    let (old_units, _) = collect_units(old, &old_parsed.data, dialect);
    let (new_units, _) = collect_units(new, &new_parsed.data, dialect);
    let matched = match_units(&old_units, &new_units);
    let wanted = |u: &Unit| name.is_none_or(|want| u.key.1 == want);

    let mut changed = Vec::new();
    for (ou, nu) in &matched.changed {
        if !wanted(nu) {
            continue;
        }
        if let Some(diff) = diff_forms(ou.form, nu.form, old, new) {
            changed.push(UnitTreeDiff {
                kind: nu.key.0.clone(),
                name: nu.key.1.clone(),
                signature: nu.key.2.clone(),
                line: nu.line,
                hash: nu.hash.clone(),
                diff,
            });
        }
    }
    changed.sort_by_key(|u| u.line);

    let lens_of = |u: &Unit, src: &str| UnitLens {
        kind: u.key.0.clone(),
        name: u.key.1.clone(),
        signature: u.key.2.clone(),
        line: u.line,
        hash: u.hash.clone(),
        lens: node_lens(src, u.form),
        verbatim: verbose.then(|| slice(src, u.form).to_string()),
    };
    let mut added: Vec<UnitLens> = matched
        .added
        .iter()
        .filter(|u| wanted(u))
        .map(|u| lens_of(u, new))
        .collect();
    added.sort_by_key(|u| u.line);
    let mut removed: Vec<UnitLens> = matched
        .removed
        .iter()
        .filter(|u| wanted(u))
        .map(|u| lens_of(u, old))
        .collect();
    removed.sort_by_key(|u| u.line);

    DeepDiff {
        changed,
        added,
        removed,
    }
}

/// Parse two source snippets each as a single form and diff them (ADR-0048's
/// general two-form primitive; the MCP form-string path). `None` if either side
/// has no form, or they are equal modulo formatting.
pub fn diff_source_forms(old: &str, new: &str, dialect: Dialect) -> Option<FormDiff> {
    let old_parsed = parse(old, &Options::for_dialect(dialect));
    let new_parsed = parse(new, &Options::for_dialect(dialect));
    let od = old_parsed.data.first()?;
    let nd = new_parsed.data.first()?;
    diff_forms(od, nd, old, new)
}

// ---- Tree-diff rendering ---------------------------------------------------

/// Render a [`FormDiff`] as a pruned structural tree (ADR-0048): the spine of
/// changed paths, unchanged siblings elided to `…`, changes marked `+`/`-`/`~`.
/// `indent` is the current depth (2 spaces each).
pub fn form_diff_text(diff: &FormDiff) -> String {
    let mut out = String::new();
    write_form_diff(diff, 1, &mut out);
    out
}

fn pad(indent: usize) -> String {
    "  ".repeat(indent)
}

fn write_form_diff(diff: &FormDiff, indent: usize, out: &mut String) {
    match diff {
        FormDiff::Replace { old, new } => {
            out.push_str(&format!("{}~ {} ⇒ {}\n", pad(indent), old.text, new.text));
        }
        FormDiff::Descend {
            label,
            child_count,
            children,
        } => {
            out.push_str(&format!("{}({label}\n", pad(indent)));
            let mut prev: Option<usize> = None;
            for child in children {
                let idx = child_index(child);
                if prev.map_or(idx > 0, |p| idx > p + 1) {
                    out.push_str(&format!("{}…\n", pad(indent + 1)));
                }
                write_child_diff(child, indent + 1, out);
                prev = Some(idx);
            }
            if prev.map_or(*child_count > 0, |p| p + 1 < *child_count) {
                out.push_str(&format!("{}…\n", pad(indent + 1)));
            }
            out.push_str(&format!("{})\n", pad(indent)));
        }
    }
}

fn child_index(child: &ChildDiff) -> usize {
    match child {
        ChildDiff::Added { index, .. }
        | ChildDiff::Removed { index, .. }
        | ChildDiff::Paired { index, .. } => *index,
    }
}

fn write_child_diff(child: &ChildDiff, indent: usize, out: &mut String) {
    match child {
        ChildDiff::Added { frag, .. } => {
            out.push_str(&format!("{}+ {}\n", pad(indent), frag.text));
        }
        ChildDiff::Removed { frag, .. } => {
            out.push_str(&format!("{}- {}\n", pad(indent), frag.text));
        }
        ChildDiff::Paired { diff, .. } => write_form_diff(diff, indent, out),
    }
}

/// Render a [`FormDiff`] as JSON (ADR-0048): a recursive node with `status` and
/// either `children` (descend) or `old`/`new` frags (replace).
pub fn form_diff_json(diff: &FormDiff) -> serde_json::Value {
    use serde_json::json;
    match diff {
        FormDiff::Replace { old, new } => json!({
            "status": "replaced",
            "old": frag_json(old),
            "new": frag_json(new),
        }),
        FormDiff::Descend {
            label,
            child_count,
            children,
        } => json!({
            "status": "changed",
            "label": label,
            "childCount": child_count,
            "children": children.iter().map(child_diff_json).collect::<Vec<_>>(),
        }),
    }
}

fn frag_json(f: &Frag) -> serde_json::Value {
    serde_json::json!({ "text": f.text, "line": f.line, "hash": f.hash })
}

fn child_diff_json(child: &ChildDiff) -> serde_json::Value {
    use serde_json::json;
    match child {
        ChildDiff::Added { index, frag } => {
            json!({ "status": "added", "index": index, "new": frag_json(frag) })
        }
        ChildDiff::Removed { index, frag } => {
            json!({ "status": "removed", "index": index, "old": frag_json(frag) })
        }
        ChildDiff::Paired { index, diff } => {
            let mut node = form_diff_json(diff);
            node.as_object_mut()
                .unwrap()
                .insert("index".into(), json!(index));
            node
        }
    }
}

fn sig_suffix(signature: &Option<String>) -> String {
    signature
        .as_deref()
        .map(|s| format!(" {s}"))
        .unwrap_or_default()
}

/// Render a deep diff ([`diff_files_deep`]) as text: changed definitions with
/// their pruned tree, then added / removed definitions with their Lens (#44).
pub fn deep_text(diff: &DeepDiff) -> String {
    let mut out = String::new();
    for u in &diff.changed {
        out.push_str(&format!(
            "~ {} {}{}  {}:{}\n",
            u.kind,
            u.name,
            sig_suffix(&u.signature),
            u.line,
            u.hash
        ));
        out.push_str(&form_diff_text(&u.diff));
    }
    let mut lens_section = |marker: char, units: &[UnitLens]| {
        for u in units {
            out.push_str(&format!(
                "{marker} {} {}{}  {}:{}\n",
                u.kind,
                u.name,
                sig_suffix(&u.signature),
                u.line,
                u.hash
            ));
            if let Some(src) = &u.verbatim {
                // Verbose: the definition's exact source, each line indented one
                // step under the header.
                for line in src.lines() {
                    out.push_str("  ");
                    out.push_str(line);
                    out.push('\n');
                }
                continue;
            }
            // Skip depth 0 (the definition itself, already in the header); show
            // its inner Lens indented.
            for node in u.lens.iter().skip(1) {
                out.push_str(&format!(
                    "{}{}  {}\n",
                    "  ".repeat(node.depth as usize),
                    node.hash,
                    node.preview
                ));
            }
        }
    };
    lens_section('+', &diff.added);
    lens_section('-', &diff.removed);
    out
}

/// Render a deep diff ([`diff_files_deep`]) as JSON: `{changed, added, removed}`,
/// where added/removed carry their Lens nodes (#44).
pub fn deep_json(diff: &DeepDiff) -> serde_json::Value {
    use serde_json::json;
    let changed: Vec<_> = diff
        .changed
        .iter()
        .map(|u| {
            json!({
                "status": "changed",
                "kind": u.kind,
                "name": u.name,
                "signature": u.signature,
                "line": u.line,
                "hash": u.hash,
                "diff": form_diff_json(&u.diff),
            })
        })
        .collect();
    let lens_arr = |status: &str, units: &[UnitLens]| -> Vec<serde_json::Value> {
        units
            .iter()
            .map(|u| {
                json!({
                    "status": status,
                    "kind": u.kind,
                    "name": u.name,
                    "signature": u.signature,
                    "line": u.line,
                    "hash": u.hash,
                    "lens": u.lens.iter().map(|n| json!({
                        "line": n.line, "depth": n.depth, "hash": n.hash, "preview": n.preview,
                    })).collect::<Vec<_>>(),
                    "verbatim": u.verbatim,
                })
            })
            .collect()
    };
    json!({
        "changed": changed,
        "added": lens_arr("added", &diff.added),
        "removed": lens_arr("removed", &diff.removed),
    })
}

// ---- HTML rendering (#42) --------------------------------------------------
//
// A self-contained, human-facing view of a diff for eyeball reading. It renders
// the *same* structures the text/JSON renderers use — no re-parsing, no new diff
// logic — into one standalone HTML document (inline CSS, no external assets), so
// an agent can hand the file to a person or open it in a browser.

/// HTML-escape a source fragment. Lisp is full of `<`/`>`/`&` (cc-engine's
/// `c-forward-<>-arglist`, `&optional`, string literals), so every fragment,
/// name, and preview must pass through here before landing in the document.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

const HTML_STYLE: &str = "\
:root{--add:#1a7f37;--add-bg:#e6f4ea;--del:#b3261e;--del-bg:#fbeae9;\
--chg:#8a5a00;--chg-bg:#fbf0d9;--ink:#1b1f24;--muted:#6a737d;--line:#e2e5ea;--bg:#f6f7f8}\
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);\
font:14px/1.5 system-ui,-apple-system,Segoe UI,Roboto,sans-serif}\
.wrap{max-width:960px;margin:0 auto;padding:28px 20px}\
h1{font-size:20px;margin:0 0 4px}.sub{color:var(--muted);font-size:13px;margin-bottom:16px}\
.pills{display:flex;gap:8px;flex-wrap:wrap;margin-bottom:22px}\
.pill{font:600 12px ui-monospace,Menlo,monospace;padding:3px 10px;border-radius:999px;border:1px solid var(--line);background:#fff}\
.pill.add{color:var(--add);background:var(--add-bg);border-color:transparent}\
.pill.del{color:var(--del);background:var(--del-bg);border-color:transparent}\
.pill.chg{color:var(--chg);background:var(--chg-bg);border-color:transparent}\
.unit{background:#fff;border:1px solid var(--line);border-radius:10px;margin:0 0 14px;overflow:hidden}\
.uhead{display:flex;gap:8px;align-items:baseline;padding:10px 14px;border-bottom:1px solid var(--line);\
font:13px ui-monospace,Menlo,monospace}.uhead .mk{font-weight:700}\
.uhead .k{color:var(--muted)}.uhead .n{font-weight:600}.uhead .a{margin-left:auto;color:var(--muted);font-size:12px}\
.mk.add,.k.add{color:var(--add)}.mk.del,.k.del{color:var(--del)}.mk.chg{color:var(--chg)}\
.tree{margin:0;padding:12px 14px;font:12.5px/1.55 ui-monospace,Menlo,monospace;overflow-x:auto;white-space:pre}\
.row{white-space:pre}.row.add{color:var(--add)}.row.del{color:var(--del)}\
.row .rep-old{color:var(--del);text-decoration:line-through;text-decoration-color:#d9a} \
.row .rep-new{color:var(--add)}.row .arrow{color:var(--muted)}.row .lbl{color:var(--chg)}\
.ell{color:var(--muted)}.empty{color:var(--muted);padding:8px 14px;font-style:italic}\
footer{color:var(--muted);font:12px ui-monospace,Menlo,monospace;margin-top:8px}";

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>{}</title><style>{}</style></head><body><div class=\"wrap\">{}</div></body></html>\n",
        esc(title),
        HTML_STYLE,
        body
    )
}

fn pad_em(depth: usize) -> String {
    format!("padding-left:{}em", depth as f32 * 1.4)
}

fn sig_html(signature: &Option<String>) -> String {
    signature
        .as_deref()
        .map(|s| format!(" {}", esc(s)))
        .unwrap_or_default()
}

/// Render a [`FileDiff`] as a standalone HTML page (the definition-level map).
pub fn diff_html(diff: &FileDiff) -> String {
    let mut b = String::new();
    b.push_str("<h1>Structural diff</h1><div class=\"sub\">definition-level map</div>");
    b.push_str(&summary_pills(
        count(diff, UnitStatus::Changed),
        count(diff, UnitStatus::Added),
        count(diff, UnitStatus::Removed),
        diff.other_forms_changed,
    ));
    if diff.is_empty() {
        b.push_str("<div class=\"empty\">No structural change.</div>");
        return page("Structural diff", &b);
    }
    for (status, cls) in [
        (UnitStatus::Changed, "chg"),
        (UnitStatus::Added, "add"),
        (UnitStatus::Removed, "del"),
    ] {
        let mut rows: Vec<&UnitDiff> = diff.units.iter().filter(|u| u.status == status).collect();
        if rows.is_empty() {
            continue;
        }
        rows.sort_by_key(|u| u.line);
        for u in rows {
            b.push_str(&format!(
                "<div class=\"unit\"><div class=\"uhead\"><span class=\"mk {cls}\">{}</span>\
<span class=\"k\">{}</span> <span class=\"n\">{}</span>{}<span class=\"a\">{}:{}</span></div></div>",
                status.marker(),
                esc(&u.kind),
                esc(&u.name),
                sig_html(&u.signature),
                u.line,
                esc(&u.hash),
            ));
        }
    }
    page("Structural diff", &b)
}

/// Render a [`DeepDiff`] as a standalone HTML page (per-unit trees + Lenses).
pub fn deep_html(diff: &DeepDiff) -> String {
    let mut b = String::new();
    b.push_str("<h1>Structural diff</h1><div class=\"sub\">how each definition changed</div>");
    b.push_str(&summary_pills(
        diff.changed.len(),
        diff.added.len(),
        diff.removed.len(),
        false,
    ));
    if diff.is_empty() {
        b.push_str("<div class=\"empty\">No structural change.</div>");
        return page("Structural diff", &b);
    }
    for u in &diff.changed {
        b.push_str(&format!(
            "<div class=\"unit\"><div class=\"uhead\"><span class=\"mk chg\">~</span>\
<span class=\"k\">{}</span> <span class=\"n\">{}</span>{}<span class=\"a\">{}:{}</span></div>\
<div class=\"tree\">",
            esc(&u.kind),
            esc(&u.name),
            sig_html(&u.signature),
            u.line,
            esc(&u.hash),
        ));
        html_form_diff(&u.diff, 0, &mut b);
        b.push_str("</div></div>");
    }
    for (units, cls, mk) in [(&diff.added, "add", "+"), (&diff.removed, "del", "-")] {
        for u in units {
            b.push_str(&format!(
                "<div class=\"unit\"><div class=\"uhead\"><span class=\"mk {cls}\">{mk}</span>\
<span class=\"k {cls}\">{}</span> <span class=\"n\">{}</span>{}<span class=\"a\">{}:{}</span></div>\
<div class=\"tree\">",
                esc(&u.kind),
                esc(&u.name),
                sig_html(&u.signature),
                u.line,
                esc(&u.hash),
            ));
            if let Some(src) = &u.verbatim {
                // Verbose: the definition's exact source (the `.tree` block is
                // `white-space:pre`, so newlines and indentation survive).
                b.push_str(&format!("<div class=\"row\">{}</div>", esc(src)));
            } else {
                for node in u.lens.iter().skip(1) {
                    b.push_str(&format!(
                        "<div class=\"row\" style=\"{}\">{}  {}</div>",
                        pad_em(node.depth as usize),
                        esc(&node.hash),
                        esc(&node.preview)
                    ));
                }
            }
            b.push_str("</div></div>");
        }
    }
    page("Structural diff", &b)
}

fn count(diff: &FileDiff, status: UnitStatus) -> usize {
    diff.units.iter().filter(|u| u.status == status).count()
}

fn summary_pills(changed: usize, added: usize, removed: usize, other: bool) -> String {
    let mut s = String::from("<div class=\"pills\">");
    s.push_str(&format!(
        "<span class=\"pill chg\">~ {changed} changed</span>"
    ));
    s.push_str(&format!("<span class=\"pill add\">+ {added} added</span>"));
    s.push_str(&format!(
        "<span class=\"pill del\">- {removed} removed</span>"
    ));
    if other {
        s.push_str("<span class=\"pill\">! other top-level forms</span>");
    }
    s.push_str("</div>");
    s
}

fn html_form_diff(diff: &FormDiff, depth: usize, out: &mut String) {
    match diff {
        FormDiff::Replace { old, new } => {
            out.push_str(&format!(
                "<div class=\"row\" style=\"{}\"><span class=\"mk chg\">~</span> \
<span class=\"rep-old\">{}</span> <span class=\"arrow\">⇒</span> <span class=\"rep-new\">{}</span></div>",
                pad_em(depth),
                esc(&old.text),
                esc(&new.text)
            ));
        }
        FormDiff::Descend {
            label,
            child_count,
            children,
        } => {
            out.push_str(&format!(
                "<div class=\"row\" style=\"{}\">(<span class=\"lbl\">{}</span></div>",
                pad_em(depth),
                esc(label)
            ));
            let mut prev: Option<usize> = None;
            for child in children {
                let idx = child_index(child);
                if prev.map_or(idx > 0, |p| idx > p + 1) {
                    out.push_str(&format!(
                        "<div class=\"row ell\" style=\"{}\">…</div>",
                        pad_em(depth + 1)
                    ));
                }
                html_child_diff(child, depth + 1, out);
                prev = Some(idx);
            }
            if prev.map_or(*child_count > 0, |p| p + 1 < *child_count) {
                out.push_str(&format!(
                    "<div class=\"row ell\" style=\"{}\">…</div>",
                    pad_em(depth + 1)
                ));
            }
            out.push_str(&format!(
                "<div class=\"row\" style=\"{}\">)</div>",
                pad_em(depth)
            ));
        }
    }
}

fn html_child_diff(child: &ChildDiff, depth: usize, out: &mut String) {
    match child {
        ChildDiff::Added { frag, .. } => out.push_str(&format!(
            "<div class=\"row add\" style=\"{}\">+ {}</div>",
            pad_em(depth),
            esc(&frag.text)
        )),
        ChildDiff::Removed { frag, .. } => out.push_str(&format!(
            "<div class=\"row del\" style=\"{}\">- {}</div>",
            pad_em(depth),
            esc(&frag.text)
        )),
        ChildDiff::Paired { diff, .. } => html_form_diff(diff, depth, out),
    }
}

/// Render a single [`FormDiff`] (the two-form primitive) as a standalone HTML page.
pub fn form_diff_html(diff: &FormDiff) -> String {
    let mut b = String::from("<h1>Structural diff</h1><div class=\"sub\">two forms</div>");
    b.push_str("<div class=\"unit\"><div class=\"tree\">");
    html_form_diff(diff, 0, &mut b);
    b.push_str("</div></div>");
    page("Structural diff", &b)
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

    // ---- Tree diff (ADR-0048) ----

    fn only_child(diff: &FormDiff) -> &ChildDiff {
        match diff {
            FormDiff::Descend { children, .. } => {
                assert_eq!(children.len(), 1, "expected exactly one changed child");
                &children[0]
            }
            FormDiff::Replace { .. } => panic!("expected a Descend, got a Replace"),
        }
    }

    fn diff2(old: &str, new: &str) -> FormDiff {
        diff_source_forms(old, new, Dialect::EmacsLisp).expect("forms should differ")
    }

    #[test]
    fn formatting_modulo_is_no_diff() {
        assert!(diff_source_forms("(f  a\n   b)", "(f a b)", Dialect::EmacsLisp).is_none());
    }

    #[test]
    fn head_change_is_a_child_zero_replace() {
        // `when` -> `unless`: the head is child 0, so it reads as a replace there,
        // not a whole-form replace.
        let d = diff2("(when c a)", "(unless c a)");
        match only_child(&d) {
            ChildDiff::Paired { index, diff } => {
                assert_eq!(*index, 0);
                assert!(matches!(diff, FormDiff::Replace { .. }));
            }
            other => panic!("expected a paired child-0 replace, got {other:?}"),
        }
    }

    #[test]
    fn localized_edit_stays_local() {
        // Only the middle arg changed; the neighbours are unchanged and elided.
        let d = diff2("(f a b c)", "(f a x c)");
        match only_child(&d) {
            ChildDiff::Paired { index, diff } => {
                assert_eq!(*index, 2);
                assert!(matches!(diff, FormDiff::Replace { .. }));
            }
            other => panic!("expected one paired replace at index 2, got {other:?}"),
        }
    }

    #[test]
    fn insertion_is_one_added_child() {
        let d = diff2("(f a b)", "(f a z b)");
        match only_child(&d) {
            ChildDiff::Added { index, frag } => {
                assert_eq!(*index, 2);
                assert_eq!(frag.text, "z");
            }
            other => panic!("expected one added child, got {other:?}"),
        }
    }

    #[test]
    fn delimiter_mismatch_is_a_replace() {
        let d = diff2("(a b)", "[a b]");
        assert!(
            matches!(d, FormDiff::Replace { .. }),
            "different delimiters must not recurse"
        );
    }

    #[test]
    fn reorder_is_add_remove_or_change_not_empty() {
        // A documented non-goal: a reorder is *not* detected as a move — it
        // surfaces as some non-empty set of child changes. Pinned so the behavior
        // stays known.
        let d = diff_source_forms("(f a b)", "(f b a)", Dialect::EmacsLisp);
        assert!(d.is_some(), "a reorder is a (non-move) change, not empty");
    }

    #[test]
    fn deep_diff_of_a_changed_defun() {
        let old = "(defun g (x) (+ x 1))\n";
        let new = "(defun g (x) (+ x 2))\n";
        let deep = diff_files_deep(old, new, Dialect::EmacsLisp, None, false);
        assert_eq!(deep.changed.len(), 1);
        assert_eq!(deep.changed[0].name, "g");
        // The 1 -> 2 replace is somewhere in the tree; the text renders it.
        let text = form_diff_text(&deep.changed[0].diff);
        assert!(text.contains("1 ⇒ 2"), "rendered tree:\n{text}");
    }

    #[test]
    fn deep_diff_unit_filter() {
        let old = "(defun a () 1)\n(defun b () 2)\n";
        let new = "(defun a () 10)\n(defun b () 20)\n";
        assert_eq!(
            diff_files_deep(old, new, Dialect::EmacsLisp, None, false)
                .changed
                .len(),
            2
        );
        let only_b = diff_files_deep(old, new, Dialect::EmacsLisp, Some("b"), false);
        assert_eq!(only_b.changed.len(), 1);
        assert_eq!(only_b.changed[0].name, "b");
    }

    #[test]
    fn deep_diff_renders_added_and_removed_bodies() {
        // #44: an added definition carries its Lens (inner nodes with previews),
        // not just its name; likewise a removed one.
        let old = "(defun gone (x) (* x 2))\n";
        let new = "(defun fresh (n) (let ((r (+ n 1))) (message \"%s\" r)))\n";
        let deep = diff_files_deep(old, new, Dialect::EmacsLisp, None, false);
        assert!(deep.changed.is_empty());
        assert_eq!(deep.added.len(), 1);
        assert_eq!(deep.removed.len(), 1);
        let added = &deep.added[0];
        assert_eq!(added.name, "fresh");
        // The Lens has more than the definition node alone — it exposes the body.
        assert!(
            added.lens.len() > 1,
            "added definition should carry its inner Lens"
        );
        // The body content is visible via previews.
        let text = deep_text(&deep);
        assert!(text.contains("+ defun fresh"), "text:\n{text}");
        assert!(text.contains("- defun gone"), "text:\n{text}");
        assert!(
            text.contains("message"),
            "added body preview missing:\n{text}"
        );
        // Every Lens node carries a usable anchor hash.
        assert!(added.lens.iter().all(|n| n.hash.len() == 4));
        // Non-verbose: no verbatim body is attached.
        assert!(added.verbatim.is_none());
    }

    #[test]
    fn deep_diff_verbose_renders_full_verbatim_bodies() {
        // `--verbose`: an added/removed definition renders its exact source, so a
        // token beyond the 60-char preview boundary — invisible in the default
        // Lens view — shows up in full.
        let old = "(defun gone () nil)\n";
        let new = "\
(defun fresh (n)
  \"A long docstring past the sixty-character preview boundary END-MARKER-ZZZ.\"
  (message \"%s\" n))
";
        let terse = diff_files_deep(old, new, Dialect::EmacsLisp, None, false);
        assert!(
            !deep_text(&terse).contains("END-MARKER-ZZZ"),
            "the terse preview should truncate the long docstring"
        );

        let deep = diff_files_deep(old, new, Dialect::EmacsLisp, None, true);
        let added = &deep.added[0];
        // The exact source is attached and carries the whole body verbatim.
        let verbatim = added
            .verbatim
            .as_deref()
            .expect("verbose attaches verbatim");
        assert!(verbatim.contains("END-MARKER-ZZZ"));
        assert!(verbatim.contains("(message \"%s\" n)"));
        // …and it reaches the rendered text (and HTML) untruncated.
        let text = deep_text(&deep);
        assert!(text.contains("+ defun fresh"), "text:\n{text}");
        assert!(
            text.contains("END-MARKER-ZZZ"),
            "verbatim body missing:\n{text}"
        );
        assert!(deep_html(&deep).contains("END-MARKER-ZZZ"));
        // JSON exposes the verbatim field alongside the Lens.
        let json = deep_json(&deep);
        let added_json = &json["added"][0];
        assert!(added_json["verbatim"]
            .as_str()
            .is_some_and(|s| s.contains("END-MARKER-ZZZ")));
        assert!(added_json["lens"].is_array());
    }

    #[test]
    fn html_is_self_contained_and_escaped() {
        // Changed atoms carrying `<`/`>`/`&` — those fragments DO appear in the
        // pruned tree (unchanged ones are elided), so the escaping must hold.
        let d = diff_source_forms("(f x y)", "(f c-fwd-<>-a a&b)", Dialect::EmacsLisp).unwrap();
        let html = form_diff_html(&d);
        assert!(html.starts_with("<!doctype html>"), "not a full page");
        // Self-contained: no external network / asset references.
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(!html.contains("<script") && !html.contains("<link") && !html.contains(" src="));
        // Special characters in the changed fragments are escaped, never raw.
        assert!(
            html.contains("c-fwd-&lt;&gt;-a"),
            "angle brackets must be escaped"
        );
        assert!(
            !html.contains("c-fwd-<>-a"),
            "raw `<>` would break the markup"
        );
        assert!(html.contains("a&amp;b"), "the `&` must be escaped");
    }

    #[test]
    fn html_map_marks_added_removed_changed() {
        let old = "(defun keep () 1)\n(defun gone () 2)\n(defun edit () 3)\n";
        let new = "(defun keep () 1)\n(defun edit () 30)\n(defun fresh () 4)\n";
        let html = diff_html(&diff_files(old, new, Dialect::EmacsLisp));
        assert!(html.starts_with("<!doctype html>"));
        for name in ["edit", "fresh", "gone"] {
            assert!(html.contains(name), "missing unit {name} in map html");
        }
        // Summary pills reflect the counts.
        assert!(
            html.contains("1 changed") && html.contains("1 added") && html.contains("1 removed")
        );
    }

    #[test]
    fn deep_diff_unit_filter_matches_added() {
        let old = "(defun keep () 1)\n";
        let new = "(defun keep () 1)\n(defun brand-new () 42)\n";
        let only = diff_files_deep(old, new, Dialect::EmacsLisp, Some("brand-new"), false);
        assert!(only.changed.is_empty() && only.removed.is_empty());
        assert_eq!(only.added.len(), 1);
        assert_eq!(only.added[0].name, "brand-new");
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
