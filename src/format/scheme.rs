//! Scheme indentation — a native Rust port of Emacs's `scheme-indent-function`
//! (`scheme.el`), the style Emacs's `scheme-mode` uses for the whole Scheme family
//! (ADR-0011, ADR-0026, ADR-0031). It drives Scheme, Guile, Racket, Gauche, Mosh,
//! Gambit, and the permissive Scheme superset.
//!
//! `scheme-indent-function`'s own comment says it "duplicates almost all of
//! `lisp-indent-function`". It differs from the Emacs Lisp engine (`super`) only
//! in *which* indent-spec table it consults and in one named method:
//!
//! - it reads the `scheme-indent-function` property (the `(put 'sym
//!   'scheme-indent-function …)` block near the bottom of `scheme.el`, ~60
//!   entries) instead of `lisp-indent-function` — bundled here in [`method_for`];
//! - `syntax-rules` maps to `'defun`, and any head longer than 3 chars starting
//!   with `def` is *also* treated as `defun` (Emacs's `(string-match "\`def" …)`
//!   fallback), matching how the Emacs Lisp engine's bundled table lists `defun`s;
//! - `let` / `match-let` use the named method [`scheme_let_indent`]: a *named* let
//!   (`(let loop ((i 0)) …)`, a symbol right after `let`) indents like an integer
//!   spec of 2, an ordinary `let` like a spec of 1.
//!
//! The integer specforms reuse the shared [`specform`] ([`lisp-indent-specform`
//! is called by `scheme-indent-function` too]); the string/comment/data gates
//! are the shared driver's. `normal-indent` is this engine's own faithful port
//! of the full `calculate-lisp-indent` ([`scheme_normal`]) — `scheme-indent-
//! function` returns it directly for the data path, distinguished args past the
//! second, and body forms, so the port must be complete (the Emacs Lisp engine's
//! [`super::normal_indent`] is a partial port that suffices there only because
//! that engine computes body alignment explicitly).
//!
//! The bundled table ([`method_for`]) is the **runtime** property set of a
//! default `scheme-mode` buffer, which is the union of all of `scheme.el`'s
//! `put` blocks — the core, the DSSSL forms, **and the MIT block**, since
//! `scheme-mit-dialect` defaults to `t` (so e.g. `with-output-to-string` is `0`).
//!
//! Fidelity is validated against Emacs (`scheme-mode`); tests use golden output
//! captured from it. Like every engine here it only rewrites leading whitespace,
//! so it is always safe.

use lispexp::{Datum, DatumKind};

use super::{head_is_symbol_like, specform, Cols};

/// The `normal-indent` Emacs feeds `scheme-indent-function` — a faithful port of
/// the column `calculate-lisp-indent` positions point at before calling the hook.
/// `scheme-indent-function` returns this value directly for the data path,
/// distinguished args past the second, and body forms, so the port must be
/// complete (the Emacs Lisp engine's [`super::normal_indent`] is a partial port
/// that suffices only because that engine computes body alignment explicitly).
///
/// The cases, following `calculate-lisp-indent`:
///
/// - **no element completed before this line** (`calculate-lisp-indent-last-sexp`
///   nil) → `open + 1` (indent-point immediately follows the open paren);
/// - **first element is a list** → align under it (data);
/// - **car is not a symbol** (string, char literal `#\x`, nested list, …) or a
///   *space/tab* follows the open paren on its own line (`whitespace-after-open-
///   paren`) → align under the first element;
/// - otherwise a **function call**:
///   - only the head is completed → `open + 1` (the first argument's slot);
///   - the previous complete sibling begins its own line (case 3) → align under
///     it (`dynamic-wind`'s third `lambda` lines up under the second when the
///     second began its own line);
///   - else (case 2) skip exactly the first element and align under the second
///     (`(f a\n   b)`, or `'#u8(\n #xff #xff\n 0 0)` under the second byte).
fn scheme_normal(cols: &Cols, c: &Datum, items: &[Datum], open_col: usize, offset: usize) -> usize {
    let Some(first) = items.first() else {
        return open_col + 1;
    };
    // Nothing completed before this line → `open + 1`. Also guards against
    // aligning under an element on the current (or a later) line, whose new
    // indent is not yet known.
    let completed = items
        .iter()
        .filter(|it| (it.span.end as usize) <= offset)
        .count();
    if completed == 0 || cols.line_of(first.span.start as usize) >= cols.line_of(offset) {
        return open_col + 1;
    }
    // First element is a list → align under it (data).
    if matches!(first.kind, DatumKind::List { .. }) {
        return cols.col(first.span.start as usize);
    }
    // `whitespace-after-open-paren`: a space/tab right after `(` on its own line
    // (not a newline). When the first element is on a later line the char after
    // `(` is a newline, which Emacs does not count.
    let ws_after_open = (first.span.start as usize) > (c.span.start as usize) + 1
        && cols.line_of(first.span.start as usize) == cols.line_of(c.span.start as usize);
    // Car not a symbol (string / char literal / prefixed data) or ws-after-open
    // → align under the first element.
    if !head_is_symbol_like(first) || ws_after_open {
        return cols.col(first.span.start as usize);
    }
    // Function call. Only the first element (the head) is completed → `open + 1`
    // (the first argument's natural slot).
    if completed <= 1 {
        // A dotted pair with a lone car (`'(a . tail)`): Emacs treats the `.` as
        // the first argument, so a tail continuation aligns under the dot when it
        // is on the open line (the `'(eval . FORM)` idiom).
        if items.len() == 1 {
            if let Some(dot) = c.dot_span() {
                if cols.line_of(dot.start as usize) == cols.line_of(c.span.start as usize) {
                    return cols.col(dot.start as usize);
                }
            }
        }
        return open_col + 1;
    }
    // `calculate-lisp-indent` case 3: when the previous complete sibling *begins
    // its own line* (a line after the open paren, and it is the first sexp on
    // that line), align under it — Emacs's "indent beneath first sexp on the same
    // line as the last complete sexp." This is what lines up the third argument
    // of a spec-3 form such as `dynamic-wind` under the second when the second
    // began its own line, and a body continuation under its predecessor.
    let prev = items
        .iter()
        .rev()
        .find(|it| (it.span.end as usize) <= offset)
        .expect("completed > 1");
    let open_line = cols.line_of(c.span.start as usize);
    let prev_line = cols.line_of(prev.span.start as usize);
    let prev_is_line_start = items
        .iter()
        .find(|it| cols.line_of(it.span.start as usize) == prev_line)
        .is_some_and(|firstonline| firstonline.span.start == prev.span.start);
    if prev_line > open_line && prev_is_line_start {
        return cols.col(prev.span.start as usize);
    }
    // Case 2: skip exactly the first element (the head) and align under the
    // second (the first argument).
    match items.get(1) {
        Some(second) => cols.col(second.span.start as usize),
        None => open_col + 1,
    }
}

/// The head symbol of a list for spec lookup, seeing through any reader prefixes
/// (`'sym` / `` `sym `` / `,sym` / `#'sym`) — Emacs reads the property of the
/// symbol `backward-prefix-chars` lands on. `None` for a non-symbol head.
fn head_symbol<'a>(d: &Datum<'a>) -> Option<&'a str> {
    match &d.kind {
        DatumKind::Symbol(s) => Some(s),
        DatumKind::Prefixed { inner, .. } => head_symbol(inner),
        _ => None,
    }
}

/// A named Scheme indent method — `scheme.el`'s function-valued spec. Only
/// `scheme-let-indent` exists (`let` and `match-let` share it).
#[derive(Clone, Copy)]
enum Named {
    /// `scheme-let-indent` — distinguish a *named* let from an ordinary one.
    LetIndent,
}

/// A `scheme-indent-function` property value, as bundled in [`method_for`].
enum Spec {
    /// `'defun` — indent like a definition (body at `open_col + body`). Also the
    /// implicit spec for any `def…` head with no explicit property.
    Defun,
    /// An integer N — `lisp-indent-specform`: the first N args are distinguished.
    Int(usize),
    /// A named function method (`scheme-let-indent`).
    Fn(Named),
}

/// Indent the code line starting at `offset`, given the innermost containing
/// list `c` (Emacs's `(elt state 1)`). Returns the target column. `body` is
/// `lisp-body-indent`.
///
/// A close port of `scheme-indent-function`: compute `normal-indent` (the shared
/// function-call alignment), then dispatch on the head symbol's
/// `scheme-indent-function` property. Only a symbol head carries a property; a
/// non-symbol head (the `(not (looking-at "\\sw\\|\\s_"))` data path), or a
/// symbol head with no property and no `def…` shape, falls back to `normal`.
pub(super) fn indent(cols: &Cols, c: &Datum, offset: usize, body: usize) -> usize {
    let DatumKind::List { items, .. } = &c.kind else {
        return 0;
    };
    let open_col = cols.col(c.span.start as usize);
    let normal = scheme_normal(cols, c, items, open_col, offset);

    // `scheme-indent-function` only looks up a property when the car reads as a
    // symbol; a non-symbol head takes the data path, which is exactly `normal`.
    // (`#(…)` / `#u8(…)` reader-macro forms surface as their inner list here —
    // `container_at` descends into them — so a char-literal head such as
    // `#\x0030` is data, but a numeric head such as `#xff` is a "call" and
    // aligns under its second element, matching Emacs.)
    //
    // Reader prefixes on the head are transparent to the lookup: Emacs's
    // `backward-prefix-chars` steps over `'` / `` ` `` / `,` / `#`, so `('when …)`
    // uses `when`'s spec and `('lambda …)` `lambda`'s. A prefixed head with no
    // spec (`('foo …)`) stays a call, which `normal` already handles.
    let Some(head) = items.first().and_then(head_symbol) else {
        return normal;
    };

    match method_for(head) {
        Some(Spec::Defun) => open_col + body,
        Some(Spec::Int(n)) => specform(cols, items, offset, open_col, n, normal, body),
        Some(Spec::Fn(Named::LetIndent)) => {
            scheme_let_indent(cols, items, offset, open_col, normal, body)
        }
        // No explicit property: Emacs's `(and (null method) (> (length function)
        // 3) (string-match "\`def" function))` treats a `def…` head as `defun`.
        None if head.len() > 3 && head.starts_with("def") => open_col + body,
        None => normal,
    }
}

/// `scheme-let-indent` — Scheme's `let` is special because of *named* let:
/// `(let loop ((i 0)) …)` puts a symbol where an ordinary `let` has its binding
/// list. Emacs skips whitespace after the head and checks whether a symbol
/// character follows: if so it indents as `lisp-indent-specform 2` (the name and
/// the bindings are both distinguished), otherwise as `lisp-indent-specform 1`.
fn scheme_let_indent(
    cols: &Cols,
    items: &[Datum],
    offset: usize,
    open_col: usize,
    normal: usize,
    body: usize,
) -> usize {
    // The element right after the head: a symbol-like token → named let (spec 2),
    // anything else (a binding list `((…))`, or nothing) → plain let (spec 1).
    // Emacs's `(looking-at "[-a-zA-Z0-9+*/?!@$%^&_:~]")` is precisely a symbol,
    // number, or char literal — [`super::head_is_symbol_like`].
    let n = match items.get(1) {
        Some(d) if super::head_is_symbol_like(d) => 2,
        _ => 1,
    };
    specform(cols, items, offset, open_col, n, normal, body)
}

/// The bundled `scheme-indent-function` table. This is the **runtime** set of
/// `scheme-indent-function` properties in a default `scheme-mode` buffer, dumped
/// from a real Emacs (`(get sym 'scheme-indent-function)` over all symbols after
/// `(require 'scheme)`). It is the union of `scheme.el`'s three `put` blocks:
/// the core block, the DSSSL block (`element`/`mode`/`with-mode`/`make`/`style`/
/// `root`/`λ`, applied in `scheme-mode` too), **and the MIT block** — which is
/// active by default because `scheme-mit-dialect` defaults to `t`, not nil (so
/// `with-output-to-string` is `0`, `named-lambda`/`fluid-let`/… are `1`, etc.).
///
/// Returns the spec for `name`, or `None` when unlisted (a plain function call,
/// unless it has the `def…` shape, which the caller handles). Scheme is
/// case-sensitive, so names are matched as written (unlike the CL engine's
/// case-folded lookup). Regenerate from Emacs when the oracle changes; see
/// `docs/dev/formatter.md`.
fn method_for(name: &str) -> Option<Spec> {
    let spec = match name {
        // --- named methods ---
        "let" | "match-let" => Spec::Fn(Named::LetIndent),

        // --- `defun` ---
        "syntax-rules" => Spec::Defun,

        // --- 0 ---
        "begin" | "delay" | "make-environment" | "match-lambda" | "match-lambda*" | "sequence"
        | "with-output-to-string" => Spec::Int(0),

        // --- 1 ---
        // Core / SRFI / R6RS / R7RS.
        "and-let*" | "call-with-input-file" | "call-with-output-file" | "call-with-port"
        | "call-with-values" | "case" | "check-case" | "define-library" | "define-record-type"
        | "define-values" | "eval-when" | "guard" | "lambda" | "lambda-checked" | "let*"
        | "let*-values" | "let-syntax" | "let-values" | "letrec" | "letrec*" | "letrec-syntax"
        | "library" | "match" | "match-let*" | "match-letrec" | "opt-lambda" | "opt*-lambda"
        | "parameterize" | "test-group" | "test-group-with-cleanup" | "unless" | "when"
        | "with-input-from-file" | "with-input-from-port" | "with-output-to-file"
        | "with-output-to-port" | "with-syntax" | "λ"
        // DSSSL (`scheme.el` applies these in `scheme-mode`).
        | "element" | "make" | "mode" | "root" | "style" | "with-mode"
        // MIT (`scheme-mit-dialect` defaults to t).
        | "access-components" | "assignment-components" | "combination-components"
        | "comment-components" | "conditional-components" | "declaration-components"
        | "definition-components" | "delay-components" | "disjunction-components" | "fluid-let"
        | "in-package" | "in-package-components" | "lambda-components" | "lambda-components*"
        | "lambda-components**" | "list-search-negative" | "list-search-positive"
        | "list-transform-negative" | "list-transform-positive" | "local-declare" | "macro"
        | "named-lambda" | "open-block-components" | "pathname-components" | "procedure-components"
        | "sequence-components" | "unassigned?-components" | "unbound?-components" | "using-syntax"
        | "variable-components" | "with-input-from-string" | "with-values" => Spec::Int(1),

        // --- 2 ---
        "do" | "let-optionals" | "let-optionals*" | "receive" | "syntax-case"
        | "syntax-table-define" => Spec::Int(2),

        // --- 3 ---
        "dynamic-wind" => Spec::Int(3),

        _ => return None,
    };
    Some(spec)
}

#[cfg(test)]
mod tests {
    use crate::config::FormatConfig;
    use lispexp::Dialect;

    fn fmt(input: &str) -> String {
        crate::format::format(input, &FormatConfig::default(), Dialect::Scheme)
    }

    /// Reindent flat Scheme and compare to Emacs's canonical output (captured
    /// from `scheme-mode`, `indent-tabs-mode` nil). Covers `define`/body, plain
    /// vs named `let`, `let*`/`letrec`, `case`, `cond`, `do`, `lambda`,
    /// `when`/`unless`, and a plain function-call alignment.
    #[test]
    fn matches_emacs_on_common_forms() {
        let input = "\
(define (foo x)
(bar x)
(baz x))
(let ((a 1)
(b 2))
(+ a b))
(let loop ((i 0))
(when (< i 10)
(loop (+ i 1))))
(let* ((a 1)
(b 2))
(+ a b))
(letrec ((f (lambda () (g)))
(g (lambda () (f))))
(f))
(case x
((1) 'one)
((2) 'two))
(cond ((positive? x)
'pos)
((negative? x)
'neg))
(do ((i 0 (+ i 1))
(acc '() (cons i acc)))
((= i 5) acc))
(lambda (x)
(* x x))
(when test
(do-thing)
(do-other))
(unless test
(fallback))
(some-function arg1
arg2
arg3)
";
        let expected = "\
(define (foo x)
  (bar x)
  (baz x))
(let ((a 1)
      (b 2))
  (+ a b))
(let loop ((i 0))
  (when (< i 10)
    (loop (+ i 1))))
(let* ((a 1)
       (b 2))
  (+ a b))
(letrec ((f (lambda () (g)))
         (g (lambda () (f))))
  (f))
(case x
  ((1) 'one)
  ((2) 'two))
(cond ((positive? x)
       'pos)
      ((negative? x)
       'neg))
(do ((i 0 (+ i 1))
     (acc '() (cons i acc)))
    ((= i 5) acc))
(lambda (x)
  (* x x))
(when test
  (do-thing)
  (do-other))
(unless test
  (fallback))
(some-function arg1
               arg2
               arg3)
";
        assert_eq!(fmt(input), expected);
    }

    /// `syntax-rules` indents like `defun`; quasiquote data aligns like a data
    /// list (non-symbol head path); `define-record-type` uses its integer spec.
    /// Golden captured from Emacs `scheme-mode`.
    #[test]
    fn matches_emacs_on_macros_and_data() {
        let input = "\
(define-syntax swap!
(syntax-rules ()
((_ a b)
(let ((tmp a))
(set! a b)
(set! b tmp)))))
(define data
`(one
two
three))
(define-record-type point
(make-point x y)
point?
(x point-x)
(y point-y))
";
        let expected = "\
(define-syntax swap!
  (syntax-rules ()
    ((_ a b)
     (let ((tmp a))
       (set! a b)
       (set! b tmp)))))
(define data
  `(one
    two
    three))
(define-record-type point
  (make-point x y)
  point?
  (x point-x)
  (y point-y))
";
        assert_eq!(fmt(input), expected);
    }

    /// The specialised integer specs and their `normal-indent` interaction:
    /// `receive` (2 distinguished), `dynamic-wind` (3 distinguished — the third
    /// `lambda` also lands at `2×body`), `with-output-to-string` (0, from the MIT
    /// block, so the body is at `open+body`), `define-record-type` (1, fields
    /// under the first body form), and `do` (2). Golden captured from Emacs
    /// `scheme-mode`.
    #[test]
    fn matches_emacs_on_specialised_specforms() {
        let input = "\
(receive (a b)
(values 1 2)
(+ a b))
(dynamic-wind
(lambda () (setup))
(lambda ()
(work))
(lambda () (teardown)))
(with-output-to-string
(lambda ()
(display \"hi\")))
(define-record-type point
(make-point x y)
point?
(x point-x)
(y point-y))
(do ((i 0 (+ i 1)))
((= i 5))
(display i))
";
        let expected = "\
(receive (a b)
    (values 1 2)
  (+ a b))
(dynamic-wind
    (lambda () (setup))
    (lambda ()
      (work))
    (lambda () (teardown)))
(with-output-to-string
  (lambda ()
    (display \"hi\")))
(define-record-type point
  (make-point x y)
  point?
  (x point-x)
  (y point-y))
(do ((i 0 (+ i 1)))
    ((= i 5))
  (display i))
";
        assert_eq!(fmt(input), expected);
    }

    /// Vectors and char literals: a `#(…)` char vector (`#\\x…` heads) is *data*,
    /// aligning under the first element, while a `#u8(…)` bytevector (numeric
    /// `#xNN` heads) is a *call*, aligning under the second element. Golden
    /// captured from Emacs `scheme-mode`.
    #[test]
    fn matches_emacs_on_vectors_and_char_literals() {
        let input = "\
(define chars
'#(#\\a
#\\b
#\\c))
(define bytes
'#u8(1 2 3
4 5 6
7 8 9))
";
        let expected = "\
(define chars
  '#(#\\a
     #\\b
     #\\c))
(define bytes
  '#u8(1 2 3
         4 5 6
         7 8 9))
";
        assert_eq!(fmt(input), expected);
    }

    /// The Scheme engine only rewrites leading whitespace, so an already-
    /// formatted file is a fixed point.
    #[test]
    fn already_formatted_is_a_fixed_point() {
        let formatted = "(define (foo x)\n  (bar x))\n";
        assert_eq!(fmt(formatted), formatted);
    }
}
