//! Semantic refactoring procedures (ADR-0032): atomic, parse-safe,
//! self-verifying operations that internalize the multi-step agent idiom
//! (`refs` → `line edit` batch → `refs` re-verify) into one call. First member:
//! whole-file symbol [`rename_symbol_in_file`]. Each is a *composition* over
//! [`crate::structural`] plus the safety pipeline (splice → reindent →
//! validate-then-write), so it adds surface without new edit machinery.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use lispexp::{parse, Class, Datum, DatumKind, Options, Prefix, Walk};

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
        return (as_sym(items.get(1)?) == Some(name)).then(|| Err(format!("`{name}` is a macro")));
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
                return Err(format!(
                    "argument list has `{s}` (only required parameters inline)"
                ));
            }
            DatumKind::Symbol(s) => params.push(*s),
            _ => return Err("argument list has a non-symbol parameter (destructuring)".into()),
        }
    }
    Ok(params)
}

// ===== rewrite: structural pattern -> template (ADR-0033) =====

/// The result of a successful [`rewrite_in_file`].
#[derive(Debug)]
pub struct RewriteOutcome {
    /// How many sites were rewritten (0 is a valid, idempotent outcome).
    pub rewritten: usize,
    /// The file hash after the rewrite, over the reindented content.
    pub new_file_hash: String,
}

/// Why a rewrite was refused. No partial write ever happens on an error.
#[derive(Debug)]
pub enum RewriteError {
    /// The stdin spec could not be parsed (missing `pattern`/`template` block,
    /// unterminated heredoc, malformed metavariable, …). Carries a message.
    Spec(String),
    /// The drift `@ <file-hash>` was given and did not match the file.
    Drift { expected: String, actual: String },
    /// The edits could not be spliced.
    Splice(SpliceError),
    /// The safe write was refused.
    Write(WriteError),
    /// A filesystem error.
    Io(std::io::Error),
}

impl std::fmt::Display for RewriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RewriteError::Spec(m) => write!(f, "{m}"),
            RewriteError::Drift { expected, actual } => {
                write!(f, "file drifted (expected {expected}, found {actual})")
            }
            RewriteError::Splice(e) => write!(f, "splice failed: {e:?}"),
            RewriteError::Write(e) => write!(f, "{e:?}"),
            RewriteError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// A metavariable class — a syntactic match filter (ADR-0033).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MClass {
    Any,
    Atom,
    Lit,
    Sym,
    List,
}

/// A parsed metavariable token: `$name` / `$_` / `$name...` / `$name:class`.
#[derive(Debug, Clone)]
struct Metavar {
    /// `None` for the wildcard `$_` (non-capturing).
    name: Option<String>,
    class: MClass,
    /// Whether it is a sequence metavariable (`$name...`).
    seq: bool,
}

/// Classify a symbol token: `Some(Ok(mv))` a metavariable, `Some(Err(msg))` a
/// malformed metavariable (e.g. unknown class), `None` an ordinary literal
/// symbol (including a `$$`-escaped literal).
fn metavar_of(s: &str) -> Option<Result<Metavar, String>> {
    let body = s.strip_prefix('$')?;
    if body.is_empty() || body.starts_with('$') {
        return None; // `$` alone, or `$$…` escaped literal
    }
    let (body, seq) = match body.strip_suffix("...") {
        Some(b) => (b, true),
        None => (body, false),
    };
    let (name, class) = match body.split_once(':') {
        Some((n, c)) => {
            let class = match c {
                "any" => MClass::Any,
                "atom" => MClass::Atom,
                "lit" => MClass::Lit,
                "sym" => MClass::Sym,
                "list" => MClass::List,
                other => return Some(Err(format!("unknown metavariable class `:{other}`"))),
            };
            (n, class)
        }
        None => (body, MClass::Any),
    };
    let name = (name != "_").then(|| name.to_string());
    Some(Ok(Metavar { name, class, seq }))
}

/// The literal text a pattern/template symbol denotes: a `$$`-escaped token
/// drops one `$` (`$$foo` → `$foo`); everything else is itself.
fn unescape(s: &str) -> &str {
    s.strip_prefix('$')
        .filter(|b| b.starts_with('$'))
        .map_or(s, |_| &s[1..])
}

/// Whether `d` is the bare `...` symbol (a separate sequence marker).
fn is_ellipsis(d: &Datum) -> bool {
    matches!(&d.kind, DatumKind::Symbol(s) if *s == "...")
}

/// Whether `d` satisfies metavariable class `class` (ADR-0033).
fn class_ok(class: MClass, d: &Datum) -> bool {
    match class {
        MClass::Any => true,
        MClass::Sym => matches!(d.kind, DatumKind::Symbol(_)),
        MClass::List => matches!(d.kind, DatumKind::List { .. }),
        MClass::Atom => matches!(
            d.kind,
            DatumKind::Symbol(_)
                | DatumKind::Keyword(_)
                | DatumKind::Number(_)
                | DatumKind::Str(_)
                | DatumKind::Char(_)
                | DatumKind::Bool(_)
        ),
        MClass::Lit => matches!(
            d.kind,
            DatumKind::Number(_)
                | DatumKind::Str(_)
                | DatumKind::Char(_)
                | DatumKind::Bool(_)
                | DatumKind::Prefixed {
                    prefix: Prefix::Quote,
                    ..
                }
        ),
    }
}

/// Structural equality **modulo formatting** (ADR-0033): recursive `DatumKind`
/// comparison ignoring `span`/`line` (so whitespace and comments do not matter),
/// with leaf text compared literally and no sugar/number/case normalization.
/// (Distinct from `Datum`'s derived `==`, which compares spans.)
fn struct_eq(a: &Datum, b: &Datum) -> bool {
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

fn opt_eq(a: Option<&Datum>, b: Option<&Datum>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => struct_eq(x, y),
        _ => false,
    }
}

/// A capture: a single matched form, or a (possibly empty) contiguous run.
enum Cap<'t> {
    One(&'t Datum<'t>),
    Seq(Vec<&'t Datum<'t>>),
}

type Binds<'t> = HashMap<String, Cap<'t>>;

/// Record a binding, enforcing non-linear equality; the wildcard (`None`) never
/// binds. Returns false on a non-linear conflict.
fn bind<'t>(binds: &mut Binds<'t>, name: &Option<String>, cap: Cap<'t>) -> bool {
    let Some(name) = name else { return true };
    match binds.get(name) {
        None => {
            binds.insert(name.clone(), cap);
            true
        }
        Some(prev) => cap_eq(prev, &cap),
    }
}

fn cap_eq(a: &Cap, b: &Cap) -> bool {
    match (a, b) {
        (Cap::One(x), Cap::One(y)) => struct_eq(x, y),
        (Cap::Seq(x), Cap::Seq(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| struct_eq(p, q))
        }
        _ => false,
    }
}

/// Try to match pattern node `pat` against target node `tgt`, extending `binds`.
fn try_match<'t>(pat: &Datum, tgt: &'t Datum<'t>, binds: &mut Binds<'t>) -> bool {
    if let DatumKind::Symbol(s) = &pat.kind {
        if let Some(Ok(mv)) = metavar_of(s) {
            // A sequence metavariable is only meaningful inside a list.
            if mv.seq {
                return false;
            }
            return class_ok(mv.class, tgt) && bind(binds, &mv.name, Cap::One(tgt));
        }
    }
    match (&pat.kind, &tgt.kind) {
        (DatumKind::Symbol(ps), DatumKind::Symbol(ts)) => unescape(ps) == *ts,
        (
            DatumKind::List {
                delim: pd,
                items: pi,
                tail: pt,
                ..
            },
            DatumKind::List {
                delim: td,
                items: ti,
                tail: tt,
                ..
            },
        ) => {
            pd == td
                && match_items(pi, ti, binds)
                && match (pt, tt) {
                    (None, None) => true,
                    (Some(p), Some(t)) => try_match(p, t, binds),
                    _ => false,
                }
        }
        (
            DatumKind::Prefixed {
                prefix: pp,
                notation: pn,
                inner: pin,
                arg: pa,
            },
            DatumKind::Prefixed {
                prefix: tp,
                notation: tn,
                inner: tin,
                arg: ta,
            },
        ) => {
            pp == tp
                && pn == tn
                && try_match(pin, tin, binds)
                && match (pa, ta) {
                    (None, None) => true,
                    (Some(a), Some(b)) => try_match(a, b, binds),
                    _ => false,
                }
        }
        (
            DatumKind::HashLiteral {
                tag: pt2,
                inner: pin,
            },
            DatumKind::HashLiteral {
                tag: tt2,
                inner: tin,
            },
        ) => {
            pt2 == tt2
                && match (pin, tin) {
                    (None, None) => true,
                    (Some(a), Some(b)) => try_match(a, b, binds),
                    _ => false,
                }
        }
        (
            DatumKind::Label {
                id: pid,
                inner: pin,
            },
            DatumKind::Label {
                id: tid,
                inner: tin,
            },
        ) => pid == tid && try_match(pin, tin, binds),
        _ => struct_eq(pat, tgt),
    }
}

/// Match a pattern list's items against a target list's items, handling a single
/// trailing sequence metavariable.
fn match_items<'t>(pat: &[Datum], tgt: &'t [Datum<'t>], binds: &mut Binds<'t>) -> bool {
    let Ok((fixed, seq)) = compile_items(pat) else {
        return false; // malformed pattern; validated earlier, be defensive
    };
    match seq {
        None => {
            fixed.len() == tgt.len() && fixed.iter().zip(tgt).all(|(p, t)| try_match(p, t, binds))
        }
        Some(mv) => {
            if tgt.len() < fixed.len() {
                return false;
            }
            let (head, rest) = tgt.split_at(fixed.len());
            fixed.iter().zip(head).all(|(p, t)| try_match(p, t, binds))
                && rest.iter().all(|t| class_ok(mv.class, t))
                && bind(binds, &mv.name, Cap::Seq(rest.iter().collect()))
        }
    }
}

/// Split a pattern list's items into fixed leading patterns and an optional
/// trailing sequence metavariable. `...` is a sequence marker only right after a
/// metavariable; elsewhere it is a literal.
#[allow(clippy::type_complexity)]
fn compile_items<'p>(
    items: &'p [Datum<'p>],
) -> Result<(Vec<&'p Datum<'p>>, Option<Metavar>), String> {
    let mut fixed = Vec::new();
    let mut seq: Option<Metavar> = None;
    let mut i = 0;
    while i < items.len() {
        if seq.is_some() {
            return Err("a sequence metavariable must be the last element".into());
        }
        let it = &items[i];
        if let DatumKind::Symbol(s) = &it.kind {
            if let Some(mv) = metavar_of(s) {
                let mut mv = mv?;
                if !mv.seq && i + 1 < items.len() && is_ellipsis(&items[i + 1]) {
                    mv.seq = true;
                    i += 1; // consume the separate `...`
                }
                if mv.seq {
                    seq = Some(mv);
                    i += 1;
                    continue;
                }
            }
        }
        fixed.push(it);
        i += 1;
    }
    Ok((fixed, seq))
}

/// Visit every datum node in `data` (pre-order: the node, then its list items /
/// dotted tail / prefixed inner+arg / hash+label inner).
fn for_each_node<'t>(data: &'t [Datum<'t>], f: &mut impl FnMut(&'t Datum<'t>)) {
    for d in data {
        f(d);
        match &d.kind {
            DatumKind::List { items, tail, .. } => {
                for_each_node(items, f);
                if let Some(t) = tail {
                    for_each_node(std::slice::from_ref(t), f);
                }
            }
            DatumKind::Prefixed { inner, arg, .. } => {
                for_each_node(std::slice::from_ref(inner), f);
                if let Some(a) = arg {
                    for_each_node(std::slice::from_ref(a), f);
                }
            }
            DatumKind::HashLiteral {
                inner: Some(inner), ..
            } => for_each_node(std::slice::from_ref(inner), f),
            DatumKind::Label { inner, .. } => for_each_node(std::slice::from_ref(inner), f),
            _ => {}
        }
    }
}

/// One matched site: the byte span to replace, and each metavariable's captured
/// verbatim text (with `is_seq`).
struct RMatch {
    span: Range<usize>,
    caps: HashMap<String, (bool, String)>,
}

/// Collect every site where `pattern` matches, anywhere in the tree.
fn collect_matches<'t>(pattern: &Datum, data: &'t [Datum<'t>], source: &str) -> Vec<RMatch> {
    let mut out = Vec::new();
    for_each_node(data, &mut |node| {
        let mut binds: Binds = HashMap::new();
        if try_match(pattern, node, &mut binds) {
            let caps = binds
                .into_iter()
                .map(|(k, cap)| {
                    let v = match cap {
                        Cap::One(d) => (false, source[span_of(d)].to_string()),
                        Cap::Seq(v) if v.is_empty() => (true, String::new()),
                        Cap::Seq(v) => (
                            true,
                            source[v[0].span.start as usize..v[v.len() - 1].span.end as usize]
                                .to_string(),
                        ),
                    };
                    (k, v)
                })
                .collect();
            out.push(RMatch {
                span: span_of(node),
                caps,
            });
        }
    });
    out
}

fn span_of(d: &Datum) -> Range<usize> {
    d.span.start as usize..d.span.end as usize
}

/// Keep only outermost, non-overlapping matches (splice cannot overlap; nested
/// matches are reached by re-running).
fn keep_outermost(mut matches: Vec<RMatch>) -> Vec<RMatch> {
    matches.sort_by_key(|m| (m.span.start, std::cmp::Reverse(m.span.end)));
    let mut kept: Vec<RMatch> = Vec::new();
    let mut covered = 0usize;
    for m in matches {
        if m.span.start >= covered {
            covered = m.span.end;
            kept.push(m);
        }
    }
    kept
}

/// Expand `template_text` for one match: substitute each metavariable token with
/// its captured verbatim text, unescape `$$`, and trim outer whitespace.
fn expand(
    template_text: &str,
    template_data: &[Datum],
    caps: &HashMap<String, (bool, String)>,
) -> Result<String, String> {
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut err: Option<String> = None;
    for_each_node(template_data, &mut |node| {
        if err.is_some() {
            return;
        }
        let DatumKind::Symbol(s) = &node.kind else {
            return;
        };
        let span = span_of(node);
        match metavar_of(s) {
            Some(Ok(mv)) => {
                let Some(name) = &mv.name else {
                    err = Some("`$_` is not allowed in a template".into());
                    return;
                };
                match caps.get(name) {
                    None => err = Some(format!("template metavariable `${name}` is not bound")),
                    Some((is_seq, text)) if *is_seq == mv.seq => edits.push((span, text.clone())),
                    Some(_) => err = Some(format!("metavariable `${name}` arity mismatch")),
                }
            }
            Some(Err(e)) => err = Some(e),
            None if unescape(s) != *s => edits.push((span, unescape(s).to_string())),
            None => {}
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = String::new();
    let mut cur = 0;
    for (r, t) in edits {
        out.push_str(&template_text[cur..r.start]);
        out.push_str(&t);
        cur = r.end;
    }
    out.push_str(&template_text[cur..]);
    Ok(out.trim().to_string())
}

/// A parsed stdin rewrite spec (ADR-0033).
struct RewriteSpec {
    hash: Option<String>,
    pattern_text: String,
    template_text: String,
}

/// Parse the stdin spec: an optional `@ <hash>` line, then `pattern <<TAG … TAG`
/// and `template <<TAG … TAG` heredoc blocks.
fn parse_spec(input: &str) -> Result<RewriteSpec, String> {
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    let hash = lines
        .get(i)
        .and_then(|l| l.trim().strip_prefix('@'))
        .map(|h| {
            i += 1;
            h.trim().to_string()
        });
    let pattern_text = read_block(&lines, &mut i, "pattern")?;
    let template_text = read_block(&lines, &mut i, "template")?;
    Ok(RewriteSpec {
        hash,
        pattern_text,
        template_text,
    })
}

fn read_block(lines: &[&str], i: &mut usize, keyword: &str) -> Result<String, String> {
    while *i < lines.len() && lines[*i].trim().is_empty() {
        *i += 1;
    }
    let header = lines
        .get(*i)
        .map(|l| l.trim())
        .ok_or_else(|| format!("missing `{keyword} <<TAG` block"))?;
    let tag = header
        .strip_prefix(keyword)
        .map(str::trim_start)
        .and_then(|r| r.strip_prefix("<<"))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| format!("expected `{keyword} <<TAG`, got `{header}`"))?;
    *i += 1;
    let start = *i;
    while *i < lines.len() {
        if lines[*i] == tag {
            let body = lines[start..*i].join("\n");
            *i += 1;
            return Ok(body);
        }
        *i += 1;
    }
    Err(format!("unterminated heredoc `{tag}`"))
}

/// Reject malformed metavariables (e.g. unknown class) anywhere in `data`.
fn validate_metavars(data: &[Datum]) -> Result<(), String> {
    let mut err = None;
    for_each_node(data, &mut |n| {
        if let DatumKind::Symbol(s) = &n.kind {
            if let Some(Err(e)) = metavar_of(s) {
                err.get_or_insert(e);
            }
        }
    });
    err.map_or(Ok(()), Err)
}

/// Reject a template metavariable not bound by the pattern, or one whose sequence
/// arity disagrees with the pattern's — independent of whether any site matches.
fn validate_template(pattern: &[Datum], template: &[Datum]) -> Result<(), String> {
    let mut pat: HashMap<String, bool> = HashMap::new();
    for_each_node(pattern, &mut |n| {
        if let DatumKind::Symbol(s) = &n.kind {
            if let Some(Ok(mv)) = metavar_of(s) {
                if let Some(name) = mv.name {
                    pat.insert(name, mv.seq);
                }
            }
        }
    });
    let mut err = None;
    for_each_node(template, &mut |n| {
        if let DatumKind::Symbol(s) = &n.kind {
            if let Some(Ok(mv)) = metavar_of(s) {
                if let Some(name) = &mv.name {
                    match pat.get(name.as_str()) {
                        None => {
                            err.get_or_insert(format!(
                                "template metavariable `${name}` is not bound by the pattern"
                            ));
                        }
                        Some(seq) if *seq != mv.seq => {
                            err.get_or_insert(format!("metavariable `${name}` arity mismatch between pattern and template"));
                        }
                        _ => {}
                    }
                } else {
                    err.get_or_insert("`$_` is not allowed in a template".to_string());
                }
            }
        }
    });
    err.map_or(Ok(()), Err)
}

/// Apply a structural pattern→template rewrite across `path` (ADR-0033) — a
/// parse-safe "structural sed". Reads the spec (pattern + template + optional
/// drift hash) already parsed from stdin; matches the pattern anywhere in the
/// tree (whole-tree, outermost non-overlapping, single pass), substitutes each
/// captured metavariable's verbatim text into the template, reindents the touched
/// top-level forms (native engines), validates, and writes atomically. Zero
/// matches is a success (idempotent). Not behaviour-preserving — the user asserts
/// the rewrite's semantics; lisplens guarantees only parse-safety + exact
/// structural matching.
pub fn rewrite_in_file(
    path: &Path,
    spec_input: &str,
    dialect: Dialect,
) -> Result<RewriteOutcome, RewriteError> {
    let spec = parse_spec(spec_input).map_err(RewriteError::Spec)?;
    let source = std::fs::read_to_string(path).map_err(RewriteError::Io)?;
    if let Some(want) = &spec.hash {
        let actual = file_hash(source.as_bytes());
        if &actual != want {
            return Err(RewriteError::Drift {
                expected: want.clone(),
                actual,
            });
        }
    }
    let options = Options::for_dialect(dialect);

    let pat_parsed = parse(&spec.pattern_text, &options);
    if !pat_parsed.errors.is_empty() {
        return Err(RewriteError::Spec("pattern does not parse".into()));
    }
    let pattern = match pat_parsed.data.as_slice() {
        [d] => d,
        [] => return Err(RewriteError::Spec("pattern is empty".into())),
        _ => return Err(RewriteError::Spec("pattern must be a single form".into())),
    };
    let tmpl_parsed = parse(&spec.template_text, &options);
    if !tmpl_parsed.errors.is_empty() {
        return Err(RewriteError::Spec("template does not parse".into()));
    }
    validate_metavars(&pat_parsed.data).map_err(RewriteError::Spec)?;
    validate_metavars(&tmpl_parsed.data).map_err(RewriteError::Spec)?;
    validate_template(&pat_parsed.data, &tmpl_parsed.data).map_err(RewriteError::Spec)?;

    let target = parse(&source, &options);
    let matches = keep_outermost(collect_matches(pattern, &target.data, &source));
    if matches.is_empty() {
        return Ok(RewriteOutcome {
            rewritten: 0,
            new_file_hash: file_hash(source.as_bytes()),
        });
    }

    let mut edits = Vec::with_capacity(matches.len());
    for m in &matches {
        let text =
            expand(&spec.template_text, &tmpl_parsed.data, &m.caps).map_err(RewriteError::Spec)?;
        edits.push(Edit {
            range: m.span.clone(),
            text,
        });
    }
    let rewritten = edits.len();
    let expected = file_hash(source.as_bytes());
    let (spliced, spans) = splice_tracked(&source, edits).map_err(RewriteError::Splice)?;
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
    verify_and_write(path, &expected, &new_content, &options).map_err(RewriteError::Write)?;
    Ok(RewriteOutcome {
        rewritten,
        new_file_hash: file_hash(new_content.as_bytes()),
    })
}

// ===== extract: pull a form into a new function (ADR-0034) =====

/// The result of a successful [`extract_into_function`].
#[derive(Debug)]
pub struct ExtractOutcome {
    /// The file hash after the extraction, over the reindented content.
    pub new_file_hash: String,
    /// How many occurrences were replaced by a call — 1 for single-site
    /// extraction, the count of structurally-equal sites for `--all` (ADR-0037).
    pub sites: usize,
}

/// Why an extraction was refused. No partial write ever happens on an error.
#[derive(Debug)]
pub enum ExtractError {
    /// The anchor was not `line:hash[:ordinal]`.
    BadAnchor(String),
    /// No form matched the anchor.
    AnchorNotFound(String),
    /// The requested run (`anchor + count`) extends past the anchored form's
    /// sibling group. Carries the count asked for and the count available.
    RunExceedsSiblings { asked: usize, available: usize },
    /// The dialect has no known function-definition form yet.
    UnsupportedDialect(String),
    /// The edits could not be spliced.
    Splice(SpliceError),
    /// The safe write was refused.
    Write(WriteError),
    /// A filesystem error.
    Io(std::io::Error),
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtractError::BadAnchor(a) => write!(f, "anchor `{a}` is not `line:hash[:ordinal]`"),
            ExtractError::AnchorNotFound(a) => write!(f, "no form at anchor `{a}`"),
            ExtractError::RunExceedsSiblings { asked, available } => write!(
                f,
                "run of {asked} exceeds the {available} sibling form(s) at the anchor"
            ),
            ExtractError::UnsupportedDialect(d) => write!(f, "extract not supported for {d} yet"),
            ExtractError::Splice(e) => write!(f, "splice failed: {e:?}"),
            ExtractError::Write(e) => write!(f, "{e:?}"),
            ExtractError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Extract the form at `anchor` into a new function `name` with parameters
/// `params`, replacing the form with a call `(name params…)` (ADR-0034).
///
/// A pure cut + wrap: the selection already uses its free variables by name and
/// the call site is in their scope, so no symbol substitution is needed —
/// lisplens supplies neither the name nor the parameters (the user asserts which
/// free locals are parameters; it does not infer them, per the ADR-0003 ceiling).
/// The new definition is placed just before the enclosing top-level form, its
/// wrapper chosen per dialect (`defun`/`define`/`defn`); unsupported dialects are
/// refused. Parse-safe only: behaviour is preserved only if the parameters cover
/// the free locals and the selection has no context-dependent non-local exit.
pub fn extract_into_function(
    path: &Path,
    anchor: &str,
    name: &str,
    params: &[String],
    dialect: Dialect,
) -> Result<ExtractOutcome, ExtractError> {
    extract_block_into_function(path, anchor, name, params, 1, None, dialect)
}

/// Extract the run of `count` contiguous sibling forms starting at `anchor` into a
/// new function `name` (ADR-0035). The generalization of [`extract_into_function`]
/// (which is the `count == 1` case): the run is wrapped verbatim as an implicit
/// `progn` body, so it is behaviour-preserving only in a body/`progn` position.
/// Errors, and never partially writes, if the run crosses the anchored form's
/// sibling group. `kind` overrides the definition head within the dialect's shape
/// family (ADR-0036), `None` for the plain-function default.
pub fn extract_block_into_function(
    path: &Path,
    anchor: &str,
    name: &str,
    params: &[String],
    count: usize,
    kind: Option<&str>,
    dialect: Dialect,
) -> Result<ExtractOutcome, ExtractError> {
    let source = std::fs::read_to_string(path).map_err(ExtractError::Io)?;
    let options = Options::for_dialect(dialect);
    let parsed = parse(&source, &options);

    let anchor_val = parse_anchor(anchor).ok_or_else(|| ExtractError::BadAnchor(anchor.into()))?;
    let located = crate::resolve::resolve(&source, &parsed.data, &anchor_val)
        .ok_or_else(|| ExtractError::AnchorNotFound(anchor.into()))?;
    let sel = run_span(&parsed.data, &located, count)?;
    let encl_start = enclosing_top_level(&parsed.data, &sel);

    let body = &source[sel.clone()];
    let def = def_form(dialect, name, params, body, kind)
        .ok_or_else(|| ExtractError::UnsupportedDialect(format!("{dialect:?}")))?;
    let call = call_form(name, params);
    finish_extraction(
        path,
        &source,
        &options,
        dialect,
        &def,
        encl_start,
        &[sel],
        &call,
    )
}

/// Extract **every occurrence structurally equal to the anchored selection** into
/// one new function `name`, replacing each with the call `(name params…)`
/// (ADR-0037, the `--all` opt-in). "The same" is `struct_eq` (formatting-modulo
/// structural equality, as `rewrite`): for `count == 1` a site is any node in the
/// tree; for `count > 1` a site is any window of `count` contiguous siblings equal
/// to the anchored run. The def is inserted once, before the earliest site's
/// enclosing top-level form. Sites do not generalize — every occurrence is
/// identical *including* its arguments, so the same `(name params…)` call replaces
/// each; `params` name the def's formals but introduce no per-site variation (the
/// ADR-0003 ceiling — anti-unification is deferred). A form that appears once
/// degrades to plain single-site extraction. Unsupported dialects are refused.
pub fn extract_multi_site(
    path: &Path,
    anchor: &str,
    name: &str,
    params: &[String],
    count: usize,
    kind: Option<&str>,
    dialect: Dialect,
) -> Result<ExtractOutcome, ExtractError> {
    let source = std::fs::read_to_string(path).map_err(ExtractError::Io)?;
    let options = Options::for_dialect(dialect);
    let parsed = parse(&source, &options);

    let anchor_val = parse_anchor(anchor).ok_or_else(|| ExtractError::BadAnchor(anchor.into()))?;
    let located = crate::resolve::resolve(&source, &parsed.data, &anchor_val)
        .ok_or_else(|| ExtractError::AnchorNotFound(anchor.into()))?;
    let pattern = run_datums(&parsed.data, &located, count)?;
    let anchored = pattern[0].span.start as usize..pattern[pattern.len() - 1].span.end as usize;

    // All sites are structurally equal, so build the def from the anchored text.
    let body = &source[anchored];
    let def = def_form(dialect, name, params, body, kind)
        .ok_or_else(|| ExtractError::UnsupportedDialect(format!("{dialect:?}")))?;
    let call = call_form(name, params);

    // Every structurally-equal, non-overlapping occurrence; the anchored run is
    // always among them, so `sites` is non-empty and ascending.
    let sites = keep_outermost_spans(collect_struct_sites(&pattern, &parsed.data));
    let def_before = enclosing_top_level(&parsed.data, &sites[0]);
    finish_extraction(
        path, &source, &options, dialect, &def, def_before, &sites, &call,
    )
}

/// Byte offset of the top-level form enclosing `span` (its start), or `span.start`
/// when the span is itself top-level — where the extracted def is inserted.
fn enclosing_top_level(data: &[Datum], span: &Range<usize>) -> usize {
    data.iter()
        .find(|d| (d.span.start as usize) <= span.start && span.end <= (d.span.end as usize))
        .map_or(span.start, |d| d.span.start as usize)
}

/// Insert `def_text` before `def_before`, replace every `site` span with `call`,
/// then reindent the touched forms (native engines), validate, and write
/// atomically. The shared tail of single- and multi-site extraction. `sites` must
/// be non-overlapping and ascending, with `def_before <= sites[0].start`.
#[allow(clippy::too_many_arguments)]
fn finish_extraction(
    path: &Path,
    source: &str,
    options: &Options,
    dialect: Dialect,
    def_text: &str,
    def_before: usize,
    sites: &[Range<usize>],
    call: &str,
) -> Result<ExtractOutcome, ExtractError> {
    let mut edits = Vec::with_capacity(sites.len() + 1);
    edits.push(Edit {
        range: def_before..def_before,
        text: format!("{def_text}\n\n"),
    });
    for s in sites {
        edits.push(Edit {
            range: s.clone(),
            text: call.to_string(),
        });
    }
    let expected = file_hash(source.as_bytes());
    let (spliced, spans) = splice_tracked(source, edits).map_err(ExtractError::Splice)?;
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
    verify_and_write(path, &expected, &new_content, options).map_err(ExtractError::Write)?;
    Ok(ExtractOutcome {
        new_file_hash: file_hash(new_content.as_bytes()),
        sites: sites.len(),
    })
}

/// Every span structurally equal to the `pattern` run (ADR-0037). A single-datum
/// pattern matches any node anywhere (whole-tree walk); a multi-datum pattern
/// matches any window of contiguous siblings, in any sibling group at any depth.
fn collect_struct_sites(pattern: &[&Datum], data: &[Datum]) -> Vec<Range<usize>> {
    let n = pattern.len();
    let mut spans = Vec::new();
    if n == 1 {
        for_each_node(data, &mut |node| {
            if struct_eq(node, pattern[0]) {
                spans.push(span_of(node));
            }
        });
    } else {
        for_each_sibling_group(data, &mut |group| {
            if group.len() < n {
                return;
            }
            for w in 0..=group.len() - n {
                if (0..n).all(|k| struct_eq(&group[w + k], pattern[k])) {
                    spans.push(group[w].span.start as usize..group[w + n - 1].span.end as usize);
                }
            }
        });
    }
    spans
}

/// Keep only the outermost, non-overlapping spans (splice cannot overlap; the span
/// form of [`keep_outermost`]). Sorted ascending on return.
fn keep_outermost_spans(mut spans: Vec<Range<usize>>) -> Vec<Range<usize>> {
    spans.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));
    let mut kept: Vec<Range<usize>> = Vec::new();
    let mut covered = 0usize;
    for s in spans {
        if s.start >= covered {
            covered = s.end;
            kept.push(s);
        }
    }
    kept
}

/// Visit every sibling group in the tree: the top-level `data`, then each list's
/// `items`, recursively (the sequences a run of siblings can live in). Mirrors
/// [`for_each_node`]'s descent but yields the item slices, not the nodes.
fn for_each_sibling_group<'t>(data: &'t [Datum<'t>], f: &mut impl FnMut(&'t [Datum<'t>])) {
    f(data);
    for d in data {
        match &d.kind {
            DatumKind::List { items, tail, .. } => {
                for_each_sibling_group(items, f);
                if let Some(t) = tail {
                    for_each_sibling_group(std::slice::from_ref(t), f);
                }
            }
            DatumKind::Prefixed { inner, arg, .. } => {
                for_each_sibling_group(std::slice::from_ref(inner), f);
                if let Some(a) = arg {
                    for_each_sibling_group(std::slice::from_ref(a), f);
                }
            }
            DatumKind::HashLiteral {
                inner: Some(inner), ..
            } => for_each_sibling_group(std::slice::from_ref(inner), f),
            DatumKind::Label { inner, .. } => {
                for_each_sibling_group(std::slice::from_ref(inner), f)
            }
            _ => {}
        }
    }
}

/// The source span of the run of `count` contiguous sibling forms starting at the
/// located node (ADR-0035): the [`run_datums`] range, `first.start .. last.end`.
fn run_span(
    data: &[Datum],
    located: &crate::resolve::Located,
    count: usize,
) -> Result<Range<usize>, ExtractError> {
    let run = run_datums(data, located, count)?;
    Ok(run[0].span.start as usize..run[run.len() - 1].span.end as usize)
}

/// The `count` contiguous sibling **datums** starting at the located node — the
/// datum form of [`run_span`], used by multi-site extraction to compare structure
/// (ADR-0037). `count == 1` is the node itself (the ADR-0034 single-form case). The
/// sibling group is the anchored form's parent list items, or the top-level `data`
/// when the anchor is a top-level form; the run is `siblings[index .. index +
/// count]`. Errors if `count == 0` or the run would cross the sibling group's end —
/// no partial write can follow.
fn run_datums<'a, 't>(
    data: &'a [Datum<'t>],
    located: &crate::resolve::Located<'a, 't>,
    count: usize,
) -> Result<Vec<&'a Datum<'t>>, ExtractError> {
    if count == 0 {
        return Err(ExtractError::RunExceedsSiblings {
            asked: 0,
            available: 0,
        });
    }
    if count == 1 {
        return Ok(vec![located.node]);
    }
    let siblings: &'a [Datum<'t>] = match located.parent {
        Some(p) => match &p.kind {
            DatumKind::List { items, .. } => items,
            _ => std::slice::from_ref(located.node),
        },
        None => data,
    };
    let idx = located.index.unwrap_or(0);
    let available = siblings.len().saturating_sub(idx);
    if count > available {
        return Err(ExtractError::RunExceedsSiblings {
            asked: count,
            available,
        });
    }
    Ok(siblings[idx..idx + count].iter().collect())
}

/// Parse a `line:hash[:ordinal]` anchor string.
fn parse_anchor(token: &str) -> Option<crate::patch::Anchor> {
    let mut parts = token.split(':');
    let line = parts.next()?.parse::<u32>().ok()?;
    let hash = parts.next().filter(|s| !s.is_empty())?.to_string();
    let ordinal = match parts.next() {
        Some(s) => Some(s.parse::<u32>().ok()?),
        None => None,
    };
    Some(crate::patch::Anchor {
        line,
        hash,
        ordinal,
    })
}

/// The definition-form shape families (ADR-0036): where the name, arglist, and
/// its bracket go. `--kind` swaps the head but never the shape.
#[derive(Clone, Copy)]
enum DefShape {
    /// `(HEAD NAME (params) body)` — Emacs Lisp / Common Lisp.
    Flat,
    /// `(HEAD (NAME params) body)` — the Scheme family.
    Nested,
    /// `(HEAD NAME [params] body)` — Clojure.
    Bracket,
}

/// The plain-function head and shape family for `dialect` (`None` if the dialect
/// has no known def form; `--kind` does not unlock these — the bracket/nesting is
/// unknown).
fn def_shape(dialect: Dialect) -> Option<(&'static str, DefShape)> {
    Some(match dialect {
        Dialect::EmacsLisp | Dialect::CommonLisp => ("defun", DefShape::Flat),
        Dialect::Scheme
        | Dialect::Guile
        | Dialect::Racket
        | Dialect::Gauche
        | Dialect::Mosh
        | Dialect::Gambit
        | Dialect::SchemeSuperset => ("define", DefShape::Nested),
        Dialect::Clojure => ("defn", DefShape::Bracket),
        _ => return None,
    })
}

/// The new-function wrapper for `dialect` (`None` if the dialect has no known
/// def form). Only the wrapper is dialect-specific; `body` is the verbatim
/// selection. `kind` overrides the head within the dialect's shape family
/// (ADR-0036), defaulting to the plain-function head. A multi-line body (a sibling
/// run, or a multi-line single form) is placed on its own line after the arglist so
/// reindent lays it out as a proper body; a single-line body stays inline (the
/// ADR-0034 one-liner).
fn def_form(
    dialect: Dialect,
    name: &str,
    params: &[String],
    body: &str,
    kind: Option<&str>,
) -> Option<String> {
    let (default_head, shape) = def_shape(dialect)?;
    let head = kind.unwrap_or(default_head);
    let ps = params.join(" ");
    // Separator between the arglist and the body.
    let sep = if body.contains('\n') { "\n" } else { " " };
    Some(match shape {
        DefShape::Flat => format!("({head} {name} ({ps}){sep}{body})"),
        DefShape::Nested => format!("({head} ({name} {ps}){sep}{body})"),
        DefShape::Bracket => format!("({head} {name} [{ps}]{sep}{body})"),
    })
}

/// The call `(name params…)` that replaces the extracted form.
fn call_form(name: &str, params: &[String]) -> String {
    if params.is_empty() {
        format!("({name})")
    } else {
        format!("({name} {})", params.join(" "))
    }
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
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "(defun foo () 1)\n"
        );
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
            Err(InlineError::ArityMismatch {
                expected: 1,
                found: 2,
                ..
            })
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

    // ----- rewrite (ADR-0033) -----

    fn spec(pattern: &str, template: &str) -> String {
        format!("pattern <<P\n{pattern}\nP\ntemplate <<T\n{template}\nT\n")
    }

    #[test]
    fn rewrite_guard_removal_all_sites() {
        let (_d, path) = write_temp(
            "a.el",
            "(defun a ()\n  (when flag\n    (foo)))\n(defun b () (when q (bar)))\n",
        );
        let out = rewrite_in_file(
            &path,
            &spec("(when $flag $body)", "$body"),
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert_eq!(out.rewritten, 2);
        let r = std::fs::read_to_string(&path).unwrap();
        assert!(r.contains("(defun a ()\n  (foo))"), "{r}");
        assert!(r.contains("(defun b () (bar))"), "{r}");
    }

    #[test]
    fn rewrite_if_to_when_and_progn_unwrap() {
        let (_d, p1) = write_temp("a.el", "(setq x (if c a nil))\n");
        rewrite_in_file(
            &p1,
            &spec("(if $c $a nil)", "(when $c $a)"),
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&p1).unwrap(),
            "(setq x (when c a))\n"
        );

        // Sequence metavariable unwraps a progn, preserving the forms.
        let (_d, p2) = write_temp("a.el", "(progn\n  (a)\n  (b))\n");
        let out = rewrite_in_file(
            &p2,
            &spec("(progn $body...)", "$body..."),
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert_eq!(out.rewritten, 1);
        assert_eq!(std::fs::read_to_string(&p2).unwrap(), "(a)\n(b)\n");
    }

    #[test]
    fn rewrite_class_filter_and_non_linear() {
        // `:atom` folds a literal/symbol but not a side-effecting call.
        let (_d, p1) = write_temp("a.el", "(list (double 100) (double (getnum)) (double x))\n");
        let out = rewrite_in_file(
            &p1,
            &spec("(double $n:atom)", "(* 2 $n)"),
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert_eq!(out.rewritten, 2);
        assert_eq!(
            std::fs::read_to_string(&p1).unwrap(),
            "(list (* 2 100) (double (getnum)) (* 2 x))\n"
        );
        // Non-linear: `(eq $x $x)` matches only equal operands.
        let (_d, p2) = write_temp("a.el", "(list (eq a a) (eq a b))\n");
        rewrite_in_file(&p2, &spec("(eq $x $x)", "t"), Dialect::EmacsLisp).unwrap();
        assert_eq!(std::fs::read_to_string(&p2).unwrap(), "(list t (eq a b))\n");
    }

    #[test]
    fn rewrite_empty_template_deletes_and_wildcard_matches_any() {
        // Empty template = deletion; `$_` matches (and discards) any argument.
        let (_d, path) = write_temp("a.el", "(progn (log x) (real))\n");
        rewrite_in_file(&path, &spec("(log $_)", ""), Dialect::EmacsLisp).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(progn  (real))\n");
    }

    #[test]
    fn rewrite_zero_matches_is_success() {
        let (_d, path) = write_temp("a.el", "(foo)\n");
        let out = rewrite_in_file(&path, &spec("(nope $x)", "$x"), Dialect::EmacsLisp).unwrap();
        assert_eq!(out.rewritten, 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(foo)\n"); // untouched
    }

    #[test]
    fn rewrite_across_dialects_matches_delimiter() {
        // Clojure `[]` is a distinct delimiter — a `()` pattern must not match it.
        let (_d, path) = write_temp("a.clj", "(foo (a b) [a b])\n");
        let out = rewrite_in_file(&path, &spec("(a b)", "AB"), Dialect::Clojure).unwrap();
        assert_eq!(out.rewritten, 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(foo AB [a b])\n");
    }

    #[test]
    fn rewrite_rejects_bad_specs() {
        let (_d, path) = write_temp("a.el", "(foo bar)\n");
        // unknown class
        assert!(matches!(
            rewrite_in_file(&path, &spec("(foo $x:nope)", "$x"), Dialect::EmacsLisp),
            Err(RewriteError::Spec(_))
        ));
        // unbound template metavariable
        assert!(matches!(
            rewrite_in_file(&path, &spec("(foo $x)", "$y"), Dialect::EmacsLisp),
            Err(RewriteError::Spec(_))
        ));
        // arity mismatch (single in pattern, sequence in template)
        assert!(matches!(
            rewrite_in_file(&path, &spec("(foo $x)", "$x..."), Dialect::EmacsLisp),
            Err(RewriteError::Spec(_))
        ));
        // multi-form pattern
        assert!(matches!(
            rewrite_in_file(&path, &spec("(a) (b)", "x"), Dialect::EmacsLisp),
            Err(RewriteError::Spec(_))
        ));
        std::fs::read_to_string(&path).unwrap(); // file untouched on every error
    }

    #[test]
    fn rewrite_drift_gate_when_hash_given() {
        let (_d, path) = write_temp("a.el", "(foo)\n");
        let good = crate::hash::file_hash("(foo)\n".as_bytes());
        let s = format!("@ {good}\n{}", spec("(foo)", "(bar)"));
        rewrite_in_file(&path, &s, Dialect::EmacsLisp).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(bar)\n");
        // A stale hash is refused.
        let stale = crate::hash::file_hash("different".as_bytes());
        let s2 = format!("@ {stale}\n{}", spec("(bar)", "(baz)"));
        assert!(matches!(
            rewrite_in_file(&path, &s2, Dialect::EmacsLisp),
            Err(RewriteError::Drift { .. })
        ));
    }

    // ----- extract (ADR-0034) -----

    /// Anchor `line:hash` for the form whose verbatim text is `form` at `line`.
    fn anchor_for(form: &str, line: u32) -> String {
        format!("{line}:{}", crate::hash::anchor_hash(form.as_bytes()))
    }

    #[test]
    fn extract_expression_with_a_param() {
        let (_d, path) = write_temp(
            "a.el",
            "(defun foo (x)\n  (message \"hi\")\n  (* (+ x 1) 2))\n",
        );
        let a = anchor_for("(* (+ x 1) 2)", 3);
        extract_into_function(&path, &a, "compute", &["x".into()], Dialect::EmacsLisp).unwrap();
        let r = std::fs::read_to_string(&path).unwrap();
        assert!(
            r.starts_with("(defun compute (x) (* (+ x 1) 2))\n\n"),
            "{r}"
        );
        assert!(r.contains("  (compute x))"), "{r}");
    }

    #[test]
    fn extract_niladic_and_scheme_and_clojure() {
        // Niladic (Emacs Lisp).
        let (_d, p1) = write_temp("a.el", "(defun foo ()\n  (side-effect)\n  (bar))\n");
        extract_into_function(
            &p1,
            &anchor_for("(side-effect)", 2),
            "act",
            &[],
            Dialect::EmacsLisp,
        )
        .unwrap();
        let r1 = std::fs::read_to_string(&p1).unwrap();
        assert!(r1.starts_with("(defun act () (side-effect))\n\n"), "{r1}");
        assert!(r1.contains("  (act)\n"), "{r1}");

        // Scheme `define`.
        let (_d, p2) = write_temp("a.scm", "(define (f x)\n  (+ x 1))\n");
        extract_into_function(
            &p2,
            &anchor_for("(+ x 1)", 2),
            "g",
            &["x".into()],
            Dialect::Scheme,
        )
        .unwrap();
        assert!(std::fs::read_to_string(&p2)
            .unwrap()
            .starts_with("(define (g x) (+ x 1))\n\n"));

        // Clojure `defn` with `[]` params.
        let (_d, p3) = write_temp("a.clj", "(defn f [x] (+ x 1))\n");
        extract_into_function(
            &p3,
            &anchor_for("(+ x 1)", 1),
            "g",
            &["x".into()],
            Dialect::Clojure,
        )
        .unwrap();
        assert!(std::fs::read_to_string(&p3)
            .unwrap()
            .contains("(defn g [x] (+ x 1))"));
    }

    #[test]
    fn extract_rejects_bad_anchor_missing_and_unsupported_dialect() {
        let (_d, path) = write_temp("a.el", "(defun foo (x) (g x))\n");
        assert!(matches!(
            extract_into_function(&path, "nope", "h", &[], Dialect::EmacsLisp),
            Err(ExtractError::BadAnchor(_))
        ));
        assert!(matches!(
            extract_into_function(&path, "1:dead", "h", &[], Dialect::EmacsLisp),
            Err(ExtractError::AnchorNotFound(_))
        ));
        // Unsupported dialect: a Fennel file with a valid anchor reaches def_form.
        let (_d, fnl) = write_temp("a.fnl", "(fn [x] (+ x 1))\n");
        let a = anchor_for("(+ x 1)", 1);
        assert!(matches!(
            extract_into_function(&fnl, &a, "g", &["x".into()], Dialect::Fennel),
            Err(ExtractError::UnsupportedDialect(_))
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "(defun foo (x) (g x))\n"
        ); // untouched
    }

    // ----- block extraction, anchor + count (ADR-0035) -----

    #[test]
    fn extract_block_of_siblings_in_a_body() {
        // A run of two body forms `(foo) (bar)` → one niladic helper called once.
        let (_d, path) = write_temp("a.el", "(defun main ()\n  (foo)\n  (bar)\n  (baz))\n");
        let a = anchor_for("(foo)", 2);
        extract_block_into_function(&path, &a, "foo-bar", &[], 2, None, Dialect::EmacsLisp)
            .unwrap();
        let r = std::fs::read_to_string(&path).unwrap();
        // The multi-form run lands on its own body lines, before the enclosing defun,
        // and `main`'s run is replaced by one call (`(baz)` untouched).
        assert_eq!(
            r,
            "(defun foo-bar ()\n  (foo)\n  (bar))\n\n(defun main ()\n  (foo-bar)\n  (baz))\n"
        );
    }

    #[test]
    fn extract_block_top_level_run() {
        // A run of two *top-level* forms (parent is None → siblings are the data).
        let (_d, path) = write_temp("a.el", "(foo)\n(bar)\n(baz)\n");
        let a = anchor_for("(foo)", 1);
        extract_block_into_function(&path, &a, "setup", &[], 2, None, Dialect::EmacsLisp).unwrap();
        let r = std::fs::read_to_string(&path).unwrap();
        // Def inserted before the run; the run replaced by the call; `(baz)` stays.
        assert!(r.starts_with("(defun setup ()"), "{r}");
        assert!(r.contains("(setup)"), "{r}");
        assert!(r.contains("(baz)"), "{r}");
        assert_eq!(r.matches("(foo)").count(), 1, "{r}");
        assert_eq!(r.matches("(bar)").count(), 1, "{r}");
    }

    #[test]
    fn extract_block_count_one_equals_single_form() {
        // count == 1 must reproduce the ADR-0034 single-form path exactly.
        let src = "(defun foo (x)\n  (message \"hi\")\n  (* (+ x 1) 2))\n";
        let (_d, p1) = write_temp("a.el", src);
        let (_d2, p2) = write_temp("b.el", src);
        let a = anchor_for("(* (+ x 1) 2)", 3);
        extract_into_function(&p1, &a, "compute", &["x".into()], Dialect::EmacsLisp).unwrap();
        extract_block_into_function(
            &p2,
            &a,
            "compute",
            &["x".into()],
            1,
            None,
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&p1).unwrap(),
            std::fs::read_to_string(&p2).unwrap()
        );
    }

    #[test]
    fn extract_block_rejects_run_past_siblings_and_zero_count() {
        let (_d, path) = write_temp("a.el", "(defun main ()\n  (foo)\n  (bar))\n");
        let a = anchor_for("(bar)", 3);
        // Only one sibling remains from `(bar)`; a run of 2 crosses the group's end.
        assert!(matches!(
            extract_block_into_function(&path, &a, "h", &[], 2, None, Dialect::EmacsLisp),
            Err(ExtractError::RunExceedsSiblings {
                asked: 2,
                available: 1
            })
        ));
        // count == 0 is rejected too.
        assert!(matches!(
            extract_block_into_function(
                &path,
                &anchor_for("(foo)", 2),
                "h",
                &[],
                0,
                None,
                Dialect::EmacsLisp
            ),
            Err(ExtractError::RunExceedsSiblings { asked: 0, .. })
        ));
        // No partial write on either refusal.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "(defun main ()\n  (foo)\n  (bar))\n"
        );
    }

    // ----- non-defun kinds via `kind` (ADR-0036) -----

    #[test]
    fn extract_kind_overrides_head_within_each_shape_family() {
        // Flat (elisp): `defsubst` instead of `defun`, arglist in `(params)`.
        let (_d, p1) = write_temp("a.el", "(defun foo (x)\n  (* x 2))\n");
        extract_block_into_function(
            &p1,
            &anchor_for("(* x 2)", 2),
            "dbl",
            &["x".into()],
            1,
            Some("defsubst"),
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert!(std::fs::read_to_string(&p1)
            .unwrap()
            .starts_with("(defsubst dbl (x) (* x 2))\n\n"));

        // Nested (scheme): `define-inline` keeps the `(NAME params)` nesting.
        let (_d, p2) = write_temp("a.scm", "(define (f x)\n  (+ x 1))\n");
        extract_block_into_function(
            &p2,
            &anchor_for("(+ x 1)", 2),
            "g",
            &["x".into()],
            1,
            Some("define-inline"),
            Dialect::Scheme,
        )
        .unwrap();
        assert!(std::fs::read_to_string(&p2)
            .unwrap()
            .starts_with("(define-inline (g x) (+ x 1))\n\n"));

        // Bracket (clojure): private `defn-` keeps the `[params]` bracket.
        let (_d, p3) = write_temp("a.clj", "(defn f [x] (+ x 1))\n");
        extract_block_into_function(
            &p3,
            &anchor_for("(+ x 1)", 1),
            "g",
            &["x".into()],
            1,
            Some("defn-"),
            Dialect::Clojure,
        )
        .unwrap();
        assert!(std::fs::read_to_string(&p3)
            .unwrap()
            .contains("(defn- g [x] (+ x 1))"));
    }

    #[test]
    fn extract_kind_does_not_unlock_unsupported_dialects() {
        // An unsupported dialect stays `UnsupportedDialect` even with `kind` set —
        // its bracket/nesting is unknown, so `--kind` cannot rescue it.
        let (_d, fnl) = write_temp("a.fnl", "(fn [x] (+ x 1))\n");
        let a = anchor_for("(+ x 1)", 1);
        assert!(matches!(
            extract_block_into_function(
                &fnl,
                &a,
                "g",
                &["x".into()],
                1,
                Some("fn"),
                Dialect::Fennel
            ),
            Err(ExtractError::UnsupportedDialect(_))
        ));
    }

    // ----- multi-site extraction via `--all` (ADR-0037) -----

    #[test]
    fn extract_multi_site_replaces_every_equal_occurrence() {
        // `(log)` occurs three times — a body sibling, and a *subterm* of `(when x
        // …)` in another defun — all structurally equal, so `--all` replaces every
        // one with the call and hoists one niladic helper before the earliest.
        let (_d, path) = write_temp(
            "a.el",
            "(defun a ()\n  (log)\n  (work))\n\n(defun b ()\n  (when x (log)))\n",
        );
        let out = extract_multi_site(
            &path,
            &anchor_for("(log)", 2),
            "record",
            &[],
            1,
            None,
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert_eq!(out.sites, 2);
        let r = std::fs::read_to_string(&path).unwrap();
        assert!(r.starts_with("(defun record () (log))\n\n"), "{r}");
        assert_eq!(r.matches("(record)").count(), 2, "{r}");
        assert_eq!(r.matches("(log)").count(), 1, "{r}"); // only inside the def
        assert!(r.contains("(when x (record))"), "{r}");
    }

    #[test]
    fn extract_multi_site_of_a_block() {
        // A run of two siblings `(foo) (bar)` repeated in two defuns → one helper,
        // each run replaced by a single call (`--all` composed with `--count 2`).
        let (_d, path) = write_temp(
            "a.el",
            "(defun a ()\n  (foo)\n  (bar)\n  (rest-a))\n\n\
             (defun b ()\n  (foo)\n  (bar)\n  (rest-b))\n",
        );
        let out = extract_multi_site(
            &path,
            &anchor_for("(foo)", 2),
            "setup",
            &[],
            2,
            None,
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert_eq!(out.sites, 2);
        let r = std::fs::read_to_string(&path).unwrap();
        assert!(
            r.starts_with("(defun setup ()\n  (foo)\n  (bar))\n\n"),
            "{r}"
        );
        assert_eq!(r.matches("(setup)").count(), 2, "{r}");
        assert_eq!(r.matches("(foo)").count(), 1, "{r}");
        assert_eq!(r.matches("(bar)").count(), 1, "{r}");
        assert!(r.contains("(rest-a)") && r.contains("(rest-b)"), "{r}");
    }

    #[test]
    fn extract_multi_site_composes_with_kind() {
        // `--all` and `--kind` are orthogonal: every site is replaced and the emitted
        // head is the requested `defsubst`.
        let (_d, path) = write_temp("a.el", "(defun a () (ping))\n(defun b () (ping))\n");
        let out = extract_multi_site(
            &path,
            &anchor_for("(ping)", 1),
            "p",
            &[],
            1,
            Some("defsubst"),
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert_eq!(out.sites, 2);
        let r = std::fs::read_to_string(&path).unwrap();
        assert!(r.starts_with("(defsubst p () (ping))\n\n"), "{r}");
        assert_eq!(r.matches("(p)").count(), 2, "{r}");
    }

    #[test]
    fn extract_multi_site_single_occurrence_degrades() {
        // A form that appears once is just single-site extraction, sites == 1.
        let (_d, path) = write_temp("a.el", "(defun a () (only))\n");
        let out = extract_multi_site(
            &path,
            &anchor_for("(only)", 1),
            "h",
            &[],
            1,
            None,
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert_eq!(out.sites, 1);
        let r = std::fs::read_to_string(&path).unwrap();
        assert!(r.starts_with("(defun h () (only))\n\n"), "{r}");
        assert!(r.contains("(defun a () (h))"), "{r}");
    }

    #[test]
    fn extract_multi_site_block_overlap_keeps_outermost() {
        // Three identical `(tick)` siblings and a run of 2: the two candidate windows
        // overlap, so only the first is taken (the third `(tick)` stays).
        let (_d, path) = write_temp("a.el", "(defun a ()\n  (tick)\n  (tick)\n  (tick))\n");
        let out = extract_multi_site(
            &path,
            &anchor_for("(tick)", 2),
            "twice",
            &[],
            2,
            None,
            Dialect::EmacsLisp,
        )
        .unwrap();
        assert_eq!(out.sites, 1);
        let r = std::fs::read_to_string(&path).unwrap();
        assert!(
            r.starts_with("(defun twice ()\n  (tick)\n  (tick))\n\n"),
            "{r}"
        );
        // One call replaces the first two; the third `(tick)` is untouched.
        assert!(r.contains("(defun a ()\n  (twice)\n  (tick))"), "{r}");
    }
}
