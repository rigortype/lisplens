//! Native Lisp indentation — Rust ports of the indenters Emacs bundles
//! (ADR-0011, ADR-0026). A shared driver (line loop, string/comment rules,
//! touched-region masking, [`Cols`] column arithmetic, rendering) walks the file
//! and, per code line, asks a dialect-specific *engine* for the indent column.
//!
//! Emacs ships three distinct Lisp indenters; each is one engine here (see
//! [`Engine`]): `lisp-indent-function` for Emacs Lisp (this module) and
//! `common-lisp-indent-function` for Common Lisp ([`commonlisp`]). The Emacs
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

use std::ops::Range;

use lispexp::indent::{harvest_indent_specs, IndentSpec, IndentTable};
use lispexp::{parse, Datum, DatumKind, Dialect, LineIndex, Options};

use crate::config::FormatConfig;
use crate::nameless::Nameless;

mod commonlisp;

/// Which native indent engine a dialect uses. Emacs bundles distinct Lisp
/// indenters; each engine is a Rust port of one:
///
/// - [`Engine::Elisp`] — Emacs's `lisp-indent-function` / `calculate-lisp-indent`
///   (`lisp-mode.el`). Drives Emacs Lisp and, for now, serves as the generic
///   fallback for every dialect without a dedicated engine yet.
/// - [`Engine::CommonLisp`] — Emacs's `common-lisp-indent-function`
///   (`cl-indent.el`), the richer style used for Common Lisp.
///
/// The Scheme family (`scheme-indent-function`) is future work; those dialects
/// fall back to [`Engine::Elisp`] until their engine lands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Engine {
    Elisp,
    CommonLisp,
}

/// The indent engine for `dialect`. `CommonLisp` uses the CL engine; everything
/// else (Emacs Lisp plus the not-yet-specialised dialects) uses the Emacs Lisp
/// engine as the generic fallback.
fn engine_for(dialect: Dialect) -> Engine {
    match dialect {
        Dialect::CommonLisp => Engine::CommonLisp,
        _ => Engine::Elisp,
    }
}

/// Whether `dialect` has a *faithful* native engine (one whose fidelity is
/// validated against Emacs), as opposed to riding the generic Emacs Lisp
/// fallback. Only these dialects are auto-formatted on Structural edit — the
/// generic fallback would risk mis-reflowing a dialect it does not model
/// (e.g. Clojure), so those files are reindented only on an explicit `format`.
/// The Scheme family joins this set once its engine lands.
#[must_use]
pub fn has_native_engine(dialect: Dialect) -> bool {
    matches!(dialect, Dialect::EmacsLisp | Dialect::CommonLisp)
}

/// Column arithmetic that accounts for reindentation already applied to earlier
/// lines: an element's output column is its offset within its line (stable under
/// reindent) plus that line's *new* indent. Alignment targets always sit on a
/// container's open line, which is processed before any line inside it, so their
/// new indent is known by the time it is needed.
pub(super) struct Cols<'a> {
    index: &'a LineIndex,
    old_indent: &'a [usize],
    new_indent: &'a [usize],
    /// Per line, the `(offset, columns_saved)` of each Nameless-composed prefix
    /// on it (ADR-0030); empty when Nameless emulation is off.
    savings: &'a [Vec<(u32, usize)>],
}

impl Cols<'_> {
    /// The output column of `offset`, in displayed columns: Nameless-composed
    /// prefixes beginning earlier on the line count as their shorter glyph.
    pub(super) fn col(&self, offset: usize) -> usize {
        let (line, column) = self.index.offset_to_line_col(offset as u32);
        let l = line as usize - 1;
        let raw = (column as usize - 1) - self.old_indent[l] + self.new_indent[l];
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

/// Reindent whole `source` for `dialect`, returning the formatted text. The
/// engine is chosen by [`engine_for`]; leading whitespace on each line is
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
    let engine = engine_for(dialect);
    let parsed = parse(source, &Options::for_dialect(dialect));
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

    // Original leading-whitespace width of each line (byte columns).
    let old_indent: Vec<usize> = (1..=count as u32)
        .map(|n| {
            let range = index.line_range(n).unwrap();
            let content = &source[range];
            content.len() - content.trim_start().len()
        })
        .collect();
    let mut new_indent = vec![0usize; count];

    // For a touched-region reindent, which lines to rewrite. `None` means
    // reindent every line (whole-file format).
    let touched_mask = touched.map(|t| touched_line_mask(&parsed.data, &index, count, t));

    let mut lines: Vec<String> = Vec::with_capacity(count);
    for n in 1..=count as u32 {
        let range = index.line_range(n).expect("n within line_count");
        let content = &source[range.clone()];
        let trimmed = content.trim_start();
        let i = n as usize - 1;

        // Outside a touched form: keep the line byte-identical, and record its
        // existing indent so any (same-form) reference stays correct.
        if touched_mask.as_ref().is_some_and(|m| !m[i]) {
            new_indent[i] = old_indent[i];
            lines.push(content.to_string());
            continue;
        }

        if trimmed.is_empty() {
            new_indent[i] = 0;
            lines.push(String::new());
        } else if in_string(&parsed.data, range.start) {
            // Inside a multi-line string: leave the line byte-identical.
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
                    index: &index,
                    old_indent: &old_indent,
                    new_indent: &new_indent,
                    savings: &savings,
                };
                match container_at(&parsed.data, range.start) {
                    Some(c) => match engine {
                        Engine::Elisp => {
                            indent_for(&cols, &table, c, range.start, config.body_indent)
                        }
                        Engine::CommonLisp => {
                            commonlisp::indent(&cols, source, &parsed.data, c, range.start, config)
                        }
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
                DatumKind::Prefixed { inner, .. } => {
                    container_at(std::slice::from_ref(inner), offset)
                }
                _ => None,
            };
        }
    }
    None
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
                DatumKind::Prefixed { inner, .. } => in_string(std::slice::from_ref(inner), offset),
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
    // `whitespace-after-open-paren` case.
    if !head_is_symbol_like(first) || first.span.start as usize > c.span.start as usize + 1 {
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
fn specform(
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
        // A body form. Emacs aligns body forms under the first body form; when
        // that form began on an earlier line, indent under it, otherwise this
        // line is the first body form and indents one body step past the open
        // paren. (`(progn (a)` puts later body under `(a)`, not at open_col+2.)
        match items.get(n + 1) {
            Some(fb) if cols.line_of(fb.span.start as usize) < cols.line_of(offset) => {
                cols.col(fb.span.start as usize)
            }
            _ => open_col + body,
        }
    }
}

/// Whether a list's head reads as a symbol for indentation (Emacs's
/// `\\sw\\|\\s_` test on the first character after any reader prefix). Symbols,
/// keywords, numbers and char literals qualify; strings and lists do not. A
/// prefixed form (`'x`, `,x`, `` `x ``, `#'x`) defers to what it wraps — so
/// `,sym` is a call head but `,(list)` and `'(list)` are data.
pub(super) fn head_is_symbol_like(d: &Datum) -> bool {
    match &d.kind {
        DatumKind::Symbol(_)
        | DatumKind::Keyword(_)
        | DatumKind::Number(_)
        | DatumKind::Char(_) => true,
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
}
