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
//! - **Paren mode** ([`Mode::Paren`]): parens are the source of truth. Balanced
//!   input is reindented to lisplens's faithful Emacs indentation (reusing the
//!   formatter and its Nameless *production* path); unbalanced input is refused
//!   unchanged with a positioned diagnostic.
//! - **Indent mode** ([`Mode::Indent`]): indentation is the source of truth;
//!   close-parens are inferred from it over a tolerant `lex()` token scan. When
//!   the Nameless overlay is enabled, columns are read as they *display*
//!   (composed prefixes count as their shorter glyph), so indentation is
//!   interpreted the way a Nameless user sees it (ADR-0030). A supplied cursor
//!   locks the paren trail on its line (minimal cursor protection, #31).
//!
//! [`run_json`] / [`run_json_line`] run one request given as the shared JSON
//! shape — used by the MCP tool and the persistent `parinfer --server`
//! (ADR-0046).
//!
//! # Safety model (ADR-0045)
//!
//! Unlike the edit pipeline's error-parity rule (ADR-0005: never introduce a
//! *new* parse error), the parinfer command **generates balance**: on success
//! the output parses clean; on an unresolvable situation the input is returned
//! **completely unchanged** with `success = false` and a positioned diagnostic.
//! Broken output is never emitted either way.

use lispexp::{lex, parse, Delim, Dialect, ErrorKind, Options, ParseError, Token, TokenKind};
use unicode_width::UnicodeWidthChar;

use crate::config::FormatConfig;
use crate::format;
use crate::nameless::Nameless;

/// Which parinfer transform to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Parens are the source of truth; indentation is corrected. Requires
    /// balanced input.
    Paren,
    /// Indentation is the source of truth; close-parens are inferred from it.
    Indent,
}

impl Mode {
    /// Parse a mode name (`paren` / `indent`), or `None` if unrecognized.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Mode> {
        match name {
            "paren" => Some(Mode::Paren),
            "indent" => Some(Mode::Indent),
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
    /// The input cursor: tracked to its post-transform position, and in indent
    /// mode it also locks the paren trail on its line (minimal protection, #31).
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
        Mode::Indent => run_indent(req),
    }
}

/// Build the "refuse, unchanged" answer: input returned verbatim, `success`
/// false, the input cursor preserved (ADR-0045 failure contract).
fn refuse(req: &Request, error: Error) -> Answer {
    Answer {
        text: req.text.to_string(),
        success: false,
        error: Some(error),
        cursor: req.cursor,
    }
}

/// Paren mode: require balance, then faithfully reindent.
fn run_paren(req: &Request) -> Answer {
    let options = Options::for_dialect(req.dialect);
    let parsed = parse(req.text, &options);
    if let Some(err) = parsed.errors.first() {
        // Imbalance / unresolvable: return the input untouched + a diagnostic.
        return refuse(req, to_error(req.text, err));
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

// ---------------------------------------------------------------------------
// Indent mode (#25)
// ---------------------------------------------------------------------------

/// The page-break character (`^L`, U+000C). It is content, not indentation, so
/// indent mode preserves it verbatim rather than treating it as strippable
/// whitespace (matching the formatter, PR #51).
const PAGE_BREAK: char = '\u{0c}';

/// One structural delimiter token on a line.
struct DelimTok {
    /// Byte offset of the delimiter's start (`(`, `[`, `#(`, `)`, …).
    start: usize,
    /// Byte offset just past the delimiter.
    end: usize,
    /// The delimiter shape.
    delim: Delim,
    /// Whether it opens (`(`/`#(`) or closes (`)`).
    open: bool,
}

/// A rendered output line, split so that inferred close-parens append to the
/// end of the *code*, before any trailing comment.
struct OutLine {
    /// The code portion (leading whitespace preserved, trailing whitespace and
    /// movable close-parens stripped); inferred closers are appended here.
    code: String,
    /// The trailing comment portion, including the whitespace gap before it.
    comment: String,
}

/// Indent mode: indentation is authoritative; infer the close-paren placement.
///
/// A tolerant `lex()` token scan (parens inside strings / comments / char
/// literals are non-structural by construction) drives a stack of open
/// delimiters keyed by their display column. Each line's leading indentation
/// closes every open delimiter at or right of it, and its movable trailing
/// close-parens are re-derived rather than trusted. Indentation itself is never
/// changed. Unresolvable lexical states (unterminated string/comment,
/// end-of-line backslash, unmatched close-paren) refuse the input unchanged.
///
/// **Cursor protection (#31).** When a cursor is given, the paren trail on the
/// cursor's line is left untouched (locked, not stripped/re-inferred) so live
/// editing doesn't collapse the trail out from under the caret. If protecting it
/// would prevent a balanced result, the protection **yields**: the transform is
/// retried without it (ADR-0045 balance guarantee wins).
fn run_indent(req: &Request) -> Answer {
    if let Some(cursor) = req.cursor {
        let protected = run_indent_inner(req, Some(cursor.line));
        if protected.success {
            return protected;
        }
    }
    run_indent_inner(req, None)
}

/// The indent-mode core. `protect` names a line whose movable trailing
/// close-parens must be kept verbatim (locked) rather than stripped — the cursor
/// line under protection (#31); `None` processes every line normally.
fn run_indent_inner(req: &Request, protect: Option<usize>) -> Answer {
    let source = req.text;
    if source.trim().is_empty() {
        return Answer {
            text: source.to_string(),
            success: true,
            error: None,
            cursor: req.cursor,
        };
    }
    let options = Options::for_dialect(req.dialect);
    let tokens: Vec<Token> = lex(source, &options).collect();
    let ranges = line_ranges(source);

    if let Some(err) = unresolvable(source, &tokens, &ranges) {
        return refuse(req, err);
    }

    // Per-line structural facts.
    let n = ranges.len();
    let mut delims: Vec<Vec<DelimTok>> = (0..n).map(|_| Vec::new()).collect();
    let mut comment_start: Vec<Option<usize>> = vec![None; n];
    let mut in_multiline = vec![false; n]; // line *starts* inside a string/comment
                                           // Per line, the `(offset, columns_saved)` of each Nameless-composed prefix
                                           // (ADR-0030/#26). Column measurement subtracts these so indentation is read
                                           // in *displayed* columns — the parinfer-rust-emacs pain point. Empty unless
                                           // the Emacs-Lisp-only Nameless overlay is enabled.
    let mut savings: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for t in &tokens {
        let l = line_of(&ranges, t.span.start as usize);
        match t.kind {
            TokenKind::Open(d) | TokenKind::HashOpen(d) => delims[l].push(DelimTok {
                start: t.span.start as usize,
                end: t.span.end as usize,
                delim: d,
                open: true,
            }),
            TokenKind::Close(d) => delims[l].push(DelimTok {
                start: t.span.start as usize,
                end: t.span.end as usize,
                delim: d,
                open: false,
            }),
            TokenKind::LineComment => {
                if comment_start[l].is_none() {
                    comment_start[l] = Some(t.span.start as usize);
                }
            }
            TokenKind::Str | TokenKind::BlockComment => {
                let end_line = line_of(&ranges, (t.span.end as usize).saturating_sub(1));
                for entry in in_multiline.iter_mut().take(end_line + 1).skip(l + 1) {
                    *entry = true;
                }
            }
            TokenKind::Atom => {
                if let Some(nl) = &req.nameless {
                    let save = nl.saving(&source[t.span.start as usize..t.span.end as usize]);
                    if save > 0 {
                        savings[l].push((t.span.start as usize, save));
                    }
                }
            }
            _ => {}
        }
    }

    let tab_width = req.config.tab_width.max(1);
    let mut out: Vec<OutLine> = Vec::with_capacity(n);
    let mut stack: Vec<(Delim, usize)> = Vec::new(); // (delim, open column)
    let mut last_code: Option<usize> = None;

    for (l, &(ls, le)) in ranges.iter().enumerate() {
        if in_multiline[l] {
            // Inside a multi-line string/comment: emit verbatim, no structure.
            out.push(OutLine {
                code: source[ls..le].to_string(),
                comment: String::new(),
            });
            continue;
        }

        let code_region_end = comment_start[l].unwrap_or(le);

        // Strip the movable trailing close-parens (and the whitespace between
        // them): the run of `Close` tokens at the end of the code region. The
        // protected (cursor) line keeps its trail verbatim (#31) — its closers
        // stay and are scanned as locked, so live editing can't collapse them.
        // The gap check counts only spaces/tabs as strippable: a page break
        // (`^L`) after a closer is content, so that closer is locked in place
        // rather than stripped — stripping would discard the `^L` with it.
        let mut strip_from = code_region_end;
        if protect != Some(l) {
            for d in delims[l].iter().rev() {
                if d.start >= strip_from {
                    continue;
                }
                let gap_is_ws = source[d.end..strip_from]
                    .chars()
                    .all(|c| c == ' ' || c == '\t');
                if !d.open && gap_is_ws {
                    strip_from = d.start;
                } else {
                    break;
                }
            }
        }

        // Strip trailing spaces/tabs, but NOT a page break (`^L`): a page break is
        // content, not indentation, so `trim_end` (which counts `^L` as whitespace)
        // would silently delete the section separators of an Emacs Lisp file — the
        // same bug fixed in the formatter (PR #51). A line whose remaining content
        // is only whitespace and/or page breaks makes no indentation decision (like
        // a blank line) but is still emitted verbatim so the `^L` survives.
        let code = source[ls..strip_from].trim_end_matches([' ', '\t']);
        let code_owned = code.to_string();
        let structurally_blank = code
            .chars()
            .all(|c| c == ' ' || c == '\t' || c == PAGE_BREAK);
        // The comment portion keeps the whitespace immediately before it.
        let comment = match comment_start[l] {
            Some(cs) => {
                let before = &source[..cs];
                let gap = before.len() - before.trim_end_matches([' ', '\t']).len();
                source[cs - gap..le].to_string()
            }
            None => String::new(),
        };

        if structurally_blank {
            // A blank line, a comment-only line, a line that was only movable
            // closers, or a bare page-break separator: no code, so it makes no
            // indentation decision and is not a target for appended closers (a
            // closer would land on a comment or a `^L`). Any dropped closers are
            // re-inferred at the next code line or EOF. `code_owned` carries any
            // page break through verbatim; for a plain blank line it is empty.
            out.push(OutLine {
                code: code_owned,
                comment,
            });
            continue;
        }

        // Indentation column of the first code character (in displayed columns,
        // so Nameless-composed prefixes earlier on the line count as narrower).
        let indent_len = code.len() - code.trim_start_matches([' ', '\t']).len();
        let x = display_col(source, ls, ls + indent_len, tab_width, &savings[l]);

        // Close every open delimiter at or to the right of this indentation,
        // appending its closer to the previous code line.
        while let Some(&(d, c)) = stack.last() {
            if c >= x {
                stack.pop();
                append_closer(&mut out, last_code, d);
            } else {
                break;
            }
        }

        out.push(OutLine {
            code: code_owned,
            comment,
        });
        let this = out.len() - 1;
        last_code = Some(this);

        // Track the retained (locked, mid-line) delimiters against the stack.
        for d in &delims[l] {
            if d.start >= strip_from {
                continue;
            }
            if d.open {
                let col = display_col(source, ls, d.start, tab_width, &savings[l]);
                stack.push((d.delim, col));
            } else {
                match stack.last() {
                    Some(&(od, _)) if close_class(od) == close_class(d.delim) => {
                        stack.pop();
                    }
                    _ => {
                        let (line, col) = byte_to_line_col(source, d.start);
                        return refuse(
                            req,
                            Error {
                                name: "unmatched-close-paren".to_string(),
                                message: "unmatched close paren".to_string(),
                                line,
                                x: col,
                            },
                        );
                    }
                }
            }
        }
    }

    // Close everything still open at end of input.
    while let Some((d, _)) = stack.pop() {
        append_closer(&mut out, last_code, d);
    }

    let mut text = String::with_capacity(source.len() + 8);
    for (i, ol) in out.iter().enumerate() {
        if i > 0 {
            text.push('\n');
        }
        text.push_str(&ol.code);
        text.push_str(&ol.comment);
    }
    if source.ends_with('\n') {
        text.push('\n');
    }

    // The transform is balance-generating (ADR-0045): the result must parse
    // clean. It always should by construction; refuse defensively if not.
    if let Some(err) = parse(&text, &options).errors.first() {
        return refuse(req, to_error(&text, err));
    }

    let cursor = req.cursor.map(|c| clamp_cursor(&text, c));
    Answer {
        text,
        success: true,
        error: None,
        cursor,
    }
}

/// The byte range `[start, end)` (excluding the newline) of each line.
fn line_ranges(source: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let bytes = source.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            ranges.push((start, i));
            start = i + 1;
        }
    }
    if start < source.len() || source.is_empty() {
        ranges.push((start, source.len()));
    }
    ranges
}

/// The 0-based line index containing `byte` (a newline counts as ending its
/// line). `ranges` is sorted by start, so this is a binary search.
fn line_of(ranges: &[(usize, usize)], byte: usize) -> usize {
    match ranges.binary_search_by(|&(s, _)| s.cmp(&byte)) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

/// The token covering `byte`, if any (tokens tile the input).
fn token_at(tokens: &[Token], byte: usize) -> Option<&Token> {
    let i = tokens.partition_point(|t| (t.span.start as usize) <= byte);
    tokens
        .get(i.wrapping_sub(1))
        .filter(|t| (t.span.start as usize) <= byte && byte < t.span.end as usize)
}

/// Detect the lexical states indent mode cannot resolve into balanced output:
/// an unterminated string/comment/… (from the lexer's own `Unterminated` state)
/// or a backslash escaping a line end outside a string/comment/char.
fn unresolvable(source: &str, tokens: &[Token], ranges: &[(usize, usize)]) -> Option<Error> {
    for t in tokens {
        if let TokenKind::Unterminated(_) = t.kind {
            let (line, x) = byte_to_line_col(source, t.span.start as usize);
            return Some(Error {
                name: "unclosed-quote".to_string(),
                message: "unterminated string, comment, or delimited token".to_string(),
                line,
                x,
            });
        }
    }
    let bytes = source.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\\' && (i + 1 == bytes.len() || bytes[i + 1] == b'\n') {
            let benign = token_at(tokens, i).is_some_and(|t| {
                matches!(
                    t.kind,
                    TokenKind::Str
                        | TokenKind::LineComment
                        | TokenKind::BlockComment
                        | TokenKind::Char
                        | TokenKind::Bool(_)
                )
            });
            if !benign {
                let _ = ranges;
                let (line, x) = byte_to_line_col(source, i);
                return Some(Error {
                    name: "eol-backslash".to_string(),
                    message: "backslash at end of line".to_string(),
                    line,
                    x,
                });
            }
        }
    }
    None
}

/// The displayed column of byte `offset` on the line starting at `line_start`,
/// expanding tabs to the next multiple of `tab_width` and measuring glyphs by
/// East Asian Width (Emacs's `current-column`), then subtracting the savings of
/// every Nameless-composed prefix that begins earlier on the line (ADR-0030) —
/// so indentation and open-paren columns are read as the user sees them.
fn display_col(
    source: &str,
    line_start: usize,
    offset: usize,
    tab_width: usize,
    savings: &[(usize, usize)],
) -> usize {
    let raw = col_at(&source[line_start..offset], tab_width);
    let saved: usize = savings
        .iter()
        .filter(|(o, _)| *o < offset)
        .map(|(_, s)| *s)
        .sum();
    raw.saturating_sub(saved)
}

/// The raw display column of the string `prefix` (a line prefix), expanding tabs
/// to the next multiple of `tab_width` and measuring glyphs by East Asian Width
/// — matching Emacs's `current-column` before any Nameless composition.
fn col_at(prefix: &str, tab_width: usize) -> usize {
    let mut col = 0;
    for ch in prefix.chars() {
        if ch == '\t' {
            col = (col / tab_width + 1) * tab_width;
        } else {
            col += UnicodeWidthChar::width(ch).unwrap_or(0);
        }
    }
    col
}

/// A close-delimiter equivalence class: `}` closes both `{` and `#{`.
fn close_class(delim: Delim) -> u8 {
    match delim {
        Delim::Round => 0,
        Delim::Square => 1,
        Delim::Curly | Delim::Set => 2,
    }
}

/// The closing glyph for a delimiter shape.
fn close_glyph(delim: Delim) -> &'static str {
    match delim {
        Delim::Round => ")",
        Delim::Square => "]",
        Delim::Curly | Delim::Set => "}",
    }
}

/// Append the closer for `delim` to the end of the last code line's code.
fn append_closer(out: &mut [OutLine], last_code: Option<usize>, delim: Delim) {
    if let Some(i) = last_code {
        out[i].code.push_str(close_glyph(delim));
    }
}

/// Track a cursor across an indent-mode transform. Indentation and code are
/// preserved verbatim up to each line's original code end (only movable trailing
/// closers/whitespace change and inferred closers append), and the line count is
/// preserved, so the cursor keeps its line and clamps to the new line length.
fn clamp_cursor(new: &str, cursor: Cursor) -> Cursor {
    let len = new
        .lines()
        .nth(cursor.line)
        .map_or(cursor.x, |l| l.chars().count());
    Cursor {
        line: cursor.line,
        x: cursor.x.min(len),
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

/// An answer-shaped JSON value for a request that could not even be run (bad
/// mode, unknown dialect, malformed JSON): `success = false`, the given text
/// echoed unchanged, and a positioned-at-origin error. Keeps every request →
/// exactly-one-answer, so a persistent server never desynchronizes.
fn error_answer(text: &str, name: &str, message: &str) -> serde_json::Value {
    serde_json::json!({
        "text": text,
        "success": false,
        "error": { "name": name, "message": message, "line": 0, "x": 0 },
        "cursorX": serde_json::Value::Null,
        "cursorLine": serde_json::Value::Null,
    })
}

/// Run one request given as a JSON object `{mode, text, dialect?, nameless?,
/// name?, cursorLine?, cursorX?}` (the MCP `parinfer` tool's shape and the
/// server's line protocol), returning the answer as a JSON value. Invalid input
/// yields an `error_answer` rather than panicking, so callers can always emit
/// exactly one answer per request.
#[must_use]
pub fn run_json(request: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    let text = request.get("text").and_then(Value::as_str).unwrap_or("");
    let Some(mode) = request
        .get("mode")
        .and_then(Value::as_str)
        .and_then(Mode::from_name)
    else {
        return error_answer(
            text,
            "bad-request",
            "missing or unknown `mode` (paren|indent)",
        );
    };
    let dialect = match request.get("dialect").and_then(Value::as_str) {
        Some(d) => match d.parse::<Dialect>() {
            Ok(d) => d,
            Err(_) => return error_answer(text, "bad-request", &format!("unknown dialect `{d}`")),
        },
        None => Dialect::EmacsLisp,
    };
    let nameless_on = request
        .get("nameless")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let name_hint = request.get("name").and_then(Value::as_str);
    let cursor = match (
        request.get("cursorLine").and_then(Value::as_u64),
        request.get("cursorX").and_then(Value::as_u64),
    ) {
        (Some(line), Some(x)) => Some(Cursor {
            line: line as usize,
            x: x as usize,
        }),
        _ => None,
    };
    let mut config = FormatConfig::default();
    config.nameless |= nameless_on;
    let nameless = if config.nameless && dialect == Dialect::EmacsLisp {
        Some(Nameless::for_file(name_hint.unwrap_or("")))
    } else {
        None
    };
    answer_to_json(&run(&Request {
        mode,
        text,
        dialect,
        config,
        nameless,
        cursor,
    }))
}

/// Parse one line of the server protocol (a JSON request object) and run it,
/// always returning exactly one answer-shaped JSON value — a malformed line
/// becomes an `error_answer` so the server stays in lock-step.
#[must_use]
pub fn run_json_line(line: &str) -> serde_json::Value {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(request) => run_json(&request),
        Err(e) => error_answer("", "bad-json", &format!("malformed request JSON: {e}")),
    }
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
    fn mode_name_parsing() {
        assert_eq!(Mode::from_name("paren"), Some(Mode::Paren));
        assert_eq!(Mode::from_name("indent"), Some(Mode::Indent));
        assert_eq!(Mode::from_name("smart"), None);
    }

    // --- indent mode ---

    fn indent(src: &str, dialect: Dialect) -> Answer {
        run(&req(Mode::Indent, src, dialect))
    }

    /// Every successful indent-mode result must parse clean (balance-generating).
    fn assert_balanced(text: &str, dialect: Dialect) {
        assert!(
            parse(text, &Options::for_dialect(dialect))
                .errors
                .is_empty(),
            "expected balanced output, got:\n{text}"
        );
    }

    #[test]
    fn indent_infers_closers_at_eof() {
        let a = indent("(defun f (x)\n  (+ x\n     1", Dialect::EmacsLisp);
        assert!(a.success);
        assert_eq!(a.text, "(defun f (x)\n  (+ x\n     1))");
        assert_balanced(&a.text, Dialect::EmacsLisp);
    }

    #[test]
    fn indent_dedent_closes_and_indentation_is_untouched() {
        // Indentation is authoritative and never rewritten; only closers move.
        let a = indent("(when x\n  (foo)\n  (bar))", Dialect::EmacsLisp);
        assert_eq!(a.text, "(when x\n  (foo)\n  (bar))");
    }

    #[test]
    fn indent_moves_paren_when_next_line_is_indented_deeper() {
        // qux is indented inside bar, so bar's close-paren moves after qux.
        let a = indent("(foo (bar baz)\n          qux)\n", Dialect::EmacsLisp);
        assert_eq!(a.text, "(foo (bar baz\n          qux))\n");
        assert_balanced(&a.text, Dialect::EmacsLisp);
    }

    #[test]
    fn indent_places_closer_before_a_trailing_comment() {
        let a = indent("(defun f ()\n  body  ; hi\n", Dialect::EmacsLisp);
        assert_eq!(a.text, "(defun f ()\n  body)  ; hi\n");
        assert_balanced(&a.text, Dialect::EmacsLisp);
    }

    #[test]
    fn indent_ignores_parens_inside_strings() {
        let a = indent("(list \"a)b\"\n      c\n", Dialect::EmacsLisp);
        assert_eq!(a.text, "(list \"a)b\"\n      c)\n");
        assert_balanced(&a.text, Dialect::EmacsLisp);
    }

    #[test]
    fn indent_all_trail_line_moves_closer_up() {
        let a = indent("(foo\n  bar\n  )\n", Dialect::EmacsLisp);
        assert_eq!(a.text, "(foo\n  bar)\n\n");
        assert_balanced(&a.text, Dialect::EmacsLisp);
    }

    #[test]
    fn indent_trims_superfluous_trailing_closers() {
        let a = indent("(foo))\n", Dialect::EmacsLisp);
        assert!(a.success);
        assert_eq!(a.text, "(foo)\n");
    }

    #[test]
    fn indent_clojure_brackets_maps_and_sets() {
        let a = indent("(defn f [x]\n  {:a 1\n   :b #{1 2}", Dialect::Clojure);
        assert!(a.success);
        assert_eq!(a.text, "(defn f [x]\n  {:a 1\n   :b #{1 2}})");
        assert_balanced(&a.text, Dialect::Clojure);
    }

    #[test]
    fn indent_is_idempotent_on_balanced_input() {
        let src = "(defun f (x)\n  (let ((y (+ x 1)))\n    (message \"%d\" y)))\n";
        let once = indent(src, Dialect::EmacsLisp);
        assert_eq!(once.text, src, "well-formed input is unchanged");
        let twice = indent(&once.text, Dialect::EmacsLisp);
        assert_eq!(twice.text, once.text, "idempotent");
    }

    #[test]
    fn indent_refuses_unterminated_string_unchanged() {
        let src = "(foo \"bar\n";
        let a = indent(src, Dialect::EmacsLisp);
        assert!(!a.success);
        assert_eq!(a.text, src);
        assert_eq!(a.error.unwrap().name, "unclosed-quote");
    }

    #[test]
    fn indent_refuses_eol_backslash_unchanged() {
        let src = "(foo bar\\\n baz)\n";
        let a = indent(src, Dialect::EmacsLisp);
        assert!(!a.success);
        assert_eq!(a.text, src);
        assert_eq!(a.error.unwrap().name, "eol-backslash");
    }

    #[test]
    fn indent_refuses_mid_line_unmatched_close() {
        let a = indent("a)b\n", Dialect::EmacsLisp);
        assert!(!a.success);
        assert_eq!(a.error.unwrap().name, "unmatched-close-paren");
    }

    #[test]
    fn indent_cursor_is_clamped_to_line_length() {
        let mut r = req(Mode::Indent, "(a\n  b", Dialect::EmacsLisp);
        r.cursor = Some(Cursor { line: 1, x: 99 });
        let a = run(&r);
        let c = a.cursor.unwrap();
        assert_eq!(c.line, 1);
        assert_eq!(c.x, 4, "clamped to `  b)` length");
    }

    #[test]
    fn indent_empty_input_is_unchanged() {
        let a = indent("", Dialect::EmacsLisp);
        assert!(a.success);
        assert_eq!(a.text, "");
    }

    /// The #26 headline: under Nameless, `php-foo` displays as `:foo`, so the
    /// inner `(` sits at displayed column 6, not raw column 9. `baz` indented to
    /// 7 spaces is therefore *inside* `bar` (7 > 6). Without the overlay, naive
    /// raw-column reading (7 < 9) would close `bar` and kick `baz` out — exactly
    /// the parinfer-rust-emacs pain point.
    #[test]
    fn indent_nameless_reads_displayed_columns() {
        let src = "(php-foo (bar\n       baz";
        let mut r = req(Mode::Indent, src, Dialect::EmacsLisp);
        r.nameless = Some(Nameless::for_file("php-mode.el"));
        let a = run(&r);
        assert!(a.success);
        assert_eq!(
            a.text, "(php-foo (bar\n       baz))",
            "baz stays inside bar"
        );
        assert_balanced(&a.text, Dialect::EmacsLisp);
    }

    #[test]
    fn indent_nameless_off_is_unchanged_from_core() {
        // Same input, no overlay: raw columns kick baz out of bar (the #25 path).
        let src = "(php-foo (bar\n       baz";
        let a = indent(src, Dialect::EmacsLisp);
        assert_eq!(a.text, "(php-foo (bar)\n       baz)");
    }

    // --- page breaks (^L) are content, not whitespace ---

    /// Paren mode reindents through the formatter, which preserves `^L` page-break
    /// separators (PR #51). The transform must equal the formatter's output and
    /// keep every `^L` — parinfer must not reintroduce the separator-deleting bug.
    #[test]
    fn paren_mode_preserves_page_breaks() {
        let src = "(defun a ()\n1)\n\u{0c}\n(defun b ()\n2)\n";
        let answer = run(&req(Mode::Paren, src, Dialect::EmacsLisp));
        assert!(answer.success);
        let expected = format::format(src, &FormatConfig::default(), Dialect::EmacsLisp);
        assert_eq!(answer.text, expected, "paren mode must equal the formatter");
        assert_eq!(
            answer.text.matches('\u{0c}').count(),
            1,
            "the page break survives"
        );
    }

    /// Indent mode must preserve a bare `^L` line: it is content, not strippable
    /// whitespace, so `trim_end` must not eat it (the formatter bug, PR #51). A
    /// page-break-only line makes no indentation decision but is emitted verbatim,
    /// leaving well-formed input byte-identical (and the transform idempotent).
    #[test]
    fn indent_preserves_a_page_break_line() {
        let src = "(defun a ()\n  1)\n\u{0c}\n(defun b ()\n  2)\n";
        let a = indent(src, Dialect::EmacsLisp);
        assert!(a.success);
        assert_eq!(a.text, src, "page break kept, structure untouched");
        assert_balanced(&a.text, Dialect::EmacsLisp);
        // And running it again changes nothing.
        assert_eq!(indent(&a.text, Dialect::EmacsLisp).text, src);
    }

    /// A `^L` sharing a line with a following comment survives too — the page break
    /// stays as leading content, the comment after it is preserved, and the line is
    /// treated as structurally blank (it makes no indentation decision).
    #[test]
    fn indent_preserves_a_page_break_before_a_comment() {
        let src = "(defun a ()\n  1)\n\u{0c};; Section\n(defun b ()\n  2)\n";
        let a = indent(src, Dialect::EmacsLisp);
        assert!(a.success);
        assert_eq!(a.text, src, "`^L;; Section` line preserved verbatim");
    }

    /// A closer immediately followed by a `^L` is locked, not stripped: stripping
    /// the "movable" trail would discard the page break with it. Well-formed input
    /// of this shape passes through byte-identical.
    #[test]
    fn indent_locks_a_closer_followed_by_a_page_break() {
        let src = "(foo\n  bar)\u{0c}\n(baz)\n";
        let a = indent(src, Dialect::EmacsLisp);
        assert!(a.success);
        assert_eq!(a.text, src, "closer + `^L` kept verbatim");
        assert_balanced(&a.text, Dialect::EmacsLisp);
    }

    /// A `^L` *inside* a paren trail splits it: the closer before the `^L` is
    /// locked (keeping the page break), while closers after it are still movable —
    /// here the final one is superfluous and gets trimmed, and indentation
    /// authority still restructures as usual. The `^L` must survive it all.
    #[test]
    fn indent_page_break_inside_a_trail_survives() {
        let a = indent("(foo (bar\n  baz)\u{0c})\n", Dialect::EmacsLisp);
        assert!(a.success);
        assert_eq!(a.text, "(foo (bar)\n  baz)\u{0c}\n");
        assert_balanced(&a.text, Dialect::EmacsLisp);
    }

    // --- cursor-line trail protection (#31) ---

    #[test]
    fn indent_cursor_protects_the_trail_on_its_line() {
        // Without a cursor, bar's close-paren is stripped and the deep `baz` is
        // pulled inside bar. With the cursor on bar's line (its paren trail), that
        // closer is locked — `baz` is not yanked in, the trail stays put.
        let src = "(foo (bar)\n          baz";
        let plain = indent(src, Dialect::EmacsLisp);
        assert_eq!(plain.text, "(foo (bar\n          baz))");

        let mut r = req(Mode::Indent, src, Dialect::EmacsLisp);
        r.cursor = Some(Cursor { line: 0, x: 10 });
        let protected = run(&r);
        assert!(protected.success);
        assert_eq!(protected.text, "(foo (bar)\n          baz)");
        assert_balanced(&protected.text, Dialect::EmacsLisp);
    }

    #[test]
    fn indent_protection_yields_to_preserve_balance() {
        // Protecting the cursor line's trail would keep an extra `)` and unbalance
        // the result, so protection yields and the transform trims it (ADR-0045).
        let mut r = req(Mode::Indent, "(a))\n", Dialect::EmacsLisp);
        r.cursor = Some(Cursor { line: 0, x: 3 });
        let a = run(&r);
        assert!(a.success);
        assert_eq!(a.text, "(a)\n");
    }

    // --- server / JSON request shape (#30) ---

    #[test]
    fn run_json_runs_both_modes() {
        let paren = run_json(&serde_json::json!({ "mode": "paren", "text": "(a\n(b))\n" }));
        assert_eq!(paren["success"], serde_json::json!(true));
        let indent = run_json(&serde_json::json!({ "mode": "indent", "text": "(a\n  (b" }));
        assert_eq!(indent["success"], serde_json::json!(true));
        assert_eq!(indent["text"], serde_json::json!("(a\n  (b))"));
    }

    #[test]
    fn run_json_bad_request_echoes_text_unchanged() {
        // Missing mode → success:false, input echoed, so a server stays in lock-step.
        let a = run_json(&serde_json::json!({ "text": "(a)" }));
        assert_eq!(a["success"], serde_json::json!(false));
        assert_eq!(a["text"], serde_json::json!("(a)"));
        assert_eq!(a["error"]["name"], serde_json::json!("bad-request"));
    }

    #[test]
    fn run_json_line_handles_malformed_json() {
        let a = run_json_line("not json");
        assert_eq!(a["success"], serde_json::json!(false));
        assert_eq!(a["error"]["name"], serde_json::json!("bad-json"));
    }

    #[test]
    fn run_json_carries_cursor_and_dialect() {
        let a = run_json(&serde_json::json!({
            "mode": "indent", "dialect": "clojure",
            "text": "(defn f [x]\n  {:a 1", "cursorLine": 1, "cursorX": 6
        }));
        assert_eq!(a["success"], serde_json::json!(true));
        assert_eq!(a["text"], serde_json::json!("(defn f [x]\n  {:a 1})"));
        assert_eq!(a["cursorLine"], serde_json::json!(1));
    }
}
