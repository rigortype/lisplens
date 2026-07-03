//! Common Lisp indentation — a native Rust port of Emacs's
//! `common-lisp-indent-function` (`cl-indent.el`), the style Emacs's `lisp-mode`
//! uses for Common Lisp (ADR-0011, ADR-0026). Distinct from the Emacs Lisp
//! engine (`super`): where `lisp-indent-function` looks only at the innermost
//! containing list, this walks *up* several levels (`MAX_BACKTRACKING`) building
//! a `path`, and drives a small spec language (`lisp-indent-259`) — nil / integer
//! / `&lambda` / `&rest` / `&body` / `&whole` / destructuring sublists / named
//! methods — from a bundled standard table.
//!
//! `normal-indent` (the "align like a function call" fallback) is computed by the
//! same [`super::normal_indent`] port of `calculate-lisp-indent`: Emacs feeds the
//! identical value to both indent functions, so a form with no special method
//! indents the same in either engine. Only the method table and the richer spec
//! walker make Common Lisp diverge.
//!
//! Fidelity is validated against Emacs (`lisp-mode` with
//! `lisp-indent-function` = `common-lisp-indent-function`); tests use golden
//! output captured from it. Like every engine here it only rewrites leading
//! whitespace, so it is always safe.

use lispexp::{Datum, DatumKind};

use super::Cols;
use crate::config::FormatConfig;

/// `lisp-indent-maximum-backtracking` — how far to walk out from the innermost
/// list looking for a form that specifies indentation (e.g. reaching `flet`
/// from inside a local-function body).
const MAX_BACKTRACKING: usize = 3;
/// `lisp-tag-indentation` — a `tagbody` tag, relative to the containing list.
const TAG_INDENTATION: i64 = 1;
/// `lisp-tag-body-indentation` — a non-tag line inside a `tagbody`.
const TAG_BODY_INDENTATION: i64 = 3;
/// `lisp-simple-loop-indentation` — forms in a simple `loop`.
const SIMPLE_LOOP_INDENTATION: i64 = 1;
/// `lisp-loop-keyword-indentation` / `lisp-loop-forms-indentation` — clauses in
/// an extended `loop` (Emacs defaults both to 6).
const LOOP_KEYWORD_INDENTATION: i64 = 6;
const LOOP_FORMS_INDENTATION: i64 = 6;

/// A named indent method — `cl-indent.el`'s function-valued specs, dispatched by
/// [`call_named`]. (`lisp-indent-259` is the default and is not named.)
#[derive(Clone, Copy)]
enum Named {
    /// `lisp-indent-tagbody`.
    Tagbody,
    /// `lisp-indent-do`.
    Do,
    /// `lisp-indent-defmethod`.
    Defmethod,
    /// `lisp-indent-function-lambda-hack`.
    LambdaHack,
}

/// One element of a list-valued indent method (a member of the `method` list
/// walked by [`lisp_indent_259`]). Mirrors the elisp spec vocabulary.
enum Elem {
    /// nil — default (normal) indentation for this argument.
    Nil,
    /// An explicit column offset from the containing form.
    Int(i64),
    /// `&lambda` — a lambda list, indented by 4 (or aligned within).
    Lambda,
    /// `&rest` — the following element applies to all remaining arguments.
    Rest,
    /// `&body` — `&rest lisp-body-indent`.
    Body,
    /// `&whole` — the marker leading a destructuring sublist (`(&whole X …)`);
    /// only its position matters, never inspected directly.
    Whole,
    /// A named function method.
    Fn(Named),
    /// A destructuring sublist, applied when the argument is itself a list.
    List(Vec<Elem>),
}

/// A top-level indent method: the value of a symbol's `common-lisp-indent-function`
/// property, as stored in the bundled table.
enum Spec {
    /// An integer N — indent the first N arguments like distinguished forms and
    /// the rest like a body (`(4 4 … &body)`).
    Int(i64),
    /// A named function method.
    Fn(Named),
    /// A list spec, walked by [`lisp_indent_259`].
    List(Vec<Elem>),
}

/// `lisp-indent-defun-method` = `(4 &lambda &body)`, the expansion of the
/// `defun` property value.
fn defun_method() -> Vec<Elem> {
    vec![Elem::Int(4), Elem::Lambda, Elem::Body]
}

/// Shared state for one line's indentation, mirroring the dynamic variables
/// `common-lisp-indent-function-1` threads through its helpers. `sexp_column`,
/// `normal`, and `body` are `calculate-lisp-indent`'s `sexp-column`,
/// `normal-indent`, and `lisp-body-indent` respectively; `sexp_column` is fixed
/// at the *innermost* containing list's column even while backtracking outward.
struct Ctx<'a> {
    cols: &'a Cols<'a>,
    source: &'a str,
    /// The innermost containing list (Emacs's `(elt state 1)`) — needed to align
    /// a continued lambda list against its keywords.
    innermost: &'a Datum<'a>,
    offset: usize,
    sexp_column: i64,
    normal: i64,
    body: i64,
}

/// `lisp-lambda-list-keyword-parameter-indentation` — a lambda-list parameter's
/// offset from its keyword (e.g. `&key`).
const LAMBDA_LIST_KEYWORD_PARAMETER_INDENTATION: i64 = 2;

/// Indent the code line starting at `offset`, given the innermost containing
/// list `innermost` (Emacs's `(elt state 1)`). Returns the target column.
pub(super) fn indent(
    cols: &Cols,
    source: &str,
    data: &[Datum],
    innermost: &Datum,
    offset: usize,
    config: &FormatConfig,
) -> usize {
    let body = config.body_indent as i64;
    let sexp_column = cols.col(innermost.span.start as usize) as i64;

    // `normal-indent`, computed exactly as `calculate-lisp-indent` does (the
    // same value Emacs would feed the indent hook).
    let normal = cl_normal_indent(cols, innermost, offset);

    // The `loop` special-case fires before the general walker (Emacs's FIXME).
    if is_loop_form(innermost) {
        return loop_indent(cols, source, innermost, offset).max(0) as usize;
    }

    let mut normal = normal;
    let chain = containing_chain(data, offset);
    let mut path: Vec<usize> = Vec::new();
    let mut calculated: Option<i64> = None;
    let mut tentative: Option<i64> = None;

    // Walk from the innermost containing list outward (`backward-up-list`).
    for (ri, level) in chain.iter().rev().enumerate() {
        if path.len() >= MAX_BACKTRACKING {
            break;
        }
        // The level one step further out (`backward-up-list` one more), for the
        // `lambda`/`function` hack.
        let li = chain.len() - 1 - ri;
        let parent = li.checked_sub(1).map(|j| chain[j]);
        let DatumKind::List { items, .. } = &level.kind else {
            continue;
        };
        // `n` = how far into this containing form the current form is: the count
        // of complete sexps that end before `offset` (0 = the head).
        let n = items
            .iter()
            .filter(|it| (it.span.end as usize) <= offset)
            .count();
        path.insert(0, n);

        let ctx = Ctx {
            cols,
            source,
            innermost,
            offset,
            sexp_column,
            normal,
            body,
        };

        // The head function name, with a package prefix stripped as a fallback
        // when the qualified name has no method (Emacs's "pleblisp" feature:
        // `cl:defconstant` → `defconstant`). The stripped name also drives the
        // `def…`/`with-…` heuristics below.
        let mut fname = head_name(items);
        let mut method = fname.as_deref().and_then(method_for);
        if method.is_none() {
            if let Some(stripped) = fname.as_deref().and_then(strip_package_prefix) {
                method = method_for(stripped);
                fname = Some(stripped.to_string());
            }
        }

        // Backwards-compatibility method inference — only at the innermost level
        // (`(null (cdr path))`).
        let mut tentative_defun = false;
        if path.len() == 1 {
            if let Some(name) = &fname {
                if method.is_none() {
                    if name.starts_with("def") {
                        tentative_defun = true;
                    } else if is_with_or_do_prefixed(name) {
                        method = Some(Spec::List(vec![Elem::Lambda, Elem::Body]));
                    }
                }
            }
        }

        // Reader-prefix handling on the open paren of this level.
        let open = level.span.start as usize;
        let mut cap_backtracking = false;
        if is_quote_data(source, open) {
            // `'(…)` — indent as data, under the open paren.
            calculated = Some(sexp_column + 1);
        } else {
            if is_comma_substitution(source, open) {
                // `,(…)` / `,@(…)` inside a backquote: fall back to normal and
                // stop backtracking past the substitution (backquote mode t).
                tentative = Some(normal);
                cap_backtracking = true;
            }
            if calculated.is_none() {
                if is_hash(source, open) {
                    // `#(…)` — indent under the open paren.
                    calculated = Some(sexp_column + 1);
                } else if fname.is_none() {
                    // Head is not a symbol: no method, keep backtracking.
                } else if let Some(spec) = method {
                    calculated = Some(match spec {
                        Spec::Int(m) => integer_method(m, &path, sexp_column, normal, body),
                        Spec::Fn(named) => call_named(named, &path, &ctx, level, parent),
                        Spec::List(elems) => lisp_indent_259(&elems, &path, &ctx, level, parent),
                    });
                } else if tentative_defun {
                    // Looks like a `def…`: indent as a defun, but tentatively —
                    // an outer construct may override. Also update `normal` for
                    // deeper backtracking.
                    let t = lisp_indent_259(&defun_method(), &path, &ctx, level, parent);
                    tentative = Some(t);
                    normal = t;
                }
            }
        }

        if calculated.is_some() || cap_backtracking {
            break;
        }
    }

    calculated.or(tentative).unwrap_or(normal).max(0) as usize
}

/// `calculate-lisp-indent`'s `normal-indent`: the "align like a function call"
/// column Emacs feeds the indent hook. Three cases, in order:
///
/// 1. the containing form's first element is a list → align under that element;
/// 2. the last complete sexp is on the form's first line (a fresh call) → align
///    under the first argument, or under the head if there is no argument yet or
///    whitespace follows the open paren;
/// 3. otherwise → align under the first sexp on the same line as that last
///    complete sexp (i.e. under the previous sibling on its own line).
///
/// The Emacs Lisp engine's [`super::normal_indent`] covers only cases 1–2, since
/// that engine computes body alignment explicitly; Common Lisp reaches case 3
/// through `&body`/`&rest`, so it needs the full computation.
fn cl_normal_indent(cols: &Cols, innermost: &Datum, offset: usize) -> i64 {
    let DatumKind::List { items, .. } = &innermost.kind else {
        return cols.col(innermost.span.start as usize) as i64 + 1;
    };
    let open = innermost.span.start as usize;
    let open_col = cols.col(open) as i64;
    // The last complete sexp before this line (the previous sibling).
    let Some(last) = items
        .iter()
        .rev()
        .find(|it| (it.span.end as usize) <= offset)
    else {
        return open_col + 1; // indent-point immediately follows the open paren
    };
    let first = &items[0];
    // Case 1.
    if matches!(first.kind, DatumKind::List { .. }) {
        return cols.col(first.span.start as usize) as i64;
    }
    if cols.line_of(last.span.start as usize) == cols.line_of(first.span.start as usize) {
        // Case 2.
        let ws_after_open = (first.span.start as usize) > open + 1;
        let first_is_last = first.span.start == last.span.start;
        if ws_after_open || first_is_last {
            cols.col(first.span.start as usize) as i64
        } else {
            cols.col(items[1].span.start as usize) as i64
        }
    } else {
        // Case 3.
        let target = cols.line_of(last.span.start as usize);
        let anchor = items
            .iter()
            .find(|it| cols.line_of(it.span.start as usize) == target)
            .unwrap_or(last);
        cols.col(anchor.span.start as usize) as i64
    }
}

/// `lisp-indent-259` — walk `method` guided by `path` to a column. Returns the
/// indentation for the current line. `level` is the containing list whose head
/// carries this method, `parent` the one further out (both needed by named
/// sub-methods).
fn lisp_indent_259(
    method: &[Elem],
    path: &[usize],
    ctx: &Ctx,
    level: &Datum,
    parent: Option<&Datum>,
) -> i64 {
    let mut method = method;
    let mut p = path;
    // Outer loop: destructure one path element per iteration.
    while !p.is_empty() {
        let mut n: i64 = p[0] as i64 - 1;
        p = &p[1..];
        let mut tail = false;
        // Inner loop: advance along `method` until the relevant element. It exits
        // only by returning a column or by `break`ing after descending into a
        // destructuring sublist (then the outer loop continues on the new path).
        loop {
            let Some(tem) = method.first() else {
                return ctx.normal;
            };
            // `&rest` tail: a plain (non-list, non-fn) element indents like the
            // first rest element.
            if tail && !matches!(tem, Elem::List(_) | Elem::Fn(_)) {
                return ctx.normal;
            }
            if matches!(tem, Elem::Body) {
                return if n == 0 && p.is_empty() {
                    ctx.sexp_column + ctx.body
                } else {
                    ctx.normal
                };
            } else if matches!(tem, Elem::Rest) {
                tail = n > 0;
                n = 0;
                method = &method[1..];
                continue;
            } else if n > 0 {
                n -= 1;
                method = &method[1..];
                continue;
            } else if matches!(tem, Elem::Nil) {
                return ctx.normal;
            } else if matches!(tem, Elem::Lambda) {
                return if p.is_empty() {
                    ctx.sexp_column + 4
                } else if p.len() == 1 {
                    lambda_list_indent(ctx)
                } else {
                    ctx.normal
                };
            } else if let Elem::Int(k) = tem {
                return if p.is_empty() {
                    ctx.sexp_column + *k
                } else {
                    ctx.normal
                };
            } else if let Elem::Fn(named) = tem {
                return call_named(*named, path, ctx, level, parent);
            } else if let Elem::List(sub) = tem {
                if !p.is_empty() {
                    // Descend into the destructuring sublist: skip `&whole X`.
                    method = if sub.len() >= 2 { &sub[2..] } else { &[] };
                    break;
                }
                // Last path element: the `&whole` spec (2nd elem) governs.
                let x = sub.get(1);
                return if tail {
                    ctx.normal
                } else {
                    match x {
                        Some(Elem::Int(k)) => ctx.sexp_column + *k,
                        Some(Elem::Fn(named)) => call_named(*named, path, ctx, level, parent),
                        _ => ctx.normal,
                    }
                };
            } else {
                // A bare `&whole` in head position — not expected.
                return ctx.normal;
            }
        }
    }
    ctx.normal
}

/// An integer method N (handled directly in `common-lisp-indent-function-1`, not
/// via the walker): the first N arguments are distinguished (`sexp_column + 4`),
/// the first body form lands at `sexp_column + body`, the rest at `normal`.
fn integer_method(m: i64, path: &[usize], sexp_column: i64, normal: i64, body: i64) -> i64 {
    if path.len() > 1 {
        normal
    } else {
        let car = path[0] as i64;
        if car <= m {
            sexp_column + 4
        } else if car == m + 1 {
            sexp_column + body
        } else {
            normal
        }
    }
}

/// Dispatch a named method (`cl-indent.el`'s function-valued specs).
fn call_named(
    named: Named,
    path: &[usize],
    ctx: &Ctx,
    level: &Datum,
    parent: Option<&Datum>,
) -> i64 {
    match named {
        Named::Tagbody => tagbody_indent(path, ctx, TAG_BODY_INDENTATION),
        Named::Do => do_indent(path, ctx, level, parent),
        Named::Defmethod => defmethod_indent(path, ctx, level, parent),
        Named::LambdaHack => lambda_hack_indent(path, ctx, parent),
    }
}

/// `lisp-indent-tagbody` — a tag line indents by `TAG_INDENTATION`, a body line
/// by `tag_body` (rebound to `lisp-body-indent` when reached via `do`).
fn tagbody_indent(path: &[usize], ctx: &Ctx, tag_body: i64) -> i64 {
    if path.len() > 1 {
        return ctx.normal;
    }
    if line_starts_with_tag(ctx.source, ctx.offset) {
        ctx.sexp_column + TAG_INDENTATION
    } else {
        ctx.sexp_column + tag_body
    }
}

/// `lisp-indent-do` — the step/binding clauses use a nested spec; from the body
/// (`path[0] >= 3`) it indents as a `tagbody` with tag-body = `lisp-body-indent`.
fn do_indent(path: &[usize], ctx: &Ctx, level: &Datum, parent: Option<&Datum>) -> i64 {
    if path[0] >= 3 {
        tagbody_indent(path, ctx, ctx.body)
    } else {
        let spec = vec![
            Elem::List(vec![Elem::Whole, Elem::Nil, Elem::Rest]),
            Elem::List(vec![Elem::Whole, Elem::Nil, Elem::Rest, Elem::Int(1)]),
        ];
        lisp_indent_259(&spec, path, ctx, level, parent)
    }
}

/// `lisp-indent-defmethod` — count method qualifiers (symbols between the name
/// and the lambda list) and indent `(4 4… &lambda &body)`, else like `defun`.
fn defmethod_indent(path: &[usize], ctx: &Ctx, level: &Datum, parent: Option<&Datum>) -> i64 {
    let nqual = defmethod_qualifiers(level);
    let method = if path[0] >= 3 && nqual > 0 {
        let mut v = vec![Elem::Int(4)];
        v.extend((0..nqual).map(|_| Elem::Int(4)));
        v.push(Elem::Lambda);
        v.push(Elem::Body);
        v
    } else {
        defun_method()
    };
    lisp_indent_259(&method, path, ctx, level, parent)
}

/// The count of method qualifiers in a `defmethod` form: consecutive
/// symbol/keyword elements after the generic-function name, before the lambda
/// list (the first list argument).
fn defmethod_qualifiers(level: &Datum) -> usize {
    let DatumKind::List { items, .. } = &level.kind else {
        return 0;
    };
    items
        .iter()
        .skip(2)
        .take_while(|it| matches!(it.kind, DatumKind::Symbol(_) | DatumKind::Keyword(_)))
        .count()
}

/// `lisp-indent-function-lambda-hack` — for `lambda`, line the body up under the
/// form. When the lambda sits directly inside a `(function …)` list (Emacs's
/// `\(lisp:+\)?function` test), it lines up under that `function` symbol less one
/// column (to conserve width, as `#'` does); otherwise `sexp_column + body`.
fn lambda_hack_indent(path: &[usize], ctx: &Ctx, parent: Option<&Datum>) -> i64 {
    if path.len() > 1 || path[0] > 3 {
        return ctx.normal;
    }
    if let Some(p) = parent {
        if let DatumKind::List { items, .. } = &p.kind {
            if head_is_function(items) {
                let col = ctx.cols.col(items[0].span.start as usize) as i64;
                return ctx.body - 1 + col;
            }
        }
    }
    ctx.sexp_column + ctx.body
}

/// Whether a list's head is the `function` special operator (optionally
/// package-qualified `lisp:function`), Emacs's `\(lisp:+\)?function` test.
fn head_is_function(items: &[Datum]) -> bool {
    match head_name(items) {
        Some(name) => name == "function" || name.ends_with(":function"),
        None => false,
    }
}

/// `lisp-indent-lambda-list` — align a continued lambda list (the innermost
/// containing list). With Emacs's default settings
/// (`lisp-lambda-list-keyword-*-alignment` both nil):
///
/// - a line that itself opens with a lambda-list keyword (`&optional`, `&key`, …)
///   aligns under the first parameter (`1 + sexp_column`);
/// - otherwise, if a lambda-list keyword precedes this line, align its column
///   plus `LAMBDA_LIST_KEYWORD_PARAMETER_INDENTATION`;
/// - with no keyword yet, align under the first parameter (`1 + sexp_column`).
fn lambda_list_indent(ctx: &Ctx) -> i64 {
    let DatumKind::List { items, .. } = &ctx.innermost.kind else {
        return ctx.sexp_column + 1;
    };
    // A line that itself opens with a lambda-list keyword (`&key` etc.) followed
    // by whitespace or end-of-line — Emacs's keyword regex. `&allow-other-keys)`
    // (followed by `)`) does *not* qualify and falls through to keyword+2 below.
    if line_faces_lambda_list_keyword(ctx.source, ctx.offset) {
        return ctx.sexp_column + 1;
    }
    // The last lambda-list keyword completed before this line.
    let keyword = items
        .iter()
        .rev()
        .find(|it| (it.span.end as usize) <= ctx.offset && is_lambda_list_keyword(it));
    match keyword {
        Some(k) => {
            ctx.cols.col(k.span.start as usize) as i64 + LAMBDA_LIST_KEYWORD_PARAMETER_INDENTATION
        }
        None => ctx.sexp_column + 1,
    }
}

/// Whether the indent line opens with a lambda-list keyword *token* — the
/// keyword immediately followed by whitespace or end-of-line (Emacs's
/// `&\(optional\|…\)\([ \t]\|$\)`). `&allow-other-keys)` fails this (a `)`
/// follows), so it aligns as an ordinary parameter instead.
fn line_faces_lambda_list_keyword(source: &str, offset: usize) -> bool {
    let Some(line) = source[offset..].lines().next().map(str::trim_start) else {
        return false;
    };
    if !line.starts_with('&') {
        return false;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .unwrap_or(line.len());
    let after = &line[end..];
    is_lambda_list_keyword_name(&line[..end])
        && (after.is_empty() || after.starts_with(char::is_whitespace))
}

/// Whether `name` (with its leading `&`) is a lambda-list keyword.
fn is_lambda_list_keyword_name(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "&optional"
            | "&rest"
            | "&key"
            | "&body"
            | "&whole"
            | "&aux"
            | "&environment"
            | "&allow-other-keys"
    )
}

/// Whether `d` is a lambda-list keyword (`&optional`, `&rest`, `&key`, `&body`,
/// `&whole`, `&aux`, `&environment`, `&allow-other-keys`).
fn is_lambda_list_keyword(d: &Datum) -> bool {
    matches!(&d.kind, DatumKind::Symbol(s) if is_lambda_list_keyword_name(s))
}

// --- loop -----------------------------------------------------------------

/// Whether `list`'s head is `loop` (or `cl-loop`), triggering the loop
/// special-case. Common Lisp is case-insensitive, so the name is matched folded.
fn is_loop_form(list: &Datum) -> bool {
    let DatumKind::List { items, .. } = &list.kind else {
        return false;
    };
    matches!(head_name(items).as_deref(), Some("loop") | Some("cl-loop"))
}

/// `common-lisp-loop-part-indentation` — clauses of a `loop` form.
fn loop_indent(cols: &Cols, source: &str, list: &Datum, offset: usize) -> i64 {
    let loop_col = cols.col(list.span.start as usize) as i64;
    let DatumKind::List { items, .. } = &list.kind else {
        return loop_col;
    };
    // Extended loop: the first clause is a keyword/symbol (`for`, `with`, …).
    let extended = matches!(
        items.get(1).map(|d| &d.kind),
        Some(DatumKind::Symbol(_)) | Some(DatumKind::Keyword(_))
    );
    if !extended {
        return loop_col + SIMPLE_LOOP_INDENTATION;
    }
    if line_is_loop_keyword(source, offset) {
        loop_col + LOOP_KEYWORD_INDENTATION
    } else {
        loop_col + LOOP_FORMS_INDENTATION
    }
}

/// Whether the indent line opens with a loop keyword (`:?word`) or a comment —
/// Emacs's `^\s-*\(:?\sw+\|;\)`.
fn line_is_loop_keyword(source: &str, offset: usize) -> bool {
    match line_first_char(source, offset) {
        Some(';') => true,
        Some(':') => true,
        Some(c) => c.is_alphanumeric(),
        None => false,
    }
}

// --- small source helpers -------------------------------------------------

/// The chain of lists strictly containing `offset`, outermost first (innermost
/// last) — the levels `backward-up-list` walks. Mirrors [`super::container_at`]'s
/// descent (into list items and dotted tails; reader prefixes are transparent).
fn containing_chain<'a, 't>(data: &'a [Datum<'t>], offset: usize) -> Vec<&'a Datum<'t>> {
    let mut out = Vec::new();
    collect_chain(data, offset, &mut out);
    out
}

fn collect_chain<'a, 't>(data: &'a [Datum<'t>], offset: usize, out: &mut Vec<&'a Datum<'t>>) {
    for d in data {
        let (start, end) = (d.span.start as usize, d.span.end as usize);
        if start < offset && offset < end {
            match &d.kind {
                DatumKind::List { items, tail, .. } => {
                    out.push(d);
                    let before = out.len();
                    collect_chain(items, offset, out);
                    if out.len() == before {
                        if let Some(t) = tail {
                            if (t.span.start as usize) < offset && offset < (t.span.end as usize) {
                                collect_chain(std::slice::from_ref(t), offset, out);
                            }
                        }
                    }
                }
                DatumKind::Prefixed { inner, .. } => {
                    collect_chain(std::slice::from_ref(inner), offset, out);
                }
                _ => {}
            }
            return;
        }
    }
}

/// The head symbol/keyword name of a list, case-folded (Common Lisp is
/// case-insensitive). Keywords keep their leading `:`. Non-symbol heads (lists,
/// strings, numbers) return `None` — Emacs's `looking-at "\\sw\\|\\s_"` gate.
fn head_name(items: &[Datum]) -> Option<String> {
    match items.first().map(|d| &d.kind) {
        // A keyword's text already carries its leading `:` (`:method`).
        Some(DatumKind::Symbol(s)) => Some(s.to_lowercase()),
        Some(DatumKind::Keyword(k)) => Some(k.to_lowercase()),
        _ => None,
    }
}

/// Strip a package prefix, mirroring Emacs's `:[^:]+` match: the substring after
/// the first colon that is followed by a non-colon (so `cl:defconstant` →
/// `defconstant`, `ppcre::scan` → `scan`). Returns `None` when there is no such
/// prefix to strip.
fn strip_package_prefix(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    let colon = (0..bytes.len()).find(|&i| bytes[i] == b':' && bytes.get(i + 1) != Some(&b':'))?;
    // Require at least one non-colon char after the colon (`:[^:]+`).
    let rest = &name[colon + 1..];
    if rest.is_empty() || rest.starts_with(':') {
        None
    } else {
        Some(rest)
    }
}

/// Whether `name` begins with `with-`, `without-`, or `do-` — the backwards-
/// compatibility heuristic that indents such macros like `(&lambda &body)`.
fn is_with_or_do_prefixed(name: &str) -> bool {
    name.starts_with("with-") || name.starts_with("without-") || name.starts_with("do-")
}

/// The byte immediately before `pos`, as a char (the reader-prefix bytes we
/// test — `'` `` ` `` `,` `@` `#` — are all ASCII).
fn byte_before(source: &str, pos: usize) -> Option<char> {
    pos.checked_sub(1)
        .and_then(|i| source.as_bytes().get(i))
        .map(|&b| b as char)
}

/// `'(…)` — a quoted list (indent as data), and not `#'(…)`.
fn is_quote_data(source: &str, open: usize) -> bool {
    byte_before(source, open) == Some('\'')
        && byte_before(source, open.wrapping_sub(1)) != Some('#')
}

/// `,(…)` or `,@(…)` — a backquote substitution.
fn is_comma_substitution(source: &str, open: usize) -> bool {
    match byte_before(source, open) {
        Some(',') => true,
        Some('@') => byte_before(source, open.wrapping_sub(1)) == Some(','),
        _ => false,
    }
}

/// `#(…)` — a reader-macro list (vector etc.).
fn is_hash(source: &str, open: usize) -> bool {
    byte_before(source, open) == Some('#')
}

/// The first non-whitespace char of the line containing `offset`.
fn line_first_char(source: &str, offset: usize) -> Option<char> {
    source[offset..].lines().next()?.trim_start().chars().next()
}

/// Whether the indent line opens with a `tagbody` tag (a symbol/number) rather
/// than a form — Emacs's `looking-at "\\sw\\|\\s_"`. Anything opening a list,
/// string, or reader prefix is a body form.
fn line_starts_with_tag(source: &str, offset: usize) -> bool {
    match line_first_char(source, offset) {
        Some('(') | Some(')') | Some('"') | Some('\'') | Some('`') | Some(',') | Some(';')
        | Some('#') => false,
        Some(_) => true,
        None => false,
    }
}

/// The bundled standard indent table — `cl-indent.el`'s alist, with `. symbol`
/// aliases resolved and `defun` expanded to `(4 &lambda &body)`. Names are
/// case-folded. Returns the method for `name`, or `None` if unlisted.
fn method_for(name: &str) -> Option<Spec> {
    use Elem::{Body, Int, Lambda, Nil, Rest, Whole};
    // `(&whole 2 &rest 1)` — the common clause sublist.
    let whole2_rest1 = || Elem::List(vec![Whole, Int(2), Rest, Int(1)]);
    let spec = match name {
        // integers
        "block"
        | "catch"
        | "eval-when"
        | "locally"
        | "multiple-value-prog1"
        | "prog1"
        | "throw"
        | "unless"
        | "when" => Spec::Int(1),
        "prog2" => Spec::Int(2),
        "progn" | "return" => Spec::Int(0),

        // (4 …) families
        "case" | "ccase" | "ecase" | "typecase" | "etypecase" | "ctypecase" => {
            Spec::List(vec![Int(4), Rest, whole2_rest1()])
        }
        "cond" => Spec::List(vec![Rest, whole2_rest1()]),
        "defvar" | "defconstant" | "defparameter" => Spec::List(vec![Int(4), Int(2), Int(2)]),
        "defcustom" | "defconst" => Spec::List(vec![Int(4), Int(2), Int(2), Int(2)]),
        "defclass" | "define-condition" => {
            Spec::List(vec![Int(6), Int(4), whole2_rest1(), whole2_rest1()])
        }
        "define-modify-macro" | "with-compilation-unit" => Spec::List(vec![Lambda, Body]),
        "defsetf" => Spec::List(vec![Int(4), Lambda, Int(4), Body]),
        "defun"
        | "defgeneric"
        | "define-setf-method"
        | "define-setf-expander"
        | "defmacro"
        | "defsubst"
        | "deftype" => Spec::List(defun_method()),
        "defpackage" => Spec::List(vec![Int(4), Int(2)]),
        "defstruct" => Spec::List(vec![
            Elem::List(vec![Whole, Int(4), Rest, whole2_rest1()]),
            Rest,
            whole2_rest1(),
        ]),
        "destructuring-bind"
        | "multiple-value-bind"
        | "with-accessors"
        | "with-condition-restarts"
        | "with-slots" => Spec::List(vec![
            Elem::List(vec![Whole, Int(6), Rest, Int(1)]),
            Int(4),
            Body,
        ]),
        "dolist" | "dotimes" => {
            Spec::List(vec![Elem::List(vec![Whole, Int(4), Int(2), Int(1)]), Body])
        }
        "flet" | "labels" | "macrolet" | "generic-flet" | "generic-labels" => Spec::List(vec![
            Elem::List(vec![
                Whole,
                Int(4),
                Rest,
                Elem::List(vec![Whole, Int(1), Lambda, Body]),
            ]),
            Body,
        ]),
        "handler-case" | "restart-case" => Spec::List(vec![
            Int(4),
            Rest,
            Elem::List(vec![Whole, Int(2), Lambda, Body]),
        ]),
        // Both `if` entries are `put`; the later `(&rest nil)` (then/else equally
        // indented) wins.
        "if" => Spec::List(vec![Rest, Nil]),
        "lambda" => Spec::List(vec![Lambda, Rest, Elem::Fn(Named::LambdaHack)]),
        "let" | "let*" | "compiler-let" | "handler-bind" | "restart-bind" | "symbol-macrolet" => {
            Spec::List(vec![
                Elem::List(vec![
                    Whole,
                    Int(4),
                    Rest,
                    Elem::List(vec![Whole, Int(1), Int(1), Int(2)]),
                ]),
                Body,
            ])
        }
        ":method" => Spec::List(vec![Lambda, Body]),
        "multiple-value-call" => Spec::List(vec![Int(4), Body]),
        "multiple-value-setq" | "multiple-value-setf" => Spec::List(vec![Int(4), Int(2)]),
        "pprint-logical-block" | "with-output-to-string" => Spec::List(vec![Int(4), Int(2)]),
        "print-unreadable-object" => Spec::List(vec![
            Elem::List(vec![Whole, Int(4), Int(1), Rest, Int(1)]),
            Body,
        ]),
        "prog" | "prog*" => Spec::List(vec![Lambda, Rest, Elem::Fn(Named::Tagbody)]),
        "progv" => Spec::List(vec![Int(4), Int(4), Body]),
        "return-from" => Spec::List(vec![Nil, Body]),
        "unwind-protect" => Spec::List(vec![Int(5), Body]),
        "with-standard-io-syntax" => Spec::List(vec![Int(2)]),

        // named function methods
        "defmethod" => Spec::Fn(Named::Defmethod),
        "do" | "do*" => Spec::Fn(Named::Do),
        "tagbody" => Spec::Fn(Named::Tagbody),

        _ => return None,
    };
    Some(spec)
}

#[cfg(test)]
mod tests {
    use crate::config::FormatConfig;
    use lispexp::Dialect;

    /// Reindent flat Common Lisp and compare to Emacs's canonical output
    /// (captured from `lisp-mode` with `lisp-indent-function` =
    /// `common-lisp-indent-function`, `indent-tabs-mode` nil). Covers the forms
    /// that diverge from the Emacs Lisp engine: `defun`/`&body`, `let` bindings,
    /// `case`/`cond` clause alignment, CL `if` (then/else equal), nested `flet`,
    /// `handler-case`, `destructuring-bind`, and backquote-as-code.
    #[test]
    fn matches_emacs_on_common_forms() {
        let input = "\
(defun foo (x)
(bar x)
(baz x))
(let ((a 1)
(b 2))
(+ a b))
(case x
(1 one)
(2 two))
(cond ((evenp x)
(foo))
((oddp x)
(bar)))
(if test
then
else)
(flet ((sq (n)
(* n n)))
(sq 3))
(handler-case
(risky)
(error (e)
(report e)))
(destructuring-bind (a b)
lst
(list a b))
(defmacro my-when (test &body body)
`(if ,test
(progn ,@body)))
";
        let expected = "\
(defun foo (x)
  (bar x)
  (baz x))
(let ((a 1)
      (b 2))
  (+ a b))
(case x
  (1 one)
  (2 two))
(cond ((evenp x)
       (foo))
      ((oddp x)
       (bar)))
(if test
    then
    else)
(flet ((sq (n)
         (* n n)))
  (sq 3))
(handler-case
    (risky)
  (error (e)
    (report e)))
(destructuring-bind (a b)
    lst
  (list a b))
(defmacro my-when (test &body body)
  `(if ,test
       (progn ,@body)))
";
        let out = crate::format::format(input, &FormatConfig::default(), Dialect::CommonLisp);
        assert_eq!(out, expected);
    }

    /// The specialised methods: simple vs extended `loop`, `do` clauses,
    /// `dolist`, backquote substitution body alignment with a package-qualified
    /// head (`cl:defconstant` → `defconstant`, `(4 2 2)`), and lambda-list
    /// keyword-parameter alignment (`&key` continuation at keyword + 2). Golden
    /// captured from Emacs.
    #[test]
    fn matches_emacs_on_specialised_forms() {
        let input = "\
(loop for i from 1 to 10
collect i
when (evenp i)
do (print i))
(do ((i 0 (1+ i)))
((= i 10))
(process i))
(dolist (x items)
(print x))
`(cl:defconstant ,name
,value
,@(when doc (list doc)))
(defun create (regex &key case-insensitive
multi-line
extended)
(scan regex))
";
        let expected = "\
(loop for i from 1 to 10
      collect i
      when (evenp i)
      do (print i))
(do ((i 0 (1+ i)))
    ((= i 10))
  (process i))
(dolist (x items)
  (print x))
`(cl:defconstant ,name
   ,value
   ,@(when doc (list doc)))
(defun create (regex &key case-insensitive
                       multi-line
                       extended)
  (scan regex))
";
        let out = crate::format::format(input, &FormatConfig::default(), Dialect::CommonLisp);
        assert_eq!(out, expected);
    }

    /// A `defmethod` with a qualifier (`:after`) indents its lambda list and body
    /// by `lisp-indent-defmethod`; a bare `defmethod` indents like `defun`. This
    /// output matches a real Emacs on a properly-structured file — the body at
    /// `defun` depth, `&allow-other-keys` at its keyword + 2. (Emacs's *flat*
    /// reindent disagrees only because `lisp-indent-defmethod`'s
    /// `beginning-of-defun` mis-scans fully de-indented input; see the harness
    /// note in `docs/dev/formatter.md`.)
    #[test]
    fn defmethod_is_a_fixed_point() {
        let formatted = "\
(defmethod initialize-instance :after ((data data) &key streamspec
                                                     &allow-other-keys)
  (let ((c-data 1))
    (foo c-data)))
(defmethod print-object ((x point) stream)
  (princ x stream))
";
        let out = crate::format::format(formatted, &FormatConfig::default(), Dialect::CommonLisp);
        assert_eq!(out, formatted);
    }

    /// The Common Lisp engine only rewrites leading whitespace, so an
    /// already-formatted file is a fixed point.
    #[test]
    fn already_formatted_is_a_fixed_point() {
        let formatted = "(defun foo (x)\n  (bar x))\n";
        let out = crate::format::format(formatted, &FormatConfig::default(), Dialect::CommonLisp);
        assert_eq!(out, formatted);
    }
}
