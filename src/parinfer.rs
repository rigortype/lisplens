//! A native, from-scratch parinfer-style transform built on lisplens's own
//! toolset — the faithful Emacs indenter, the Nameless model (ADR-0030), and the
//! lispexp reader — **not** an integration with `parinfer-rust` /
//! `parinfer-rust-emacs`, and intentionally **not** API-compatible with
//! `parinfer.js`. lisplens becomes its own parinfer alternative (ADR-0045).
//!
//! Execution is a stateless whole-buffer transform (no live editing loop). This
//! module is the engine; the CLI (`lisplens parinfer <mode>`, stdin→stdout) and
//! the MCP `parinfer` tool are thin wrappers over [`run`].
//!
//! # Modes
//!
//! - **Paren mode** ([`Mode::Paren`], this slice): parens are the source of
//!   truth. Balanced input is reindented to lisplens's faithful Emacs
//!   indentation (reusing the formatter and its Nameless *production* path);
//!   unbalanced input is refused unchanged with a positioned diagnostic.
//! - **Indent mode** and **Nameless-aware indentation** are the follow-ups
//!   (issues #25 / #26).
//!
//! # Safety model (ADR-0045)
//!
//! Unlike the edit pipeline's error-parity rule (ADR-0005: never introduce a
//! *new* parse error), the parinfer command **generates balance**: on success
//! the output parses clean; on an unresolvable situation the input is returned
//! **completely unchanged** with `success = false` and a positioned diagnostic.
//! Broken output is never emitted either way.

use lispexp::{parse, Dialect, ErrorKind, Options, ParseError};

use crate::config::FormatConfig;
use crate::format;
use crate::nameless::Nameless;

/// Which parinfer transform to run. Indent mode (#25) is a future variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Parens are the source of truth; indentation is corrected. Requires
    /// balanced input.
    Paren,
}

impl Mode {
    /// Parse a mode name (`paren`), or `None` if unrecognized.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Mode> {
        match name {
            "paren" => Some(Mode::Paren),
            _ => None,
        }
    }
}

/// A 0-based text position (parinfer convention): `line`, and `x` as a character
/// offset within that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// 0-based line number.
    pub line: usize,
    /// 0-based character offset within the line.
    pub x: usize,
}

/// Everything a transform needs. `nameless` is the Emacs-Lisp-only overlay
/// (ADR-0030), already built by the caller; `config` carries the indentation
/// parameters (defaults, or resolved from a file hint).
pub struct Request<'a> {
    /// The transform to run.
    pub mode: Mode,
    /// The buffer text to transform.
    pub text: &'a str,
    /// The dialect the text is in.
    pub dialect: Dialect,
    /// Indentation parameters (tab width, body indent, …).
    pub config: FormatConfig,
    /// Nameless overlay (Emacs Lisp only), when enabled.
    pub nameless: Option<Nameless>,
    /// The input cursor, tracked to its post-transform position (position
    /// tracking only — no cursor-protection rules in this slice).
    pub cursor: Option<Cursor>,
}

/// A positioned failure, mirroring parinfer's error shape with 0-based line/`x`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// A short, stable error name (e.g. `unclosed-paren`).
    pub name: String,
    /// The human-readable message (lispexp's `ErrorKind` display).
    pub message: String,
    /// 0-based line where the failure was detected.
    pub line: usize,
    /// 0-based character offset where the failure was detected.
    pub x: usize,
}

/// The result of a transform.
pub struct Answer {
    /// The transformed text on success; the **unchanged input** on failure.
    pub text: String,
    /// Whether the transform produced a clean, balanced result.
    pub success: bool,
    /// The positioned failure, when `success` is false.
    pub error: Option<Error>,
    /// The cursor's post-transform position, if an input cursor was given.
    pub cursor: Option<Cursor>,
}

/// Run the parinfer transform described by `req`.
#[must_use]
pub fn run(req: &Request) -> Answer {
    match req.mode {
        Mode::Paren => run_paren(req),
    }
}

/// Paren mode: require balance, then faithfully reindent.
fn run_paren(req: &Request) -> Answer {
    let options = Options::for_dialect(req.dialect);
    let parsed = parse(req.text, &options);
    if let Some(err) = parsed.errors.first() {
        // Imbalance / unresolvable: return the input untouched + a diagnostic.
        return Answer {
            text: req.text.to_string(),
            success: false,
            error: Some(to_error(req.text, err)),
            cursor: req.cursor,
        };
    }
    // Balanced: faithful reindent. Nameless production side (column *generation*)
    // is Emacs-Lisp-only, reusing the formatter's existing path.
    let formatted = match (&req.nameless, req.dialect) {
        (Some(nl), Dialect::EmacsLisp) => format::format_elisp_nameless(req.text, &req.config, nl),
        _ => format::format(req.text, &req.config, req.dialect),
    };
    let cursor = req.cursor.map(|c| remap_cursor(req.text, &formatted, c));
    Answer {
        text: formatted,
        success: true,
        error: None,
        cursor,
    }
}

/// Map a lispexp [`ParseError`] to a parinfer-style [`Error`] with a 0-based
/// position derived from its span.
fn to_error(source: &str, err: &ParseError) -> Error {
    let (line, x) = byte_to_line_col(source, err.span.start as usize);
    Error {
        name: error_name(&err.kind).to_string(),
        message: err.kind.to_string(),
        line,
        x,
    }
}

/// A short, stable name for an error kind. Round-list imbalance maps to the
/// familiar parinfer names; the rest keep descriptive kebab-case names.
fn error_name(kind: &ErrorKind) -> &'static str {
    match kind {
        ErrorKind::UnclosedList { .. } => "unclosed-paren",
        ErrorKind::UnexpectedDelimiter { .. } => "unmatched-close-paren",
        ErrorKind::MismatchedDelimiter { .. } => "mismatched-close-paren",
        ErrorKind::MalformedToken { .. } => "malformed-token",
        ErrorKind::DanglingPrefix { .. } => "dangling-prefix",
        ErrorKind::DanglingTag => "dangling-tag",
        ErrorKind::DanglingLabel => "dangling-label",
        ErrorKind::DanglingDot => "dangling-dot",
        ErrorKind::ItemAfterDottedTail => "item-after-dotted-tail",
        ErrorKind::DepthLimitExceeded => "depth-limit-exceeded",
        // `ErrorKind` is `#[non_exhaustive]`.
        _ => "parse-error",
    }
}

/// The 0-based (line, character-column) of `byte` in `source`.
fn byte_to_line_col(source: &str, byte: usize) -> (usize, usize) {
    let byte = byte.min(source.len());
    let mut line = 0;
    let mut col = 0;
    for (i, ch) in source.char_indices() {
        if i >= byte {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Track a cursor across a paren-mode reindent. Reindent rewrites only each
/// line's leading whitespace and preserves line order (the reindent invariant),
/// so a cursor stays on its line and shifts by that line's indentation delta; a
/// cursor sitting inside the old indentation lands at the new indent.
fn remap_cursor(old: &str, new: &str, cursor: Cursor) -> Cursor {
    let old_line = old.lines().nth(cursor.line);
    let new_line = new.lines().nth(cursor.line);
    let (Some(old_line), Some(new_line)) = (old_line, new_line) else {
        return cursor;
    };
    let old_indent = leading_ws_chars(old_line);
    let new_indent = leading_ws_chars(new_line);
    let x = if cursor.x <= old_indent {
        new_indent
    } else {
        let shifted = cursor.x as isize + (new_indent as isize - old_indent as isize);
        shifted.max(0) as usize
    };
    Cursor {
        line: cursor.line,
        x,
    }
}

/// Count leading space/tab characters of `line`.
fn leading_ws_chars(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

/// Serialize an [`Answer`] to the shared structured result
/// `{text, success, error, cursorX, cursorLine}` (the `--json` CLI shape and the
/// MCP tool's return payload).
#[must_use]
pub fn answer_to_json(answer: &Answer) -> serde_json::Value {
    use serde_json::{json, Value};
    let error = match &answer.error {
        Some(e) => json!({
            "name": e.name,
            "message": e.message,
            "line": e.line,
            "x": e.x,
        }),
        None => Value::Null,
    };
    let (cursor_x, cursor_line) = match answer.cursor {
        Some(c) => (json!(c.x), json!(c.line)),
        None => (Value::Null, Value::Null),
    };
    json!({
        "text": answer.text,
        "success": answer.success,
        "error": error,
        "cursorX": cursor_x,
        "cursorLine": cursor_line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req<'a>(mode: Mode, text: &'a str, dialect: Dialect) -> Request<'a> {
        Request {
            mode,
            text,
            dialect,
            config: FormatConfig::default(),
            nameless: None,
            cursor: None,
        }
    }

    #[test]
    fn paren_mode_reindents_balanced_input_faithfully() {
        // Misindented but balanced: paren mode must produce exactly what the
        // faithful formatter would.
        let src = "(defun f (x)\n(+ x\n1))\n";
        let answer = run(&req(Mode::Paren, src, Dialect::EmacsLisp));
        assert!(answer.success);
        assert!(answer.error.is_none());
        let expected = format::format(src, &FormatConfig::default(), Dialect::EmacsLisp);
        assert_eq!(answer.text, expected);
        // And the result parses clean (balance generated / preserved).
        assert!(
            parse(&answer.text, &Options::for_dialect(Dialect::EmacsLisp))
                .errors
                .is_empty()
        );
    }

    #[test]
    fn paren_mode_refuses_imbalance_unchanged_with_diagnostic() {
        let src = "(defun f (x\n  (+ x 1)\n"; // unclosed
        let answer = run(&req(Mode::Paren, src, Dialect::EmacsLisp));
        assert!(!answer.success);
        assert_eq!(answer.text, src, "input must be returned unchanged");
        let err = answer.error.expect("a diagnostic");
        assert_eq!(err.name, "unclosed-paren");
        assert_eq!(err.line, 0, "opening line, 0-based");
    }

    #[test]
    fn cursor_tracks_indentation_delta() {
        // Line 1 (`(+ x`) is reindented from column 0 to column 2; a cursor on
        // that line, past the indentation, shifts right by the +2 delta.
        let src = "(defun f (x)\n(+ x\n1))\n";
        let mut r = req(Mode::Paren, src, Dialect::EmacsLisp);
        r.cursor = Some(Cursor { line: 1, x: 3 }); // on the `x` of `(+ x`
        let answer = run(&r);
        let c = answer.cursor.expect("a tracked cursor");
        assert_eq!(c.line, 1);
        assert_eq!(c.x, 5, "3 + (2 - 0) indentation delta");
    }

    #[test]
    fn cursor_in_indentation_lands_at_new_indent() {
        let src = "(defun f (x)\n(+ x\n1))\n";
        let mut r = req(Mode::Paren, src, Dialect::EmacsLisp);
        r.cursor = Some(Cursor { line: 1, x: 0 }); // inside the (empty) old indent
        let answer = run(&r);
        assert_eq!(
            answer.cursor.unwrap().x,
            2,
            "lands at the new indent column"
        );
    }

    #[test]
    fn nameless_production_matches_formatter() {
        // With the Nameless overlay, paren mode must equal the formatter's
        // Nameless path (column generation side).
        let src = "(defun php-mode-foo ()\n(php-mode-bar\n(baz)))\n";
        let nl = Nameless::for_file("php-mode.el");
        let mut r = req(Mode::Paren, src, Dialect::EmacsLisp);
        r.nameless = Some(Nameless::for_file("php-mode.el"));
        let answer = run(&r);
        let expected = format::format_elisp_nameless(src, &FormatConfig::default(), &nl);
        assert_eq!(answer.text, expected);
    }

    #[test]
    fn json_shape_has_all_fields() {
        let answer = run(&req(Mode::Paren, "(a)\n", Dialect::EmacsLisp));
        let v = answer_to_json(&answer);
        assert!(v.get("text").is_some());
        assert_eq!(v.get("success"), Some(&serde_json::json!(true)));
        assert_eq!(v.get("error"), Some(&serde_json::Value::Null));
        assert!(v.get("cursorX").is_some());
        assert!(v.get("cursorLine").is_some());
    }

    #[test]
    fn unknown_mode_name_is_none() {
        assert_eq!(Mode::from_name("indent"), None); // #25
        assert_eq!(Mode::from_name("paren"), Some(Mode::Paren));
    }
}
