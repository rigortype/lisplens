//! Semantic refactoring procedures (ADR-0032): atomic, parse-safe,
//! self-verifying operations that internalize the multi-step agent idiom
//! (`refs` → `line edit` batch → `refs` re-verify) into one call. First member:
//! whole-file symbol [`rename_symbol_in_file`]. Each is a *composition* over
//! [`crate::structural`] plus the safety pipeline (splice → reindent →
//! validate-then-write), so it adds surface without new edit machinery.

use std::ops::Range;
use std::path::Path;

use lispexp::{parse, Class, Datum, DatumKind, Options, Walk};

use crate::edit::{splice_tracked, Edit, SpliceError};
use crate::format::{has_native_engine, reindent, Touched};
use crate::hash::file_hash;
use crate::write::{verify_and_write, WriteError};
use crate::Dialect;

/// The result of a successful [`rename_symbol_in_file`].
#[derive(Debug)]
pub struct RenameOutcome {
    /// How many occurrences were rewritten (the post-condition: exactly this
    /// many, and zero of `from` remain — the rename is exhaustive by
    /// construction).
    pub renamed: usize,
    /// The file hash after the rename, over the reindented content (ADR-0008).
    pub new_file_hash: String,
}

/// Why a rename was refused. No partial write ever happens on an error.
#[derive(Debug)]
pub enum RenameError {
    /// `from` and `to` are the same symbol — nothing to do.
    Unchanged,
    /// The symbol `from` does not occur in the file, so a typo'd name is
    /// reported rather than silently succeeding as a no-op.
    NoOccurrences(String),
    /// The edits could not be spliced.
    Splice(SpliceError),
    /// The safe write was refused (would introduce parse errors, or I/O).
    Write(WriteError),
    /// A filesystem error reading the file.
    Io(std::io::Error),
}

impl std::fmt::Display for RenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameError::Unchanged => write!(f, "`from` and `to` are the same symbol"),
            RenameError::NoOccurrences(s) => write!(f, "no occurrences of symbol `{s}`"),
            RenameError::Splice(e) => write!(f, "splice failed: {e:?}"),
            RenameError::Write(e) => write!(f, "{e:?}"),
            RenameError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Rename every exact occurrence of the symbol `from` to `to` across `path`
/// (ADR-0032).
///
/// Symbol-exact in code **and** data — never a substring, a keyword (`:from`),
/// or text inside a string or comment (that is [`crate::structural::rename`]'s
/// guarantee), so sibling symbols like `from-bar` are untouched with no
/// hand-built lookahead. The rewritten sites are spliced, the touched top-level
/// forms are reindented for dialects with a faithful native engine
/// ([`has_native_engine`] — a rename changes token width, so alignment under the
/// symbol shifts), and the result is validated (reject new parse errors,
/// ADR-0005) before an atomic write. Returns the site count and new file hash.
pub fn rename_symbol_in_file(
    path: &Path,
    from: &str,
    to: &str,
    dialect: Dialect,
) -> Result<RenameOutcome, RenameError> {
    if from == to {
        return Err(RenameError::Unchanged);
    }
    let source = std::fs::read_to_string(path).map_err(RenameError::Io)?;
    let options = Options::for_dialect(dialect);
    let parsed = parse(&source, &options);

    let mut edits = Vec::new();
    for datum in &parsed.data {
        edits.extend(crate::structural::rename(datum, from, to));
    }
    let renamed = edits.len();
    if renamed == 0 {
        return Err(RenameError::NoOccurrences(from.to_string()));
    }

    let expected = file_hash(source.as_bytes());
    let (spliced, spans) = splice_tracked(&source, edits).map_err(RenameError::Splice)?;

    // Reindent the touched top-level forms (native-engine dialects only); other
    // dialects stay verbatim (ADR-0027).
    let new_content = if has_native_engine(dialect) {
        let config = crate::config::resolve(path, &spliced);
        reindent(
            &spliced,
            &config,
            dialect,
            None,
            Touched {
                expand: &spans,
                exact: &[],
            },
        )
    } else {
        spliced
    };

    verify_and_write(path, &expected, &new_content, &options).map_err(RenameError::Write)?;
    Ok(RenameOutcome {
        renamed,
        new_file_hash: file_hash(new_content.as_bytes()),
    })
}

/// The result of a successful [`inline_definition_in_file`].
#[derive(Debug)]
pub struct InlineOutcome {
    /// How many call sites were expanded.
    pub inlined: usize,
    /// The file hash after inlining, over the reindented content (ADR-0008).
    pub new_file_hash: String,
}

/// Why an inline was refused. No partial write ever happens on an error.
#[derive(Debug)]
pub enum InlineError {
    /// No definition of `name` was found in the file.
    NotFound(String),
    /// `name` is defined more than once — which one to inline is ambiguous.
    Ambiguous(String),
    /// `name` is defined, but not as an inlinable function (a macro, a variable,
    /// a complex lambda list, a recursive or empty body, …). The string explains.
    NotInlinable(String),
    /// A call site has the wrong number of arguments for the definition.
    ArityMismatch {
        /// Parameters the definition declares.
        expected: usize,
        /// Arguments the call passes.
        found: usize,
        /// 1-based line of the offending call.
        line: usize,
    },
    /// `name` is defined but never called, so there is nothing to inline.
    NoCallSites(String),
    /// The edits could not be spliced.
    Splice(SpliceError),
    /// The safe write was refused.
    Write(WriteError),
    /// A filesystem error.
    Io(std::io::Error),
}

impl std::fmt::Display for InlineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InlineError::NotFound(n) => write!(f, "no definition of `{n}` found"),
            InlineError::Ambiguous(n) => write!(f, "`{n}` is defined more than once"),
            InlineError::NotInlinable(r) => write!(f, "cannot inline: {r}"),
            InlineError::ArityMismatch {
                expected,
                found,
                line,
            } => write!(
                f,
                "call at line {line} passes {found} argument(s) but the definition takes {expected}"
            ),
            InlineError::NoCallSites(n) => write!(f, "`{n}` is never called"),
            InlineError::Splice(e) => write!(f, "splice failed: {e:?}"),
            InlineError::Write(e) => write!(f, "{e:?}"),
            InlineError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Inline every call of the function `name` with its body, substituting arguments
/// (ADR-0032) — the benchmark's inline-expand as one atomic, parse-safe step.
///
/// Restricted to what is provably safe: `name` must be a single, function-like
/// definition (`defun`/`defsubst`/`cl-defun`/`cl-defsubst`, or a Scheme
/// `(define (name …) …)`), with a required-only lambda list and a non-recursive
/// body. Each call `(name a…)` is replaced by the body directly when the function
/// is niladic, else by `(let ((p a) …) body)` — a `let` preserving argument
/// evaluation order and single-evaluation, which is exactly what `defsubst`
/// compiles to. Only the outermost of nested same-name calls is expanded per run
/// (a nested call stays inside the `let` binding, and a second run expands it);
/// macros, variables, complex lambda lists, recursion, and arity mismatches are
/// **refused** with a reason rather than mis-expanded. The definition itself is
/// left in place. Touched forms are reindented (native engines) and the result
/// validated before an atomic write.
pub fn inline_definition_in_file(
    path: &Path,
    name: &str,
    dialect: Dialect,
) -> Result<InlineOutcome, InlineError> {
    let source = std::fs::read_to_string(path).map_err(InlineError::Io)?;
    let options = Options::for_dialect(dialect);
    let parsed = parse(&source, &options);

    // Find the single inlinable definition of `name`.
    let mut matched: Option<(&Datum, Vec<&str>, Vec<&Datum>)> = None;
    let mut refuse: Option<String> = None;
    let mut def_count = 0usize;
    for form in &parsed.data {
        match classify_def(form, name) {
            None => {}
            Some(Err(reason)) => {
                def_count += 1;
                refuse.get_or_insert(reason);
            }
            Some(Ok((params, body))) => {
                def_count += 1;
                matched = Some((form, params, body));
            }
        }
    }
    if def_count == 0 {
        return Err(InlineError::NotFound(name.to_string()));
    }
    if def_count > 1 {
        return Err(InlineError::Ambiguous(name.to_string()));
    }
    let Some((def_form, params, body_forms)) = matched else {
        return Err(InlineError::NotInlinable(
            refuse.unwrap_or_else(|| "not an inlinable function".to_string()),
        ));
    };
    if body_forms.iter().any(|d| datum_references(d, name)) {
        return Err(InlineError::NotInlinable(format!("`{name}` is recursive")));
    }
    let body_start = body_forms[0].span.start as usize;
    let body_end = body_forms.last().unwrap().span.end as usize;
    let body_text = &source[body_start..body_end];
    let body_multi = body_forms.len() > 1;

    // Collect code-position call sites `(name a…)` outside the definition form.
    let (def_start, def_end) = (def_form.span.start as usize, def_form.span.end as usize);
    let mut calls: Vec<(Range<usize>, Vec<Range<usize>>)> = Vec::new();
    lispexp::walk(&parsed.data, |datum, class| {
        if class == Class::Code {
            if let DatumKind::List { items, .. } = &datum.kind {
                if items.first().and_then(as_sym) == Some(name) {
                    let (s, e) = (datum.span.start as usize, datum.span.end as usize);
                    if !(def_start <= s && e <= def_end) {
                        let args = items[1..]
                            .iter()
                            .map(|a| a.span.start as usize..a.span.end as usize)
                            .collect();
                        calls.push((s..e, args));
                    }
                }
            }
        }
        Walk::Descend
    });
    if calls.is_empty() {
        return Err(InlineError::NoCallSites(name.to_string()));
    }

    // Keep only outermost calls (a nested same-name call is contained in an
    // outer one; editing both would overlap). Outermost = start not inside a
    // previously kept call.
    calls.sort_by_key(|(r, _)| (r.start, std::cmp::Reverse(r.end)));
    let mut kept: Vec<(Range<usize>, Vec<Range<usize>>)> = Vec::new();
    let mut covered_to = 0usize;
    for (r, args) in calls {
        if r.start >= covered_to {
            covered_to = r.end;
            kept.push((r, args));
        }
    }

    // Arity: every kept call must match the parameter count.
    for (r, args) in &kept {
        if args.len() != params.len() {
            return Err(InlineError::ArityMismatch {
                expected: params.len(),
                found: args.len(),
                line: source[..r.start].bytes().filter(|&b| b == b'\n').count() + 1,
            });
        }
    }

    let mut edits: Vec<Edit> = Vec::with_capacity(kept.len());
    for (r, args) in &kept {
        let text = if params.is_empty() {
            if body_multi {
                format!("(progn {body_text})")
            } else {
                body_text.to_string()
            }
        } else {
            let bindings: Vec<String> = params
                .iter()
                .zip(args)
                .map(|(p, a)| format!("({p} {})", &source[a.clone()]))
                .collect();
            format!("(let ({}) {body_text})", bindings.join(" "))
        };
        edits.push(Edit {
            range: r.clone(),
            text,
        });
    }
    let inlined = edits.len();

    let expected = file_hash(source.as_bytes());
    let (spliced, spans) = splice_tracked(&source, edits).map_err(InlineError::Splice)?;
    let new_content = if has_native_engine(dialect) {
        let config = crate::config::resolve(path, &spliced);
        reindent(
            &spliced,
            &config,
            dialect,
            None,
            Touched {
                expand: &spans,
                exact: &[],
            },
        )
    } else {
        spliced
    };
    verify_and_write(path, &expected, &new_content, &options).map_err(InlineError::Write)?;
    Ok(InlineOutcome {
        inlined,
        new_file_hash: file_hash(new_content.as_bytes()),
    })
}

/// The symbol text of `d`, if it is a symbol.
fn as_sym<'a>(d: &Datum<'a>) -> Option<&'a str> {
    match &d.kind {
        DatumKind::Symbol(s) => Some(s),
        _ => None,
    }
}

/// Whether `name` occurs as a symbol anywhere in `d` (for the recursion guard).
fn datum_references(d: &Datum, name: &str) -> bool {
    match &d.kind {
        DatumKind::Symbol(s) => *s == name,
        DatumKind::List { items, tail, .. } => {
            items.iter().any(|i| datum_references(i, name))
                || tail.as_deref().is_some_and(|t| datum_references(t, name))
        }
        DatumKind::Prefixed { inner, arg, .. } => {
            datum_references(inner, name)
                || arg.as_deref().is_some_and(|a| datum_references(a, name))
        }
        _ => false,
    }
}

/// An inlinable definition's parts: its required parameters and its body forms
/// (docstring/`declare` already stripped).
type DefBody<'a> = (Vec<&'a str>, Vec<&'a Datum<'a>>);

/// Classify a top-level `form` against the target `name`:
/// `Some(Ok((params, body)))` — it is an inlinable function definition of `name`;
/// `Some(Err(reason))` — it defines `name` but cannot be inlined;
/// `None` — it does not define `name`.
fn classify_def<'a>(form: &'a Datum<'a>, name: &str) -> Option<Result<DefBody<'a>, String>> {
    const FN_DEFS: &[&str] = &["defun", "defsubst", "cl-defun", "cl-defsubst"];
    const MACRO_DEFS: &[&str] = &[
        "defmacro",
        "cl-defmacro",
        "define-syntax",
        "define-syntax-rule",
        "defmacro!",
    ];
    const VAR_DEFS: &[&str] = &[
        "defvar",
        "defvar-local",
        "defconst",
        "defconstant",
        "defcustom",
        "defparameter",
    ];

    let DatumKind::List { items, .. } = &form.kind else {
        return None;
    };
    let head = as_sym(items.first()?)?;

    if FN_DEFS.contains(&head) {
        if as_sym(items.get(1)?) != Some(name) {
            return None;
        }
        let params = match items.get(2).map(parse_params) {
            Some(Ok(p)) => p,
            Some(Err(e)) => return Some(Err(e)),
            None => return Some(Err("definition has no argument list".into())),
        };
        return Some(finish_body(name, &items[3..], params));
    }
    if MACRO_DEFS.contains(&head) {
        return (as_sym(items.get(1)?) == Some(name))
            .then(|| Err(format!("`{name}` is a macro")));
    }
    if VAR_DEFS.contains(&head) {
        return (as_sym(items.get(1)?) == Some(name))
            .then(|| Err(format!("`{name}` is a variable, not a function")));
    }
    if head == "define" || head == "define*" {
        return match &items.get(1)?.kind {
            // `(define (name params…) body…)` — a function.
            DatumKind::List { items: sig, .. } => {
                if as_sym(sig.first()?) != Some(name) {
                    return None;
                }
                match parse_params_from(&sig[1..]) {
                    Ok(params) => Some(finish_body(name, &items[2..], params)),
                    Err(e) => Some(Err(e)),
                }
            }
            // `(define name value)` — a value, not inlinable as a function.
            DatumKind::Symbol(s) if *s == name => {
                Some(Err(format!("`{name}` is a value, not a function")))
            }
            _ => None,
        };
    }
    None
}

/// Strip a leading docstring / `(declare …)` off body forms and reject an empty
/// body, packaging the `(params, body)` result.
fn finish_body<'a>(
    name: &str,
    forms: &'a [Datum<'a>],
    params: Vec<&'a str>,
) -> Result<DefBody<'a>, String> {
    let mut body: Vec<&Datum> = forms.iter().collect();
    // A leading string is a docstring only when it is not the sole body form.
    if body.len() > 1 && matches!(body[0].kind, DatumKind::Str(_)) {
        body.remove(0);
    }
    if let Some(first) = body.first() {
        if let DatumKind::List { items, .. } = &first.kind {
            if items.first().and_then(as_sym) == Some("declare") {
                body.remove(0);
            }
        }
    }
    if body.is_empty() {
        return Err(format!("`{name}` has an empty body"));
    }
    Ok((params, body))
}

/// Parse a lambda list (a list of required parameters). Rejects `&`-keywords and
/// non-symbol (destructuring) parameters — only required parameters inline.
fn parse_params<'a>(arglist: &'a Datum<'a>) -> Result<Vec<&'a str>, String> {
    match &arglist.kind {
        DatumKind::List { items, .. } => parse_params_from(items),
        _ => Err("argument list is not a list".into()),
    }
}

fn parse_params_from<'a>(items: &'a [Datum<'a>]) -> Result<Vec<&'a str>, String> {
    let mut params = Vec::with_capacity(items.len());
    for it in items {
        match &it.kind {
            DatumKind::Symbol(s) if s.starts_with('&') => {
                return Err(format!("argument list has `{s}` (only required parameters inline)"));
            }
            DatumKind::Symbol(s) => params.push(*s),
            _ => return Err("argument list has a non-symbol parameter (destructuring)".into()),
        }
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(name: &str, content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn renames_symbol_exactly_leaving_siblings_and_text() {
        // `foo` occurs twice as a symbol (the defun name and the call); the
        // sibling `foo-bar`, the string "foo", and the `; foo` comment must all
        // survive.
        let (_d, path) = write_temp(
            "a.el",
            "(defun foo (x)          ; foo comment\n  (foo-bar (foo x) \"foo\"))\n",
        );
        let out = rename_symbol_in_file(&path, "foo", "qux", Dialect::EmacsLisp).unwrap();
        assert_eq!(out.renamed, 2);
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("(defun qux (x)"));
        assert!(result.contains("(qux x)"));
        assert!(result.contains("foo-bar")); // sibling untouched
        assert!(result.contains("\"foo\"")); // string untouched
        assert!(result.contains("; foo comment")); // comment untouched
        assert!(!result.contains("(foo ")); // no stray old symbol
    }

    #[test]
    fn renames_across_dialects_including_quoted_data() {
        // Scheme: rename `x` — the definition, the body use, and the quoted datum
        // `'x` (data) all move; `xs` (a sibling) does not.
        let (_d, path) = write_temp("a.scm", "(define (f x xs)\n  (cons 'x (g x xs)))\n");
        let out = rename_symbol_in_file(&path, "x", "y", Dialect::Scheme).unwrap();
        assert_eq!(out.renamed, 3);
        let result = std::fs::read_to_string(&path).unwrap();
        assert_eq!(result, "(define (f y xs)\n  (cons 'y (g y xs)))\n");
    }

    #[test]
    fn missing_symbol_is_reported_not_a_silent_noop() {
        let (_d, path) = write_temp("a.el", "(defun foo () 1)\n");
        let err = rename_symbol_in_file(&path, "nope", "x", Dialect::EmacsLisp).unwrap_err();
        assert!(matches!(err, RenameError::NoOccurrences(_)));
        // file untouched
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(defun foo () 1)\n");
    }

    #[test]
    fn same_from_and_to_is_refused() {
        let (_d, path) = write_temp("a.el", "(defun foo () 1)\n");
        assert!(matches!(
            rename_symbol_in_file(&path, "foo", "foo", Dialect::EmacsLisp),
            Err(RenameError::Unchanged)
        ));
    }

    #[test]
    fn inlines_a_niladic_defsubst_into_call_sites() {
        // The benchmark's inline-expand: a niladic accessor `defsubst` expands to
        // its body verbatim; the definition is left in place.
        let (_d, path) = write_temp(
            "a.el",
            "(defsubst in-string-p () (nth 3 (syntax-ppss)))\n(when (in-string-p)\n  (do-thing))\n",
        );
        let out = inline_definition_in_file(&path, "in-string-p", Dialect::EmacsLisp).unwrap();
        assert_eq!(out.inlined, 1);
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("(when (nth 3 (syntax-ppss))"), "{result}");
        assert!(result.contains("(defsubst in-string-p ()")); // definition kept
    }

    #[test]
    fn inlines_params_with_a_let_preserving_evaluation() {
        // A parameterized function inlines as a `let` (single-eval, order-safe),
        // even when the argument has side effects.
        let (_d, path) = write_temp("a.el", "(defun sq (n) (* n n))\n(list (sq (pop xs)))\n");
        let out = inline_definition_in_file(&path, "sq", Dialect::EmacsLisp).unwrap();
        assert_eq!(out.inlined, 1);
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("(let ((n (pop xs))) (* n n))"), "{result}");
    }

    #[test]
    fn inlines_a_scheme_define() {
        let (_d, path) = write_temp("a.scm", "(define (inc x) (+ x 1))\n(display (inc 41))\n");
        let out = inline_definition_in_file(&path, "inc", Dialect::Scheme).unwrap();
        assert_eq!(out.inlined, 1);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("(display (let ((x 41)) (+ x 1)))"));
    }

    #[test]
    fn refuses_macros_recursion_arity_and_missing() {
        let (_d, m) = write_temp("m.el", "(defmacro mac (x) `(+ ,x 1))\n(mac 2)\n");
        assert!(matches!(
            inline_definition_in_file(&m, "mac", Dialect::EmacsLisp),
            Err(InlineError::NotInlinable(_))
        ));
        let (_d, r) = write_temp("r.el", "(defun rec (n) (rec (1- n)))\n(rec 3)\n");
        assert!(matches!(
            inline_definition_in_file(&r, "rec", Dialect::EmacsLisp),
            Err(InlineError::NotInlinable(_))
        ));
        let (_d, a) = write_temp("a.el", "(defun f (x) (+ x 1))\n(f 1 2)\n");
        assert!(matches!(
            inline_definition_in_file(&a, "f", Dialect::EmacsLisp),
            Err(InlineError::ArityMismatch { expected: 1, found: 2, .. })
        ));
        let (_d, n) = write_temp("n.el", "(defun f (x) x)\n(g 1)\n");
        assert!(matches!(
            inline_definition_in_file(&n, "nope", Dialect::EmacsLisp),
            Err(InlineError::NotFound(_))
        ));
        // Refuses &-lambda-list keywords.
        let (_d, o) = write_temp("o.el", "(defun f (a &rest r) a)\n(f 1 2)\n");
        assert!(matches!(
            inline_definition_in_file(&o, "f", Dialect::EmacsLisp),
            Err(InlineError::NotInlinable(_))
        ));
    }

    #[test]
    fn drops_a_docstring_when_inlining() {
        let (_d, path) = write_temp("a.el", "(defun f () \"doc\" 42)\n(list (f))\n");
        inline_definition_in_file(&path, "f", Dialect::EmacsLisp).unwrap();
        let result = std::fs::read_to_string(&path).unwrap();
        // The call site got the body without the docstring; the definition
        // (still present) keeps its docstring.
        assert!(result.contains("(list 42)"), "{result}");
        assert!(!result.contains("(list \"doc\""), "{result}");
    }
}
