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
use crate::nameless::Nameless;

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
    /// Per line, the `(offset, columns_saved)` of each Nameless-composed prefix
    /// on it (ADR-0030); empty when Nameless emulation is off.
    savings: &'a [Vec<(u32, usize)>],
}

impl Cols<'_> {
    /// The output column of `offset`, in displayed columns: Nameless-composed
    /// prefixes beginning earlier on the line count as their shorter glyph.
    fn col(&self, offset: usize) -> usize {
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

    fn line_of(&self, offset: usize) -> u32 {
        self.index.offset_to_line_col(offset as u32).0
    }
}

/// Reindent whole Emacs Lisp `source`, returning the formatted text. Leading
/// whitespace on each line is recomputed; tokens and line order are untouched,
/// so this never changes what the file parses to.
pub fn format_elisp(source: &str, config: &FormatConfig) -> String {
    format_elisp_impl(source, config, None)
}

/// Like [`format_elisp`], but measuring columns as they display under Nameless
/// (ADR-0030) — used when the caller opts into Nameless emulation for a file.
pub fn format_elisp_nameless(source: &str, config: &FormatConfig, nameless: &Nameless) -> String {
    format_elisp_impl(source, config, Some(nameless))
}

fn format_elisp_impl(source: &str, config: &FormatConfig, nameless: Option<&Nameless>) -> String {
    let parsed = parse(source, &Options::emacs_lisp());
    let mut table = builtin_indent_table();
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
        } else if trimmed.starts_with(";;;") {
            // Emacs's `lisp-indent-line` never reindents a `;;;` comment line
            // (three comment-start chars) — its column is left as written.
            new_indent[i] = old_indent[i];
            lines.push(content.to_string());
        } else {
            let indent = {
                let cols = Cols {
                    index: &index,
                    old_indent: &old_indent,
                    new_indent: &new_indent,
                    savings: &savings,
                };
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

/// Record, per line, each Nameless-composed symbol's start offset and the
/// columns its prefix collapses by (ADR-0030).
fn collect_savings(data: &[Datum], index: &LineIndex, nl: &Nameless, out: &mut [Vec<(u32, usize)>]) {
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
fn container_at<'a, 't>(data: &'a [Datum<'t>], offset: usize) -> Option<&'a Datum<'t>> {
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
                            return Some(container_at(std::slice::from_ref(t), offset).unwrap_or(d));
                        }
                    }
                    Some(d)
                }
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

/// The indent column for a line starting at `offset` inside list `c`.
fn indent_for(cols: &Cols, table: &IndentTable, c: &Datum, offset: usize) -> usize {
    let DatumKind::List { items, .. } = &c.kind else {
        return 0;
    };
    let open_col = cols.col(c.span.start as usize);
    let normal = normal_indent(cols, c, items, open_col, offset);
    match items.first().and_then(as_symbol).and_then(|h| table.get(h)) {
        Some(IndentSpec::Number(n)) => specform(cols, items, offset, open_col, *n as usize, normal),
        Some(IndentSpec::Defun) => open_col + BODY,
        // A named indent function can't be run (reader-only), and any other or
        // future spec falls back to function-call alignment.
        _ => normal,
    }
}

/// Function-call alignment (`calculate-lisp-indent`'s `normal-indent`). `offset`
/// is the start of the line being indented.
fn normal_indent(cols: &Cols, c: &Datum, items: &[Datum], open_col: usize, offset: usize) -> usize {
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

/// `lisp-indent-specform` for an integer spec `n`.
fn specform(cols: &Cols, items: &[Datum], offset: usize, open_col: usize, n: usize, normal: usize) -> usize {
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
        // A body form. Emacs aligns body forms under the first body form; when
        // that form began on an earlier line, indent under it, otherwise this
        // line is the first body form and indents one body step past the open
        // paren. (`(progn (a)` puts later body under `(a)`, not at open_col+2.)
        match items.get(n + 1) {
            Some(fb) if cols.line_of(fb.span.start as usize) < cols.line_of(offset) => {
                cols.col(fb.span.start as usize)
            }
            _ => open_col + BODY,
        }
    }
}

/// Whether a list's head reads as a symbol for indentation (Emacs's
/// `\\sw\\|\\s_` test on the first character after any reader prefix). Symbols,
/// keywords, numbers and char literals qualify; strings and lists do not. A
/// prefixed form (`'x`, `,x`, `` `x ``, `#'x`) defers to what it wraps — so
/// `,sym` is a call head but `,(list)` and `'(list)` are data.
fn head_is_symbol_like(d: &Datum) -> bool {
    match &d.kind {
        DatumKind::Symbol(_)
        | DatumKind::Keyword(_)
        | DatumKind::Number(_)
        | DatumKind::Char(_) => true,
        DatumKind::Prefixed { inner, .. } => head_is_symbol_like(inner),
        _ => false,
    }
}

fn as_symbol<'a>(d: &Datum<'a>) -> Option<&'a str> {
    match &d.kind {
        DatumKind::Symbol(s) => Some(s),
        _ => None,
    }
}

/// The standard Emacs Lisp indent specs, captured wholesale from a real Emacs
/// (core plus cl-lib / cl-macs / pcase / subr-x / seq / let-alist / rx / map /
/// gv / cl-generic and the bundled org/transient/… that come with them) via
/// `(function-get SYM 'lisp-indent-function 'macro)`. Forms with no spec use
/// function-call alignment.
fn builtin_indent_table() -> IndentTable {
    let mut table = IndentTable::new();
    for &(sym, n) in NUMBER_SPECS {
        table.insert(sym, IndentSpec::Number(n));
    }
    for &sym in DEFUN_SPECS {
        table.insert(sym, IndentSpec::Defun);
    }
    table
}

#[rustfmt::skip]
const NUMBER_SPECS: &[(&str, u32)] = &[
    ("and-let*", 1), ("atomic-change-group", 0), ("benchmark-elapse", 0), ("benchmark-progn",
    0), ("benchmark-run", 1), ("benchmark-run-compiled", 1), ("byte-compile-maybe-guarded", 1),
    ("byte-optimize--pcase", 1), ("cal-menu-x-popup-menu", 2), ("calendar-dlet", 1),
    ("calendar-in-read-only-buffer", 1), ("catch", 1), ("cl--define-built-in-type", 2),
    ("cl-block", 1), ("cl-callf", 2), ("cl-callf2", 3), ("cl-case", 1), ("cl-defgeneric", 2),
    ("cl-define-compiler-macro", 2), ("cl-defmacro", 2), ("cl-defstruct", 1), ("cl-defsubst",
    2), ("cl-deftype", 2), ("cl-defun", 2), ("cl-destructuring-bind", 2), ("cl-do", 2),
    ("cl-do*", 2), ("cl-do-all-symbols", 1), ("cl-do-symbols", 1), ("cl-dolist", 1),
    ("cl-dotimes", 1), ("cl-ecase", 1), ("cl-etypecase", 1), ("cl-eval-when", 1), ("cl-flet",
    1), ("cl-flet*", 1), ("cl-generic-define-generalizer", 1), ("cl-iter-defun", 2),
    ("cl-labels", 1), ("cl-letf", 1), ("cl-letf*", 1), ("cl-locally", 0), ("cl-macrolet", 1),
    ("cl-multiple-value-bind", 2), ("cl-multiple-value-setq", 1), ("cl-once-only", 1),
    ("cl-progv", 2), ("cl-return-from", 1), ("cl-symbol-macrolet", 1), ("cl-the", 1),
    ("cl-typecase", 1), ("cl-with-accessors", 2), ("cl-with-gensyms", 1),
    ("combine-after-change-calls", 0), ("combine-change-calls", 2), ("comment-with-narrowing",
    2), ("condition-case", 2), ("condition-case-unless-debug", 2), ("cps--add-state", 1),
    ("custom-dirlocals-with-buffer", 0), ("debugger-env-macro", 0), ("def-edebug-elem-spec",
    1), ("def-edebug-spec", 1), ("defadvice", 2), ("define-advice", 2), ("define-generic-mode",
    1), ("define-ibuffer-filter", 2), ("define-ibuffer-op", 2), ("define-ibuffer-sorter", 1),
    ("define-icon", 2), ("defmacro", 2), ("defsubst", 2), ("deftheme", 1), ("defun", 2),
    ("defvar-keymap", 1), ("delay-mode-hooks", 0), ("dlet", 1), ("dolist", 1),
    ("dolist-with-progress-reporter", 2), ("dont-compile", 0), ("dotimes", 1),
    ("dotimes-with-progress-reporter", 2), ("easy-mmode-define-navigation", 5),
    ("easy-mmode-defmap", 1), ("easy-mmode-defsyntax", 1),
    ("eldoc--documentation-strategy-defcustom", 2), ("ert-deftest", 2),
    ("ert-font-lock-deftest", 1), ("ert-font-lock-deftest-file", 1), ("ert-info", 1),
    ("ert-with-buffer-renamed", 1), ("ert-with-buffer-selected", 1),
    ("ert-with-message-capture", 1), ("ert-with-temp-file", 1),
    ("ert-with-test-buffer-selected", 1), ("eval-after-load", 1), ("eval-and-compile", 0),
    ("eval-when-compile", 0), ("flymake--with-backend-state", 2), ("gv-define-expander", 1),
    ("gv-define-setter", 2), ("gv-letplace", 2), ("handler-bind", 1), ("ibuffer-aif", 2),
    ("ibuffer-awhen", 1), ("ibuffer-save-marks", 0), ("if", 2), ("if-let", 2), ("if-let*", 2),
    ("ignore-error", 1), ("ignore-errors", 0), ("inhibit-auto-revert", 0), ("inline", 0),
    ("inline--leteval", 1), ("inline-letevals", 1), ("iter-do", 1), ("let", 1), ("let*", 1),
    ("let-alist", 1), ("let-when-compile", 1), ("letrec", 1), ("macroexp--accumulate", 1),
    ("macroexp--with-extended-form-stack", 1), ("macroexp-let2", 3), ("macroexp-let2*", 2),
    ("macroexp-preserve-posification", 1), ("map-let", 2), ("minibuffer-with-setup-hook", 1),
    ("named-let", 2), ("oclosure--lambda", 3), ("oclosure-define", 1), ("oclosure-lambda", 2),
    ("org-add-props", 2), ("org-agenda-with-point-at-orig-entry", 1),
    ("org-babel-comint-async-delete-dangling-and-eval", 1), ("org-babel-comint-in-buffer", 1),
    ("org-babel-comint-with-output", 1), ("org-babel-map-call-lines", 1),
    ("org-babel-map-inline-src-blocks", 1), ("org-babel-map-src-blocks", 1),
    ("org-babel-result-cond", 1), ("org-babel-with-temp-filebuffer", 1), ("org-cite-emphasize",
    1), ("org-cite-register-processor", 1), ("org-combine-change-calls", 2), ("org-dlet", 1),
    ("org-element-adopt", 1), ("org-element-adopt-elements", 1), ("org-element-ast-map", 2),
    ("org-element-lineage-map", 2), ("org-element-map", 2), ("org-element-with-disabled-cache",
    0), ("org-eval-in-environment", 1), ("org-export-to-buffer", 2), ("org-export-to-file", 2),
    ("org-fold-core-cycle-over-indirect-buffers", 0), ("org-fold-core-ignore-modifications",
    0), ("org-fold-core-save-visibility", 1), ("org-fold-core-suppress-folding-fix", 0),
    ("org-fold-save-outline-visibility", 1), ("org-lint-add-checker", 1), ("org-no-warnings",
    0), ("org-save-outline-visibility", 1), ("org-unbracket-string", 2),
    ("org-with-base-buffer", 1), ("org-with-gensyms", 1), ("org-with-point-at", 1),
    ("org-with-remote-undo", 1), ("org-with-syntax-table", 1), ("org-with-undo-amalgamate", 0),
    ("org-without-partial-completion", 0), ("pcase", 1), ("pcase-defmacro", 2),
    ("pcase-dolist", 1), ("pcase-exhaustive", 1), ("pcase-let", 1), ("pcase-let*", 1),
    ("prog1", 1), ("prog2", 2), ("progn", 0), ("replace--push-stack", 0), ("report-errors", 1),
    ("rx-let", 1), ("rx-let-eval", 1), ("save-current-buffer", 0), ("save-excursion", 0),
    ("save-mark-and-excursion", 0), ("save-match-data", 0), ("save-restriction", 0),
    ("save-selected-window", 0), ("save-window-excursion", 0), ("seq-doseq", 1), ("seq-let",
    2), ("static-if", 2), ("static-unless", 1), ("static-when", 1), ("thread-first", 0),
    ("thread-last", 0), ("track-mouse", 0), ("treesit--some", 1), ("treesit-node-get", 1),
    ("treesit-query-first-valid", 1), ("treesit-query-with-fallback", 1), ("unless", 1),
    ("unwind-protect", 1), ("when", 1), ("when-let", 1), ("when-let*", 1), ("while", 1),
    ("while-let", 1), ("while-no-input", 0), ("with-auto-compression-mode", 0),
    ("with-buffer-unmodified-if-unchanged", 0), ("with-case-table", 1), ("with-category-table",
    1), ("with-coding-priority", 1), ("with-connection-local-application-variables", 1),
    ("with-connection-local-variables", 0), ("with-current-buffer", 1),
    ("with-current-buffer-window", 3), ("with-decoded-time-value", 1), ("with-delayed-message",
    1), ("with-demoted-errors", 1), ("with-displayed-buffer-window", 3),
    ("with-environment-variables", 1), ("with-eval-after-load", 1), ("with-existing-directory",
    0), ("with-file-modes", 1), ("with-help-window", 1), ("with-local-quit", 0),
    ("with-locale-environment", 1), ("with-memoization", 1),
    ("with-minibuffer-completions-window", 0), ("with-minibuffer-selected-window", 0),
    ("with-mutex", 1), ("with-no-warnings", 0), ("with-output-to-string", 0),
    ("with-output-to-temp-buffer", 1), ("with-restriction", 2), ("with-selected-frame", 1),
    ("with-selected-window", 1), ("with-silent-modifications", 0), ("with-slots", 2),
    ("with-sqlite-transaction", 1), ("with-suppressed-warnings", 1), ("with-syntax-table", 1),
    ("with-system-sleep-block", 1), ("with-temp-buffer", 0), ("with-temp-buffer-window", 3),
    ("with-temp-file", 1), ("with-temp-message", 1), ("with-timeout", 1),
    ("with-undo-amalgamate", 0), ("with-window-non-dedicated", 1), ("with-work-buffer", 0),
    ("with-wrapper-hook", 2), ("without-remote-files", 0), ("without-restriction", 0),
];

#[rustfmt::skip]
const DEFUN_SPECS: &[&str] = &[
    "autoload", "cl-defmethod", "cl-generic-define-context-rewriter", "defalias",
    "defcalcmodevar", "defclass", "defconst", "defcustom", "defface", "defgroup", "defimage",
    "define-abbrev", "define-abbrev-table", "define-alternatives", "define-auto-insert",
    "define-button-type", "define-category", "define-ccl-program", "define-char-code-property",
    "define-charset", "define-charset-internal", "define-coding-system",
    "define-compilation-mode", "define-completion-category", "define-derived-mode",
    "define-fringe-bitmap", "define-global-minor-mode", "define-globalized-minor-mode",
    "define-ibuffer-column", "define-inline", "define-key-after", "define-keymap",
    "define-mail-user-agent", "define-minor-mode", "define-multisession-variable",
    "define-obsolete-function-alias", "define-obsolete-variable-alias",
    "define-short-documentation-group", "define-skeleton", "define-translation-hash-table",
    "define-translation-table", "define-treesit-generic-mode", "define-widget",
    "define-widget-keywords", "defmath", "defvar", "defvar-local", "defvaralias",
    "easy-menu-define", "easy-mmode-define-global-mode", "easy-mmode-define-minor-mode",
    "isearch-define-mode-toggle", "iter-defun", "keymap-set-after", "lambda",
    "org-agenda--insert-overriding-header", "org-defvaralias", "pcase-lambda", "rx-define",
    "transient-append-suffix", "transient-inline-group", "transient-insert-suffix",
    "transient-remove-suffix", "transient-replace-suffix", "use-package",
    "use-package-only-one", "which-key-add-keymap-based-replacements",
    "which-key-add-major-mode-key-based-replacements",
];

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
        assert_eq!(format_elisp_nameless(input, &FormatConfig::default(), &nl), expected);
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
