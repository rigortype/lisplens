//! Emacs Lisp indentation — a native Rust port of Emacs's `calculate-lisp-indent`
//! / `lisp-indent-function` (ADR-0011, ADR-0026), the first-release formatter.
//!
//! Faithful to the model Emacs uses: each line's indentation is derived from the
//! innermost containing list and the indent spec of its head symbol. Fidelity is
//! validated against Emacs itself (the tests use output captured from a real
//! Emacs buffer). Standard indent specs are bundled (harvested once from Emacs);
//! file-local `declare`/`put` specs are layered on via lispexp's harvester.
//!
//! Scope: Emacs Lisp only, whole-file reindent. Other dialects, touched-region
//! reindent (ADR-0025), and tabs are future work; output is space-indented, LF.
//!
//! Fidelity: byte-exact with Emacs on top-level and common forms (the tests use
//! Emacs-captured golden output). Known gaps remain on deeply nested specforms
//! (e.g. a long `if-let` condition) and complex macro/quoted-menu forms; the
//! formatter is always **safe** — it only rewrites leading whitespace, so it
//! never changes what a file parses to — and these gaps are closed iteratively
//! against the Emacs oracle (ADR-0026).

use lispexp::indent::{harvest_indent_specs, IndentSpec, IndentTable};
use lispexp::{parse, Datum, DatumKind, LineIndex, Options};

use crate::config::FormatConfig;

const BODY: usize = 2; // lisp-body-indent

/// Column arithmetic that accounts for reindentation already applied to earlier
/// lines: an element's output column is its offset within its line (stable under
/// reindent) plus that line's *new* indent. Alignment targets always sit on a
/// container's open line, which is processed before any line inside it, so their
/// new indent is known by the time it is needed.
struct Cols<'a> {
    index: &'a LineIndex,
    old_indent: &'a [usize],
    new_indent: &'a [usize],
}

impl Cols<'_> {
    /// The output column of `offset`.
    fn col(&self, offset: usize) -> usize {
        let (line, column) = self.index.offset_to_line_col(offset as u32);
        let l = line as usize - 1;
        (column as usize - 1) - self.old_indent[l] + self.new_indent[l]
    }

    fn line_of(&self, offset: usize) -> u32 {
        self.index.offset_to_line_col(offset as u32).0
    }
}

/// Reindent whole Emacs Lisp `source`, returning the formatted text. Leading
/// whitespace on each line is recomputed; tokens and line order are untouched,
/// so this never changes what the file parses to.
pub fn format_elisp(source: &str, config: &FormatConfig) -> String {
    let parsed = parse(source, &Options::emacs_lisp());
    let mut table = builtin_indent_table();
    table.merge(harvest_indent_specs(source));
    let index = LineIndex::new(source);
    let count = index.line_count();

    // Original leading-whitespace width of each line (byte columns).
    let old_indent: Vec<usize> = (1..=count as u32)
        .map(|n| {
            let range = index.line_range(n).unwrap();
            let content = &source[range];
            content.len() - content.trim_start().len()
        })
        .collect();
    let mut new_indent = vec![0usize; count];

    let mut lines: Vec<String> = Vec::with_capacity(count);
    for n in 1..=count as u32 {
        let range = index.line_range(n).expect("n within line_count");
        let content = &source[range.clone()];
        let trimmed = content.trim_start();
        let i = n as usize - 1;

        if trimmed.is_empty() {
            new_indent[i] = 0;
            lines.push(String::new());
        } else if in_string(&parsed.data, range.start) {
            // Inside a multi-line string: leave the line byte-identical.
            new_indent[i] = old_indent[i];
            lines.push(content.to_string());
        } else {
            let indent = {
                let cols = Cols { index: &index, old_indent: &old_indent, new_indent: &new_indent };
                match container_at(&parsed.data, range.start) {
                    Some(c) => indent_for(&cols, &table, c, range.start),
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

/// The deepest enclosing list whose span strictly contains `offset`.
fn container_at<'a, 't>(data: &'a [Datum<'t>], offset: usize) -> Option<&'a Datum<'t>> {
    for d in data {
        let (start, end) = (d.span.start as usize, d.span.end as usize);
        if start < offset && offset < end {
            return match &d.kind {
                DatumKind::List { items, .. } => Some(container_at(items, offset).unwrap_or(d)),
                DatumKind::Prefixed { inner, .. } => container_at(std::slice::from_ref(inner), offset),
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
                DatumKind::List { items, .. } => in_string(items, offset),
                DatumKind::Prefixed { inner, .. } => in_string(std::slice::from_ref(inner), offset),
                _ => false,
            };
        }
    }
    false
}

/// The indent column for a line starting at `offset` inside list `c`.
fn indent_for(cols: &Cols, table: &IndentTable, c: &Datum, offset: usize) -> usize {
    let DatumKind::List { items, .. } = &c.kind else {
        return 0;
    };
    let open_col = cols.col(c.span.start as usize);
    let normal = normal_indent(cols, c, items, open_col);
    match items.first().and_then(as_symbol).and_then(|h| table.get(h)) {
        Some(IndentSpec::Number(n)) => specform(items, offset, open_col, *n as usize, normal),
        Some(IndentSpec::Defun) => open_col + BODY,
        // A named indent function can't be run (reader-only), and any other or
        // future spec falls back to function-call alignment.
        _ => normal,
    }
}

/// Function-call alignment (`calculate-lisp-indent`'s `normal-indent`).
fn normal_indent(cols: &Cols, c: &Datum, items: &[Datum], open_col: usize) -> usize {
    let Some(first) = items.first() else {
        return open_col + 1;
    };
    // First element is a list → indent under that list.
    if matches!(first.kind, DatumKind::List { .. }) {
        return cols.col(first.span.start as usize);
    }
    // A first argument on the open-paren's line → align under it.
    if let Some(second) = items.get(1) {
        if cols.line_of(c.span.start as usize) == cols.line_of(second.span.start as usize) {
            return cols.col(second.span.start as usize);
        }
    }
    // Otherwise align under the head.
    cols.col(first.span.start as usize)
}

/// `lisp-indent-specform` for an integer spec `n`.
fn specform(items: &[Datum], offset: usize, open_col: usize, n: usize, normal: usize) -> usize {
    // Arguments (past the head) fully completed before this line.
    let k = items
        .iter()
        .skip(1)
        .filter(|it| (it.span.end as usize) <= offset)
        .count();
    if k < n {
        // A distinguished form: the 1st/2nd get 2×body, the rest align.
        if k <= 1 {
            open_col + 2 * BODY
        } else {
            normal
        }
    } else {
        // A body form.
        open_col + BODY
    }
}

fn as_symbol<'a>(d: &Datum<'a>) -> Option<&'a str> {
    match &d.kind {
        DatumKind::Symbol(s) => Some(s),
        _ => None,
    }
}

/// The standard Emacs Lisp indent specs (values captured from a real Emacs via
/// `(function-get SYM 'lisp-indent-function)`). Forms with no special spec
/// (`cond`, `and`, `cl-defun`, …) are intentionally absent — they use
/// function-call alignment.
fn builtin_indent_table() -> IndentTable {
    let mut table = IndentTable::new();
    let number = [
        // 0
        ("progn", 0), ("save-excursion", 0), ("save-restriction", 0),
        ("save-match-data", 0), ("with-temp-buffer", 0), ("with-output-to-string", 0),
        ("ignore-errors", 0), ("eval-when-compile", 0), ("eval-and-compile", 0),
        // 1
        ("when", 1), ("unless", 1), ("while", 1), ("let", 1), ("let*", 1),
        ("dolist", 1), ("dotimes", 1), ("with-current-buffer", 1),
        ("with-output-to-temp-buffer", 1), ("with-eval-after-load", 1),
        ("prog1", 1), ("unwind-protect", 1), ("catch", 1), ("ignore-error", 1),
        ("cl-eval-when", 1), ("defvar-keymap", 1),
        ("pcase", 1), ("pcase-let", 1), ("pcase-let*", 1), ("pcase-dolist", 1),
        ("pcase-exhaustive", 1), ("let-alist", 1), ("cl-defstruct", 1),
        ("cl-flet", 1), ("cl-labels", 1), ("cl-flet*", 1), ("cl-macrolet", 1),
        ("cl-symbol-macrolet", 1), ("cl-letf", 1), ("cl-letf*", 1),
        ("cl-case", 1), ("cl-typecase", 1), ("cl-etypecase", 1), ("cl-ecase", 1),
        ("cl-block", 1), ("when-let", 1), ("while-let", 1), ("when-let*", 1),
        ("and-let*", 1), ("seq-doseq", 1),
        // 2
        ("defun", 2), ("defmacro", 2), ("defsubst", 2), ("if", 2), ("prog2", 2),
        ("condition-case", 2), ("condition-case-unless-debug", 2),
        ("cl-defun", 2), ("cl-defmacro", 2), ("cl-defgeneric", 2), ("cl-deftype", 2),
        ("cl-destructuring-bind", 2), ("if-let", 2), ("if-let*", 2),
        ("named-let", 2), ("seq-let", 2), ("ert-deftest", 2),
    ];
    for (sym, n) in number {
        table.insert(sym, IndentSpec::Number(n));
    }
    for sym in [
        "defvar", "defconst", "defcustom", "defface", "defgroup", "defvar-local",
        "lambda", "cl-defmethod", "define-derived-mode", "define-minor-mode",
        "define-globalized-minor-mode",
    ] {
        table.insert(sym, IndentSpec::Defun);
    }
    table
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
