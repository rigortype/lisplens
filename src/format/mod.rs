//! Native Lisp indentation — Rust ports of the indenters Emacs bundles
//! (ADR-0011, ADR-0026). A shared driver (line loop, string/comment rules,
//! touched-region masking, `Cols` column arithmetic, rendering) walks the file
//! and, per code line, asks a dialect-specific *engine* for the indent column.
//!
//! Emacs ships three distinct Lisp indenters; each is one engine here (see
//! `Engine`): `lisp-indent-function` for Emacs Lisp (this module) and
//! `common-lisp-indent-function` for Common Lisp (`commonlisp`). The Emacs
//! Lisp engine also serves as the generic fallback for dialects without a
//! dedicated engine yet.
//!
//! The Emacs Lisp engine is faithful to the model Emacs uses: each line's
//! indentation is derived from the innermost containing list and the indent spec
//! of its head symbol. Fidelity is validated against Emacs itself (the tests use
//! output captured from a real Emacs buffer). Standard indent specs are bundled
//! (harvested once from Emacs); file-local `declare`/`put` specs are layered on
//! via lispexp's harvester.
//!
//! Every engine is always **safe** — it only rewrites leading whitespace, so it
//! never changes what a file parses to — and fidelity gaps are closed
//! iteratively against the Emacs oracle (ADR-0026).

use std::collections::HashSet;
use std::ops::Range;

use lispexp::indent::{harvest_indent_specs, IndentSpec, IndentTable};
use lispexp::{lex, parse, Datum, DatumKind, Dialect, LineIndex, Options, TokenKind};

use crate::config::FormatConfig;
use crate::nameless::Nameless;

mod clojure;
mod commonlisp;
mod scheme;

/// Which native indent engine a dialect uses. Emacs bundles distinct Lisp
/// indenters; each engine is a Rust port of one:
///
/// - [`Engine::Elisp`] — Emacs's `lisp-indent-function` / `calculate-lisp-indent`
///   (`lisp-mode.el`). Drives Emacs Lisp and, for now, serves as the generic
///   fallback for every dialect without a dedicated engine yet.
/// - [`Engine::CommonLisp`] — Emacs's `common-lisp-indent-function`
///   (`cl-indent.el`), the richer style used for Common Lisp.
/// - [`Engine::Scheme`] — Emacs's `scheme-indent-function` (`scheme.el`), the
///   Scheme family's style — the Emacs Lisp algorithm with a Scheme-specific
///   spec table and a named `let` method.
///
/// Dialects with no Emacs indenter (Clojure, Fennel, Janet, Hy, LFE, …) still
/// fall back to [`Engine::Elisp`], the generic engine.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Engine {
    Elisp,
    CommonLisp,
    Scheme,
    Clojure,
}

/// The indent engine for `dialect`. `CommonLisp` uses the CL engine; the whole
/// Scheme family (Scheme, Guile, Racket, Gauche, Mosh, Gambit, and the
/// permissive superset) uses the Scheme engine; everything else (Emacs Lisp plus
/// the dialects Emacs has no indenter for) uses the Emacs Lisp engine as the
/// generic fallback.
fn engine_for(dialect: Dialect, config: &FormatConfig) -> Engine {
    match dialect {
        Dialect::CommonLisp => Engine::CommonLisp,
        Dialect::Scheme
        | Dialect::Guile
        | Dialect::Racket
        | Dialect::Gauche
        | Dialect::Mosh
        | Dialect::Gambit
        | Dialect::SchemeSuperset => Engine::Scheme,
        // Phel shares the cljfmt `:inner`/`:block` model with Clojure (ADR-0041);
        // Fennel/Janet/Hy/LFE are "standard Lisp body+2" dialects reusing the same
        // engine with a per-dialect special-form table (ADR-0043) — Fennel/Janet from
        // their formatters (`fnlfmt`, `spork/fmt`), Hy/LFE induced from their corpora.
        // Each selects its own indent table inside the engine by dialect.
        Dialect::Clojure
        | Dialect::Phel
        | Dialect::Fennel
        | Dialect::Janet
        | Dialect::Hy
        | Dialect::Lfe => Engine::Clojure,
        // ISLisp rides the same engine with a corpus-induced table (ADR-0042), but
        // only for the **opt-in** EISL style; plain ISLisp uses the generic fallback,
        // since `open + 4` / align-under-arg-0 is one community's convention.
        Dialect::Islisp if config.islisp_eisl => Engine::Clojure,
        _ => Engine::Elisp,
    }
}

/// Whether `dialect` has a *faithful* native engine (one whose fidelity is
/// validated against Emacs), as opposed to riding the generic Emacs Lisp
/// fallback. Only these dialects are auto-formatted on Structural edit — the
/// generic fallback would risk mis-reflowing a dialect it does not model
/// (e.g. Clojure), so those files are reindented only on an explicit `format`.
/// The Scheme family (`scheme-indent-function`) is in this set; the remaining
/// dialects Emacs has no indenter for still ride the generic fallback.
#[must_use]
pub fn has_native_engine(dialect: Dialect) -> bool {
    matches!(
        dialect,
        Dialect::EmacsLisp
            | Dialect::CommonLisp
            | Dialect::Scheme
            | Dialect::Guile
            | Dialect::Racket
            | Dialect::Gauche
            | Dialect::Mosh
            | Dialect::Gambit
            | Dialect::SchemeSuperset
            | Dialect::Clojure
            | Dialect::Phel
            | Dialect::Fennel
            | Dialect::Janet
            | Dialect::Hy
            | Dialect::Lfe
    )
    // ISLisp is deliberately absent: its EISL engine is an opt-in explicit-`format`
    // style (ADR-0042), and ISLisp is not extension-detected, so structural edits
    // leave it byte-identical rather than auto-reflowing via the generic fallback.
}

/// Column arithmetic that accounts for reindentation already applied to earlier
/// lines: an element's output column is its offset within its line (stable under
/// reindent) plus that line's *new* indent. Alignment targets always sit on a
/// container's open line, which is processed before any line inside it, so their
/// new indent is known by the time it is needed.
pub(super) struct Cols<'a> {
    source: &'a str,
    index: &'a LineIndex,
    old_indent: &'a [usize],
    new_indent: &'a [usize],
    /// Per line, the `(offset, columns_saved)` of each Nameless-composed prefix
    /// on it (ADR-0030); empty when Nameless emulation is off.
    savings: &'a [Vec<(u32, usize)>],
}

impl Cols<'_> {
    /// The output column of `offset`, in **displayed** columns — matching Emacs's
    /// `current-column`. The line content between the end of its original indent
    /// and `offset` is measured by East Asian Width (`unicode-width`), not by
    /// UTF-8 byte length, so a wide/multi-byte glyph (`漢` = 2, `λ` = 1, …)
    /// advances the column as Emacs would rather than by its byte count. New
    /// indent is added and Nameless-composed prefixes beginning earlier on the
    /// line count as their shorter glyph.
    ///
    /// This measures the content slice with `unicode-width` on every call, even
    /// for pure-ASCII lines. An ASCII byte-length fast path (per-line, and a
    /// whole-file `is_ascii` shortcut) was tried and **reverted**: on a 620 KB
    /// file it saved ~0.1–0.3 ms/format (≈1–3 % of the indent pass, ≪1 % of a
    /// format), because `unicode-width` already fast-handles ASCII and the indent
    /// pass is dominated by tree traversal, not width. Not worth the state. See
    /// `docs/notes/20260704-formatter-width-perf.md`.
    pub(super) fn col(&self, offset: usize) -> usize {
        let (line, column) = self.index.offset_to_line_col(offset as u32);
        let l = line as usize - 1;
        // The content on this line from the end of its original indent to
        // `offset`; its display width plus the new indent is the output column.
        let line_start = offset - (column as usize - 1);
        let content = &self.source[line_start + self.old_indent[l]..offset];
        let raw = self.new_indent[l] + display_width(content);
        let saved: usize = self.savings[l]
            .iter()
            .filter(|(o, _)| (*o as usize) < offset)
            .map(|(_, s)| *s)
            .sum();
        raw - saved
    }

    pub(super) fn line_of(&self, offset: usize) -> u32 {
        self.index.offset_to_line_col(offset as u32).0
    }
}

/// The displayed width of `s` in columns, by East Asian Width — Emacs's
/// `current-column` for the printable glyphs that appear in code (ambiguous-width
/// characters, like `λ`/`☆`, count as 1, matching Emacs's default).
fn display_width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// `content` without the leading whitespace the reindenter is allowed to rewrite.
///
/// A page break (`^L`, U+000C) is *content*, not indentation: Emacs's
/// `indent-line-to` deletes only horizontal whitespace, so a reindent pushes a
/// `^L` to the line's new column rather than removing it, and everything from the
/// `^L` onward is kept verbatim. `char::is_whitespace` counts `^L` as whitespace,
/// so plain `str::trim_start` would leave a `^L`-only line looking empty and blank
/// it out — silently deleting the page breaks that separate the `;; Variables:` /
/// `;; Functions:` sections of an Emacs Lisp file.
///
/// Stopping at the `^L` also keeps the comment rules below honest: Emacs indents
/// `^L; foo` as code (column 2 inside a form), not to `comment-column`, because the
/// line's first character is the page break and not the `;`.
fn trim_indent(content: &str) -> &str {
    content.trim_start_matches(|c: char| c.is_whitespace() && c != PAGE_BREAK)
}

/// The page-break character (`^L`, U+000C) — see [`trim_indent`].
const PAGE_BREAK: char = '\u{0c}';

/// Reindent whole `source` for `dialect`, returning the formatted text. The
/// engine is chosen by `engine_for`; leading whitespace on each line is
/// recomputed while tokens and line order are untouched, so this never changes
/// what the file parses to.
pub fn format(source: &str, config: &FormatConfig, dialect: Dialect) -> String {
    format_impl(source, config, dialect, None, None)
}

/// Reindent whole Emacs Lisp `source`, returning the formatted text. Leading
/// whitespace on each line is recomputed; tokens and line order are untouched,
/// so this never changes what the file parses to.
pub fn format_elisp(source: &str, config: &FormatConfig) -> String {
    format_impl(source, config, Dialect::EmacsLisp, None, None)
}

/// Like [`format_elisp`], but measuring columns as they display under Nameless
/// (ADR-0030) — used when the caller opts into Nameless emulation for a file.
pub fn format_elisp_nameless(source: &str, config: &FormatConfig, nameless: &Nameless) -> String {
    format_impl(source, config, Dialect::EmacsLisp, Some(nameless), None)
}

/// Which lines a touched-region reindent rewrites (ADR-0025/0028): the
/// `expand` ranges pull in the whole enclosing top-level form (auto-format on
/// edit), while the `exact` ranges rewrite only the lines they cover
/// (`format`-by-anchor of one, possibly nested, form). Everything else stays
/// byte-identical.
#[derive(Clone, Copy, Default)]
pub struct Touched<'a> {
    pub expand: &'a [Range<usize>],
    pub exact: &'a [Range<usize>],
}

/// Auto-format reindent: reindent the whole top-level form(s) overlapping any of
/// `ranges`, using `dialect`'s engine. Used for the touched region of a
/// Structural edit.
pub fn reindent_range(
    source: &str,
    config: &FormatConfig,
    dialect: Dialect,
    ranges: &[Range<usize>],
) -> String {
    reindent(
        source,
        config,
        dialect,
        None,
        Touched {
            expand: ranges,
            exact: &[],
        },
    )
}

/// Block reindent: reindent exactly the lines of `block` (one form, possibly
/// nested), in full file context — the explicit `format`-by-anchor path.
pub fn reindent_block(
    source: &str,
    config: &FormatConfig,
    dialect: Dialect,
    block: Range<usize>,
) -> String {
    reindent(
        source,
        config,
        dialect,
        None,
        Touched {
            expand: &[],
            exact: std::slice::from_ref(&block),
        },
    )
}

/// The general touched-region reindent (see [`Touched`]) for `dialect`,
/// optionally measuring columns under Nameless (ADR-0030) so an edit to a
/// Nameless file keeps its composed-prefix alignment.
pub fn reindent(
    source: &str,
    config: &FormatConfig,
    dialect: Dialect,
    nameless: Option<&Nameless>,
    touched: Touched,
) -> String {
    format_impl(source, config, dialect, nameless, Some(touched))
}

fn format_impl(
    source: &str,
    config: &FormatConfig,
    dialect: Dialect,
    nameless: Option<&Nameless>,
    touched: Option<Touched>,
) -> String {
    let engine = engine_for(dialect, config);
    // Keep `#_` / `#;` discarded forms in the tree (as `Prefixed { Discard, … }`)
    // so lines *inside* a multi-line discard indent against the discarded form,
    // not its enclosing container — the reindenter is a round-trip consumer, and
    // this matches cljfmt (which keeps every node). `Options` is `#[non_exhaustive]`.
    let mut opts = Options::for_dialect(dialect);
    opts.keep_discarded = true;
    let parsed = parse(source, &opts);
    // The Emacs Lisp engine (also the generic fallback) uses the bundled elisp
    // indent table plus the file's own harvested `declare`/`put` specs; the
    // Common Lisp engine carries its own standard table (see [`commonlisp`]).
    let mut table = lispexp_emacs::indent::bundled_table(Dialect::EmacsLisp);
    table.merge(harvest_indent_specs(source));
    let index = LineIndex::new(source);
    let count = index.line_count();

    // Per line, where Nameless composes a prefix and by how many columns.
    let mut savings: Vec<Vec<(u32, usize)>> = vec![Vec::new(); count];
    if let Some(nl) = nameless {
        collect_savings(&parsed.data, &index, nl, &mut savings);
    }

    // Original leading-whitespace width (byte columns) of each line.
    let old_indent: Vec<usize> = (1..=count as u32)
        .map(|n| {
            let content = &source[index.line_range(n).unwrap()];
            content.len() - trim_indent(content).len()
        })
        .collect();
    let mut new_indent = vec![0usize; count];

    // For a touched-region reindent, which lines to rewrite. `None` means
    // reindent every line (whole-file format).
    let touched_mask = touched.map(|t| touched_line_mask(&parsed.data, &index, count, t));

    // Lines whose first token is a line comment — the Clojure engine leaves these
    // exactly as written (cljfmt / `phel format` never reindent a comment-only
    // line). Taken from the lexer trivia, which classifies both `;` (Clojure/…)
    // and `#` (Phel) line comments and disambiguates `#(` / `#{` (ADR feedback
    // 0007). Only built for that engine; the Emacs family keeps its own rule.
    let preserve_comment_line = matches!(engine, Engine::Clojure)
        .then(|| comment_only_lines(source, &index, &opts))
        .unwrap_or_default();

    let mut lines: Vec<String> = Vec::with_capacity(count);
    for n in 1..=count as u32 {
        let range = index.line_range(n).expect("n within line_count");
        let content = &source[range.clone()];
        let trimmed = trim_indent(content);
        let i = n as usize - 1;

        // Outside a touched form: keep the line byte-identical, and record its
        // existing indent so any (same-form) reference stays correct.
        if touched_mask.as_ref().is_some_and(|m| !m[i]) {
            new_indent[i] = old_indent[i];
            lines.push(content.to_string());
            continue;
        }

        if in_string(&parsed.data, range.start) {
            // Inside a multi-line string: leave the line byte-identical. This is
            // tested *before* the blank-line case on purpose — whitespace inside a
            // string literal is part of the string's value, so blanking a
            // whitespace-only line there would rewrite the data (a docstring's
            // `"a\n   \n  b"` would silently become `"a\n\n  b"`), which a reindent
            // must never do.
            new_indent[i] = old_indent[i];
            lines.push(content.to_string());
        } else if trimmed.is_empty() {
            new_indent[i] = 0;
            lines.push(String::new());
        } else if preserve_comment_line.contains(&n) {
            // Clojure/Phel (and the induced-table dialects on this engine) never
            // reindent a comment-only line — `cljfmt` and `phel format` leave its
            // column exactly as written, whatever it is. Only the Emacs-family
            // engines apply Emacs's `;`/`;;`/`;;;` rule below.
            new_indent[i] = old_indent[i];
            lines.push(content.to_string());
        } else if trimmed.starts_with(";;;") {
            // Emacs's `lisp-indent-line` never reindents a `;;;` comment line
            // (three comment-start chars) — its column is left as written.
            new_indent[i] = old_indent[i];
            lines.push(content.to_string());
        } else if trimmed.starts_with(';') && !trimmed.starts_with(";;") {
            // A lone `;` own-line comment goes to `comment-column`, always
            // (Emacs `indent-for-comment`) — independent of nesting or any
            // earlier column. `;;` comments fall through to the code path.
            new_indent[i] = config.comment_column;
            lines.push(format!(
                "{}{trimmed}",
                render_indent(config.comment_column, config)
            ));
        } else {
            let indent = {
                let cols = Cols {
                    source,
                    index: &index,
                    old_indent: &old_indent,
                    new_indent: &new_indent,
                    savings: &savings,
                };
                match container_at(&parsed.data, range.start) {
                    Some(c) => match engine {
                        Engine::Elisp => {
                            // `cl-flet`/`cl-labels` binding bodies indent as local
                            // defuns, which needs the line's ancestor forms, not
                            // just its innermost container; check that first.
                            let mut path = Vec::new();
                            container_path(&parsed.data, range.start, &mut path);
                            cl_flet_binding_body_indent(
                                &cols,
                                &path,
                                range.start,
                                config.body_indent,
                            )
                            .unwrap_or_else(|| {
                                indent_for(&cols, &table, c, range.start, config.body_indent)
                            })
                        }
                        Engine::CommonLisp => {
                            commonlisp::indent(&cols, source, &parsed.data, c, range.start, config)
                        }
                        Engine::Scheme => scheme::indent(&cols, c, range.start, config.body_indent),
                        Engine::Clojure => clojure::indent(
                            &cols,
                            &parsed.data,
                            c,
                            range.start,
                            // EISL's body indent is edlis's fixed `paren + 4`
                            // (ADR-0042), not the Lisp default 2 that Clojure/Phel use.
                            // Only the opt-in EISL style reaches this engine for ISLisp.
                            if dialect == Dialect::Islisp && config.islisp_eisl {
                                4
                            } else {
                                config.body_indent
                            },
                            config.clojure_fixed_indent,
                            dialect,
                        ),
                    },
                    None => 0,
                }
            };
            new_indent[i] = indent;
            lines.push(format!("{}{trimmed}", render_indent(indent, config)));
        }
    }
    lines.join("\n")
}

/// The 1-based line numbers whose first non-whitespace token is a line comment
/// (`;` in Clojure/…, `#` in Phel) — comment-only lines. Derived from the lexer
/// trivia (`lex`), which correctly distinguishes a `#` comment from a `#(`/`#{`
/// dispatch. A line with code before its comment is *not* included (its comment
/// is trailing, which the formatter leaves alone anyway).
fn comment_only_lines(source: &str, index: &LineIndex, opts: &Options) -> HashSet<u32> {
    let mut set = HashSet::new();
    for token in lex(source, opts) {
        if matches!(token.kind, TokenKind::LineComment) {
            let start = token.span.start;
            let (line, _) = index.offset_to_line_col(start);
            if let Some(range) = index.line_range(line) {
                if source[range.start..start as usize].trim().is_empty() {
                    set.insert(line);
                }
            }
        }
    }
    set
}

/// Render an indent of `col` columns as spaces, or as tabs plus trailing spaces
/// when `indent-tabs-mode` is on (ADR-0029).
fn render_indent(col: usize, config: &FormatConfig) -> String {
    if config.indent_tabs && config.tab_width > 0 {
        let tabs = col / config.tab_width;
        let spaces = col % config.tab_width;
        format!("{}{}", "\t".repeat(tabs), " ".repeat(spaces))
    } else {
        " ".repeat(col)
    }
}

/// The lines a touched-region reindent rewrites (ADR-0025/0028). `expand` ranges
/// mark every line of each enclosing top-level form (so a form is reindented in
/// full and stays self-consistent); `exact` ranges mark only the lines they
/// span (one form by anchor).
fn touched_line_mask(
    data: &[Datum],
    index: &LineIndex,
    count: usize,
    touched: Touched,
) -> Vec<bool> {
    let mut mask = vec![false; count];
    let mut mark = |from: usize, to: usize| {
        let l0 = index.offset_to_line_col(from as u32).0 as usize - 1;
        let l1 = index.offset_to_line_col(to.saturating_sub(1) as u32).0 as usize - 1;
        for m in mask
            .iter_mut()
            .take(l1.min(count.saturating_sub(1)) + 1)
            .skip(l0)
        {
            *m = true;
        }
    };
    for d in data {
        let (fs, fe) = (d.span.start as usize, d.span.end as usize);
        if touched.expand.iter().any(|r| r.start < fe && fs <= r.end) {
            mark(fs, fe);
        }
    }
    for r in touched.exact {
        mark(r.start, r.end);
    }
    mask
}

/// Record, per line, each Nameless-composed symbol's start offset and the
/// columns its prefix collapses by (ADR-0030).
fn collect_savings(
    data: &[Datum],
    index: &LineIndex,
    nl: &Nameless,
    out: &mut [Vec<(u32, usize)>],
) {
    for d in data {
        match &d.kind {
            DatumKind::Symbol(s) => {
                let save = nl.saving(s);
                if save > 0 {
                    let line = index.offset_to_line_col(d.span.start).0 as usize - 1;
                    out[line].push((d.span.start, save));
                }
            }
            DatumKind::List { items, tail, .. } => {
                collect_savings(items, index, nl, out);
                if let Some(t) = tail {
                    collect_savings(std::slice::from_ref(t), index, nl, out);
                }
            }
            DatumKind::Prefixed { inner, .. } => {
                collect_savings(std::slice::from_ref(inner), index, nl, out)
            }
            _ => {}
        }
    }
}

/// The deepest enclosing list whose span strictly contains `offset`.
pub(super) fn container_at<'a, 't>(data: &'a [Datum<'t>], offset: usize) -> Option<&'a Datum<'t>> {
    for d in data {
        let (start, end) = (d.span.start as usize, d.span.end as usize);
        if start < offset && offset < end {
            return match &d.kind {
                DatumKind::List { items, tail, .. } => {
                    if let Some(inner) = container_at(items, offset) {
                        return Some(inner);
                    }
                    // A dotted tail that is itself a list — `(a . (b c))`, as
                    // Emacs reads `(a b c)` — opens its own containing sexp.
                    if let Some(t) = tail {
                        if (t.span.start as usize) < offset && offset < (t.span.end as usize) {
                            return Some(
                                container_at(std::slice::from_ref(t), offset).unwrap_or(d),
                            );
                        }
                    }
                    Some(d)
                }
                DatumKind::Prefixed { inner, arg, .. } => {
                    // Metadata `^{…} form` (Clojure) puts the metadata map in `arg`;
                    // a line inside it is contained there, not in `inner` (the form
                    // the metadata applies to). Descend whichever holds `offset`.
                    if let Some(a) = arg {
                        if (a.span.start as usize) < offset && offset < (a.span.end as usize) {
                            return container_at(std::slice::from_ref(a), offset);
                        }
                    }
                    container_at(std::slice::from_ref(inner), offset)
                }
                // A `#(…)`/`#u8(…)` reader-macro form wraps a list; its inner
                // list is a containing sexp (Emacs indents a vector's elements
                // against its own open paren, as data). Fall back to the inner
                // list itself when nothing deeper contains `offset`.
                DatumKind::HashLiteral {
                    inner: Some(inner), ..
                } => container_at(std::slice::from_ref(inner), offset).or({
                    if (inner.span.start as usize) < offset && offset < (inner.span.end as usize) {
                        Some(inner)
                    } else {
                        None
                    }
                }),
                _ => None,
            };
        }
    }
    None
}

/// The chain of enclosing lists whose spans strictly contain `offset`, outermost
/// first, so `out.last()` is the same innermost list `container_at` returns. Only
/// `List` levels are recorded (the units that anchor indentation); `Prefixed` /
/// reader-macro wrappers are descended through without being pushed, matching how
/// `container_at` treats them. Used to see a line's *ancestor* forms, which the
/// `cl-flet` binding-body rule needs and the innermost container alone can't give.
pub(super) fn container_path<'a, 't>(
    data: &'a [Datum<'t>],
    offset: usize,
    out: &mut Vec<&'a Datum<'t>>,
) {
    for d in data {
        let (start, end) = (d.span.start as usize, d.span.end as usize);
        if start < offset && offset < end {
            match &d.kind {
                DatumKind::List { items, tail, .. } => {
                    out.push(d);
                    container_path(items, offset, out);
                    if let Some(t) = tail {
                        container_path(std::slice::from_ref(t), offset, out);
                    }
                }
                DatumKind::Prefixed { inner, arg, .. } => {
                    if let Some(a) = arg {
                        container_path(std::slice::from_ref(a), offset, out);
                    }
                    container_path(std::slice::from_ref(inner), offset, out);
                }
                DatumKind::HashLiteral {
                    inner: Some(inner), ..
                } => container_path(std::slice::from_ref(inner), offset, out),
                _ => {}
            }
            return;
        }
    }
}

/// Emacs indents each `(name ARGLIST body…)` binding of `cl-flet` / `cl-flet*` /
/// `cl-labels` / `cl-macrolet` as a *local defun*: every continuation line past
/// `name` — a body form, or an `ARGLIST` written on its own line — sits at the
/// binding's own open-paren column plus `lisp-body-indent`, not aligned under
/// `name`/`ARGLIST` the way an ordinary call's arguments are. Returns that column
/// when `offset` begins such a line; `None` otherwise. A line *inside* a multi-line
/// `ARGLIST` is excluded automatically: its innermost container is the arglist, so
/// `binding` is not `path`'s last entry and the head/binding-list checks below miss
/// — that line keeps ordinary list alignment, as Emacs does.
///
/// Restricted to exactly those four heads: `cl-symbol-macrolet` also has
/// indent-function 1 but its bindings are `(name value)` data pairs, not defuns.
fn cl_flet_binding_body_indent(
    cols: &Cols,
    path: &[&Datum],
    offset: usize,
    body: usize,
) -> Option<usize> {
    let n = path.len();
    if n < 3 {
        return None;
    }
    let form = path[n - 3]; // the `(cl-flet ((…)) …)` form
    let binding_list = path[n - 2]; // its list of bindings
    let binding = path[n - 1]; // the `(name ARGLIST body…)` being indented
    let DatumKind::List {
        items: form_items, ..
    } = &form.kind
    else {
        return None;
    };
    match form_items.first().and_then(as_symbol) {
        Some("cl-flet" | "cl-flet*" | "cl-labels" | "cl-macrolet") => {}
        _ => return None,
    }
    // `binding_list` must be the form's distinguished argument (its binding list),
    // not one of the body forms that follow it — those are ordinary calls.
    if form_items.get(1).map(|b| b.span.start) != Some(binding_list.span.start) {
        return None;
    }
    // The binding is `(name ARGLIST body…)` and Emacs treats the whole thing as a
    // local defun: every continuation line past the `name` — an own-line `ARGLIST`
    // as much as a body form — goes to the binding's open-paren column plus
    // `lisp-body-indent`. (A line *inside* a multi-line `ARGLIST` has the arglist,
    // not the binding, as its innermost container, so `binding` isn't `path`'s last
    // entry there and this function returns earlier — that line keeps ordinary
    // list alignment, as Emacs does.)
    let DatumKind::List {
        items: bind_items, ..
    } = &binding.kind
    else {
        return None;
    };
    let name = bind_items.first()?;
    as_symbol(name)?; // a binding's car is the function name; a real symbol
    if offset <= name.span.end as usize {
        return None;
    }
    Some(cols.col(binding.span.start as usize) + body)
}

/// Whether `offset` falls strictly inside a string literal.
fn in_string(data: &[Datum], offset: usize) -> bool {
    for d in data {
        let (start, end) = (d.span.start as usize, d.span.end as usize);
        if start < offset && offset < end {
            return match &d.kind {
                DatumKind::Str(_) => true,
                DatumKind::List { items, tail, .. } => {
                    in_string(items, offset)
                        || tail
                            .as_ref()
                            .is_some_and(|t| in_string(std::slice::from_ref(t), offset))
                }
                DatumKind::Prefixed { inner, arg, .. } => {
                    // Metadata `^{…} form` puts a (possibly multi-line-string-bearing)
                    // map in `arg`; check it too, so a docstring inside metadata is
                    // recognized as string interior and left untouched.
                    in_string(std::slice::from_ref(inner), offset)
                        || arg
                            .as_ref()
                            .is_some_and(|a| in_string(std::slice::from_ref(a), offset))
                }
                DatumKind::HashLiteral {
                    inner: Some(inner), ..
                } => in_string(std::slice::from_ref(inner), offset),
                _ => false,
            };
        }
    }
    false
}

/// The indent column for a line starting at `offset` inside list `c`. `body` is
/// `lisp-body-indent` (columns per structural step).
fn indent_for(cols: &Cols, table: &IndentTable, c: &Datum, offset: usize, body: usize) -> usize {
    let DatumKind::List { items, .. } = &c.kind else {
        return 0;
    };
    let open_col = cols.col(c.span.start as usize);
    let normal = normal_indent(cols, c, items, open_col, offset);
    match items.first().and_then(as_symbol).and_then(|h| table.get(h)) {
        Some(IndentSpec::Number(n)) => {
            specform(cols, items, offset, open_col, *n as usize, normal, body)
        }
        Some(IndentSpec::Defun) => open_col + body,
        // A named indent function can't be run (reader-only), and any other or
        // future spec falls back to function-call alignment.
        _ => normal,
    }
}

/// Function-call alignment (`calculate-lisp-indent`'s `normal-indent`). `offset`
/// is the start of the line being indented.
pub(super) fn normal_indent(
    cols: &Cols,
    c: &Datum,
    items: &[Datum],
    open_col: usize,
    offset: usize,
) -> usize {
    let Some(first) = items.first() else {
        return open_col + 1;
    };
    // No element completed on an earlier line → Emacs's "indent-point
    // immediately follows the open paren" case: indent just past it. This also
    // avoids aligning under an element on the current (or a later) line, whose
    // new indent is not yet known — the self-reference that flattened
    // comment-led data lists to column 0.
    if cols.line_of(first.span.start as usize) >= cols.line_of(offset) {
        return open_col + 1;
    }
    // Align under the first element (not the first argument) when either the
    // head isn't a symbol-like token — a list/string/char or prefixed form, so
    // `lisp-indent-function`'s "car is not a symbol" data path applies — or
    // whitespace sits right after the open paren (`( a b`), which is Emacs's
    // `whitespace-after-open-paren` case. The whitespace test only counts a
    // *space/tab* on the open-paren's own line: when the first element is on a
    // later line the char after `(` is a newline, which Emacs does not treat as
    // `whitespace-after-open-paren` (e.g. `'#u8(` at end of line then numeric
    // elements align as a call, under the second element).
    let ws_after_open = first.span.start as usize > c.span.start as usize + 1
        && cols.line_of(first.span.start as usize) == cols.line_of(c.span.start as usize);
    if !head_is_symbol_like(first) || ws_after_open {
        return cols.col(first.span.start as usize);
    }
    // A function call whose first argument sits on the open-paren's line →
    // align the rest under that argument.
    if let Some(second) = items.get(1) {
        if cols.line_of(c.span.start as usize) == cols.line_of(second.span.start as usize) {
            return cols.col(second.span.start as usize);
        }
    } else if let Some(dot) = c.dot_span() {
        // A dotted pair with a lone car — `(a . tail)` — where Emacs's
        // text-based indenter treats the `.` as the first argument: a tail
        // continuation aligns under the `.` when it sits on the open line.
        // (`'(eval . FORM)`, the font-lock-keywords idiom.)
        if cols.line_of(dot.start as usize) == cols.line_of(c.span.start as usize) {
            return cols.col(dot.start as usize);
        }
    }
    // Otherwise align under the head.
    cols.col(first.span.start as usize)
}

/// `lisp-indent-specform` for an integer spec `n`. `body` is `lisp-body-indent`.
/// Shared with the Scheme engine — `scheme.el`'s `scheme-indent-function` calls
/// this same `lisp-indent-specform` for its integer and `scheme-let-indent`
/// specs.
pub(super) fn specform(
    cols: &Cols,
    items: &[Datum],
    offset: usize,
    open_col: usize,
    n: usize,
    normal: usize,
    body: usize,
) -> usize {
    // Arguments (past the head) fully completed before this line.
    let k = items
        .iter()
        .skip(1)
        .filter(|it| (it.span.end as usize) <= offset)
        .count();
    if k < n {
        // A distinguished form: the 1st/2nd get 2×body, the rest align.
        if k <= 1 {
            open_col + 2 * body
        } else {
            normal
        }
    } else {
        // A body form. Three sub-cases, matching Emacs:
        // - the first body form began on its *own* line (after the open paren)
        //   → align under it (`(progn` on its own line puts body under the first
        //   body form);
        // - the first body form shares the open paren's line with the head and
        //   distinguished args → it is not a line-start anchor, so this
        //   continuation falls to `normal`, which aligns under the second element
        //   (`(define-record-type name #t #t` with fields on the next line puts
        //   them under `name`, not under the first `#t`);
        // - otherwise this line *is* the first body form → one body step past the
        //   open paren.
        let open_line = cols.line_of(items[0].span.start as usize);
        match items.get(n + 1) {
            Some(fb) if cols.line_of(fb.span.start as usize) < cols.line_of(offset) => {
                if cols.line_of(fb.span.start as usize) > open_line {
                    cols.col(fb.span.start as usize)
                } else {
                    normal
                }
            }
            _ => open_col + body,
        }
    }
}

/// Whether a list's head reads as a symbol for indentation (Emacs's
/// `\\sw\\|\\s_` test on the first character after any reader prefix, reached via
/// `backward-prefix-chars`). Symbols, keywords and numbers qualify; strings and
/// lists do not. A boolean `#t`/`#f` qualifies — Emacs steps back over the `#`
/// prefix char onto the word `t`/`f`, so `'(#t #f\n #t)` aligns as a call (under
/// the second element). A char literal qualifies only when its glyph is itself a
/// word/symbol char: Emacs Lisp's `?a` does (`?` is expression-prefix syntax, so
/// point lands on the word `a`), but Scheme's `#\a` does **not** (the `#\` char
/// quote is punctuation, so the data path fires and the vector/list aligns under
/// its first element). A prefixed form (`'x`, `,x`, `` `x ``, `#'x`) defers to
/// what it wraps — so `,sym` is a call head but `,(list)` and `'(list)` are data.
pub(super) fn head_is_symbol_like(d: &Datum) -> bool {
    match &d.kind {
        DatumKind::Symbol(_)
        | DatumKind::Keyword(_)
        | DatumKind::Number(_)
        | DatumKind::Bool(_) => true,
        // `?a` (Emacs Lisp) is symbol-like; `#\a` / `\a` (Scheme, Clojure) is not.
        DatumKind::Char(s) => s.starts_with('?'),
        DatumKind::Prefixed { inner, .. } => head_is_symbol_like(inner),
        _ => false,
    }
}

pub(super) fn as_symbol<'a>(d: &Datum<'a>) -> Option<&'a str> {
    match &d.kind {
        DatumKind::Symbol(s) => Some(s),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reindenting flat input must reproduce Emacs's canonical output (captured
    /// from a real `emacs -Q --batch` buffer with `indent-tabs-mode` nil).
    #[test]
    fn matches_emacs_on_common_forms() {
        let input = "\
(defun foo (x)
(bar x)
(baz x))
(when cond
(do-thing)
(do-other))
(let ((a 1)
(b 2))
(+ a b))
(some-function arg1
arg2
arg3)
(cond
(test1 result1)
(test2 result2))
(progn
(a)
(b))
";
        let expected = "\
(defun foo (x)
  (bar x)
  (baz x))
(when cond
  (do-thing)
  (do-other))
(let ((a 1)
      (b 2))
  (+ a b))
(some-function arg1
               arg2
               arg3)
(cond
 (test1 result1)
 (test2 result2))
(progn
  (a)
  (b))
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    #[test]
    fn matches_emacs_on_def_family_and_specforms() {
        let input = "\
(cl-defun cf (a)
(body1)
(body2))
(defvar my-var
(compute))
(if test
then
else)
(condition-case err
(risky)
(error handler))
(outer (inner a
b)
c)
";
        let expected = "\
(cl-defun cf (a)
  (body1)
  (body2))
(defvar my-var
  (compute))
(if test
    then
  else)
(condition-case err
    (risky)
  (error handler))
(outer (inner a
              b)
       c)
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// Under Nameless (ADR-0030) alignment is measured against the displayed,
    /// composed width: `php-` (current name, 4 chars) collapses to `:` (1) and
    /// `font-lock-` (10) to `fl:` (2), so first-argument alignment shifts left
    /// by 3 and 8 columns respectively. Golden captured from Emacs with
    /// `nameless-mode` and `nameless-current-name` = "php".
    #[test]
    fn matches_emacs_under_nameless() {
        let input = "\
(defun php-mode-foo-bar (arg)
(php-mode-some-function arg
another-arg)
(font-lock-add-keywords nil
kw))
";
        let expected = "\
(defun php-mode-foo-bar (arg)
  (php-mode-some-function arg
                       another-arg)
  (font-lock-add-keywords nil
                  kw))
";
        let nl = Nameless::for_file("php-mode.el");
        assert_eq!(
            format_elisp_nameless(input, &FormatConfig::default(), &nl),
            expected
        );
        // Off by default: without Nameless the alignment is the literal width.
        assert!(format_elisp(input, &FormatConfig::default())
            .contains("\n                          another-arg)"));
    }

    /// Data lists (non-symbol head), dotted-tail sublists, `progn`-style body
    /// forms that start on the open-paren line, and prefixed heads — golden
    /// captured from Emacs `indent-region`.
    #[test]
    fn matches_emacs_on_data_lists_and_prefixed_heads() {
        let input = "\
(defconst names
'(;; leading comment
\"a\" \"b\"
\"c\"))
(defvar styles
`((a . ((x . 1)
(y . 2)))))
(defun f ()
(progn (a)
(b)))
(defun g ()
`(,head first
,(compute)))
";
        let expected = "\
(defconst names
  '(;; leading comment
    \"a\" \"b\"
    \"c\"))
(defvar styles
  `((a . ((x . 1)
          (y . 2)))))
(defun f ()
  (progn (a)
         (b)))
(defun g ()
  `(,head first
          ,(compute)))
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// A lone `;` own-line comment aligns to `comment-column` (default 40),
    /// independent of nesting; `;;` still indents as code. Matches Emacs
    /// `indent-for-comment`; `comment_column` is configurable.
    #[test]
    fn lone_semicolon_comment_aligns_to_comment_column() {
        let input = "(defun f ()\n; margin\n;; code\n(bar))\n";
        let expected = format!(
            "(defun f ()\n{}; margin\n  ;; code\n  (bar))\n",
            " ".repeat(40)
        );
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);

        let cfg = FormatConfig {
            comment_column: 20,
            ..FormatConfig::default()
        };
        let expected20 = format!(
            "(defun f ()\n{}; margin\n  ;; code\n  (bar))\n",
            " ".repeat(20)
        );
        assert_eq!(format_elisp(input, &cfg), expected20);
    }

    /// A touched-region reindent rewrites only the top-level forms an edit
    /// overlapped; other forms stay byte-identical, even if misindented.
    #[test]
    fn reindent_range_touches_only_overlapping_forms() {
        let source = "(defun a ()\n(x))\n(defun b ()\n(y))\n";
        let second = source.find("(defun b").unwrap();
        let out = reindent_range(
            source,
            &FormatConfig::default(),
            Dialect::EmacsLisp,
            std::slice::from_ref(&(second..second + 1)),
        );
        // Second form reindented (body at col 2); first left flat.
        assert_eq!(out, "(defun a ()\n(x))\n(defun b ()\n  (y))\n");

        // A range in the first form reindents that one instead.
        let out2 = reindent_range(
            source,
            &FormatConfig::default(),
            Dialect::EmacsLisp,
            std::slice::from_ref(&(0..1)),
        );
        assert_eq!(out2, "(defun a ()\n  (x))\n(defun b ()\n(y))\n");
    }

    /// `reindent_block` rewrites only the anchored (nested) form's lines, in
    /// full context; `reindent_range` expands to the whole top-level form. Same
    /// input, different scope.
    #[test]
    fn reindent_block_is_exact_reindent_range_expands() {
        let source = "(progn\n  (bar a\nb)\n(baz c\nd))\n";
        let bar = source.find("(bar").unwrap()..source.find("b)").unwrap() + 2;

        // Block: only `(bar a / b)` reindented; `(baz c / d)` left flat.
        let block = reindent_block(
            source,
            &FormatConfig::default(),
            Dialect::EmacsLisp,
            bar.clone(),
        );
        assert_eq!(block, "(progn\n  (bar a\n       b)\n(baz c\nd))\n");

        // Range: expands to the whole `progn`, so `baz`/`d` are fixed too.
        let range = reindent_range(
            source,
            &FormatConfig::default(),
            Dialect::EmacsLisp,
            std::slice::from_ref(&bar),
        );
        assert_eq!(range, "(progn\n  (bar a\n       b)\n  (baz c\n       d))\n");
    }

    /// `lisp-body-indent` (config `body_indent`) scales every structural step:
    /// a body form lands at `open_col + body`, a specform's 1st/2nd distinguished
    /// at `open_col + 2*body`. Golden from Emacs with `lisp-body-indent` = 4.
    #[test]
    fn body_indent_override_matches_emacs() {
        let input = "\
(defun foo (x)
(bar x))
(when c
(a))
(if test
then
else)
";
        let expected = "\
(defun foo (x)
    (bar x))
(when c
    (a))
(if test
        then
    else)
";
        let cfg = FormatConfig {
            body_indent: 4,
            ..FormatConfig::default()
        };
        assert_eq!(format_elisp(input, &cfg), expected);
    }

    /// A dotted pair with a lone car (`'(eval . FORM)`, the font-lock idiom)
    /// aligns the tail's continuation under the `.` — Emacs treats the lone dot
    /// as the first argument. Needs lispexp's `dot_span`. Golden from Emacs.
    #[test]
    fn matches_emacs_on_dotted_pair_tail() {
        let input = "\
(defvar kw
(list
'(eval .
;; comment under the dot
(list a
b))))
";
        let expected = "\
(defvar kw
  (list
   '(eval .
          ;; comment under the dot
          (list a
                b))))
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// Whitespace right after an open paren (`( a b`) makes Emacs align the
    /// continuation under the first element, not the first argument — even for a
    /// symbol head, and for a dotted tail (`( a b . ,x)`). Golden from Emacs.
    #[test]
    fn matches_emacs_on_whitespace_after_open_paren() {
        let input = "\
(defvar x
`( TIMESTAMP MULTIPLE
. ,(delq a
b)))
(defun f ()
( one two
three))
";
        let expected = "\
(defvar x
  `( TIMESTAMP MULTIPLE
     . ,(delq a
              b)))
(defun f ()
  ( one two
    three))
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// A `;;;` comment line keeps its column (Emacs never reindents one), while
    /// `;;` comments indent as code.
    #[test]
    fn triple_semicolon_comment_is_left_in_place() {
        let input = "(defun f ()\n;; two\n;;; three\n(body))\n";
        let expected = "(defun f ()\n  ;; two\n;;; three\n  (body))\n";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// A page break (`^L`) is content, not indentation: a reindent must never
    /// delete one. Emacs Lisp files separate their `;; Variables:` / `;; Functions:`
    /// sections with a `^L` on its own line, and `str::trim_start` counts `^L` as
    /// whitespace — which made a `^L`-only line look blank and get blanked out.
    /// Golden captured from Emacs `indent-region`.
    #[test]
    fn a_page_break_line_survives_a_reindent() {
        let input = "\
;;; Code:

\u{0c}
(defvar test-foo nil)

\u{0c}
(defun test-bar ()
nil)
";
        let expected = "\
;;; Code:

\u{0c}
(defvar test-foo nil)

\u{0c}
(defun test-bar ()
  nil)
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// A `^L` indents like code — Emacs's `indent-line-to` deletes only the
    /// horizontal whitespace before it, so the page break is pushed to the line's
    /// new column instead of being removed. A top-level `^L` dedents to 0; one in a
    /// body moves to the body indent. Golden captured from Emacs `indent-region`.
    #[test]
    fn a_page_break_indents_as_code() {
        let input = "(defun a ()\n  (let ((x 1))\n\u{0c}\n    (message \"in\")\n  \u{0c}\n    x))\n\n   \u{0c}\n(defun b () nil)\n";
        let expected = "(defun a ()\n  (let ((x 1))\n    \u{0c}\n    (message \"in\")\n    \u{0c}\n    x))\n\n\u{0c}\n(defun b () nil)\n";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// A line whose first character is a `^L` is code, not a comment line: Emacs
    /// sends `^L; foo` to the body indent, not to `comment-column`, because the
    /// lone-`;` rule keys on the line's *first* character. Golden captured from
    /// Emacs `indent-region`.
    #[test]
    fn a_page_break_before_a_comment_is_not_a_comment_line() {
        let input = "(defun c ()\n\u{0c};; two\n\u{0c}; lone\n  nil)\n";
        let expected = "(defun c ()\n  \u{0c};; two\n  \u{0c}; lone\n  nil)\n";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// `trim_indent` lives in the shared per-line driver, so every engine —
    /// not just Emacs Lisp — preserves a `^L` page-break line. Pin one input
    /// through each engine family (Clojure, Common Lisp, Scheme).
    #[test]
    fn a_page_break_survives_every_engine() {
        for dialect in [Dialect::Clojure, Dialect::CommonLisp, Dialect::Scheme] {
            let input = "(a\n1)\n\u{0c}\n(b\n2)\n";
            let out = format(input, &FormatConfig::default(), dialect);
            assert_eq!(
                out.matches('\u{0c}').count(),
                1,
                "{dialect:?} must keep the page break"
            );
        }
    }

    #[test]
    fn already_formatted_is_a_fixed_point() {
        let formatted = "(defun foo (x)\n  (bar x))\n";
        assert_eq!(format_elisp(formatted, &FormatConfig::default()), formatted);
    }

    #[test]
    fn a_multiline_string_is_left_untouched() {
        let src = "(defun f ()\n  \"a\n    b\")\n";
        // The string's second line must not be reindented.
        assert!(format_elisp(src, &FormatConfig::default()).contains("\n    b\""));
    }

    /// A whitespace-only line *inside* a string is part of the string's value, so
    /// the blank-line rule must not reach it: blanking it would turn the docstring
    /// `"a\n   \n  b"` into `"a\n\n  b"` — a reindent silently rewriting the data.
    /// Emacs leaves the line alone (`calculate-lisp-indent` returns nil in a
    /// string). Golden captured from Emacs `indent-region`.
    #[test]
    fn a_whitespace_only_line_inside_a_string_keeps_its_spaces() {
        let src = "(defun f ()\n  \"Line one.\n   \n  Line three.\"\n  nil)\n";
        assert_eq!(format_elisp(src, &FormatConfig::default()), src);
    }

    /// The same rule outside a string: a whitespace-only line is *not* string data,
    /// so it is normalized to empty, as Emacs's `indent-region` does.
    #[test]
    fn a_whitespace_only_line_outside_a_string_is_blanked() {
        let input = "(defun f ()\n  (a))\n   \n(defun g ()\n  (b))\n";
        let expected = "(defun f ()\n  (a))\n\n(defun g ()\n  (b))\n";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// Emacs indents each `(name ARGLIST body…)` binding of `cl-labels` / `cl-flet`
    /// / `cl-flet*` / `cl-macrolet` as a *local defun*: a body form sits at the
    /// binding's own open-paren column plus `lisp-body-indent`, not aligned under
    /// `ARGLIST` as an ordinary call's continuation would be. Goldens captured from
    /// Emacs `indent-region` (with `cl-lib`/`cl-macs` loaded).
    #[test]
    fn cl_labels_binding_body_indents_as_local_defun() {
        let input = "(cl-labels ((json-get (obj key)\n(cond\n((hash-table-p obj)\n(gethash key obj)))))\n(body))\n";
        let expected = "\
(cl-labels ((json-get (obj key)
              (cond
               ((hash-table-p obj)
                (gethash key obj)))))
  (body))
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// Multiple `cl-flet` bindings, each with its own body at binding-open + 2.
    #[test]
    fn cl_flet_multiple_bindings_each_indent_their_body() {
        let input = "(cl-flet ((f (x)\n(+ x 1))\n(g (y)\n(* y 2)))\n(f 1))\n";
        let expected = "\
(cl-flet ((f (x)
            (+ x 1))
          (g (y)
            (* y 2)))
  (f 1))
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// An empty `ARGLIST` still puts the body at binding-open + 2, and a `cl-macrolet`
    /// binding is treated the same as `cl-flet`/`cl-labels`.
    #[test]
    fn cl_flet_empty_arglist_and_cl_macrolet_body() {
        let input = "(cl-labels ((h ()\nnil))\n(h))\n\n(cl-macrolet ((m (x)\n`(+ ,x 1)))\n(m 2))\n";
        let expected = "\
(cl-labels ((h ()
              nil))
  (h))

(cl-macrolet ((m (x)
                `(+ ,x 1)))
  (m 2))
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// An `ARGLIST` written on its own line (not sharing the binding's first line)
    /// is a continuation past `name`, so Emacs sends it to binding-open + 2 just
    /// like a body form — the binding is one local defun.
    #[test]
    fn cl_labels_own_line_arglist_indents_as_body() {
        let input = "(cl-labels ((get-xrefs-in-file\n(file-locs)\n(-let [(filename . matches) file-locs]\n(list filename matches))))\n(body))\n";
        let expected = "\
(cl-labels ((get-xrefs-in-file
              (file-locs)
              (-let [(filename . matches) file-locs]
                    (list filename matches))))
  (body))
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// A multi-line `ARGLIST` keeps ordinary list alignment on its continuation
    /// (`b` under `a`), while the binding's body still goes to binding-open + 2 —
    /// the body rule keys on the innermost container being the binding itself, which
    /// the arglist line is not.
    #[test]
    fn cl_flet_multiline_arglist_keeps_list_alignment() {
        let input = "(cl-flet ((k (a\nb)\n(list a b)))\n(k 1 2))\n";
        let expected = "\
(cl-flet ((k (a
              b)
            (list a b)))
  (k 1 2))
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }

    /// Alignment is measured in **display** columns (East Asian Width), matching
    /// Emacs's `current-column`: a continuation aligns under the first argument
    /// counting each glyph by its width, not its UTF-8 byte length. `λ`/`☆`
    /// (ambiguous) are width 1, `漢` (wide) / `Ａ` (fullwidth) are width 2 — so the
    /// two blocks indent differently. Golden captured from Emacs `indent-region`.
    #[test]
    fn aligns_multibyte_heads_by_display_width() {
        let input = "\
(λλλλ arg1
arg2)
(漢漢漢漢 arg1
arg2)
(ＡＡＡＡ arg1
arg2)
(☆☆☆☆ arg1
arg2)
";
        let expected = "\
(λλλλ arg1
      arg2)
(漢漢漢漢 arg1
          arg2)
(ＡＡＡＡ arg1
          arg2)
(☆☆☆☆ arg1
      arg2)
";
        assert_eq!(format_elisp(input, &FormatConfig::default()), expected);
    }
}
