//! lisplens CLI (skeleton).
//!
//! One subcommand so far: `outline <file>`. The mode-first command surface and
//! MCP server described in ADR-0006 are not built yet.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;

/// A `--dialect NAME` override, resolved once from the command line and consulted
/// by [`resolve_dialect`]. `None` (or unset) means "guess from the file extension".
static DIALECT_OVERRIDE: OnceLock<Option<lisplens::Dialect>> = OnceLock::new();

/// The dialect for `path`: the `--dialect` override if one was given, else the
/// extension guess ([`lisplens::dialect_for_path`]). Single-file commands route
/// through here so `--dialect` can force an ambiguous extension (`.lsp` is Common
/// Lisp / AutoLISP / ISLisp); project-wide `find`/`refs` keep their per-file guess.
fn resolve_dialect(path: &Path) -> lisplens::Dialect {
    DIALECT_OVERRIDE
        .get()
        .copied()
        .flatten()
        .unwrap_or_else(|| lisplens::dialect_for_path(path))
}

/// Parse a dialect from its kebab-case name (`islisp`, `common-lisp`, `clojure`, …).
fn parse_dialect(name: &str) -> Result<lisplens::Dialect, String> {
    name.parse::<lisplens::Dialect>().map_err(|_| {
        format!("unknown --dialect `{name}` (try islisp, common-lisp, clojure, scheme, …)")
    })
}

/// Strip a global `--dialect NAME` / `--dialect=NAME` flag out of `args` (last one
/// wins), returning the parsed dialect. Leaves the remaining args for the
/// subcommand matcher; errors on an unknown or value-less flag.
fn take_dialect_flag(args: &mut Vec<String>) -> Result<Option<lisplens::Dialect>, String> {
    let mut result = None;
    let mut i = 0;
    while i < args.len() {
        if let Some(name) = args[i].strip_prefix("--dialect=") {
            result = Some(parse_dialect(name)?);
            args.remove(i);
        } else if args[i] == "--dialect" {
            let name = args
                .get(i + 1)
                .ok_or_else(|| "--dialect needs a value".to_string())?
                .clone();
            result = Some(parse_dialect(&name)?);
            args.remove(i); // the flag
            args.remove(i); // its value
        } else {
            i += 1;
        }
    }
    Ok(result)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let override_dialect = match take_dialect_flag(&mut args) {
        Ok(d) => d,
        Err(msg) => {
            eprintln!("lisplens: {msg}");
            return ExitCode::FAILURE;
        }
    };
    let _ = DIALECT_OVERRIDE.set(override_dialect);
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["struct", "read", file] => run_struct_read(PathBuf::from(file)),
        ["struct", "read", file, name] => run_struct_expand(PathBuf::from(file), name),
        ["line", "read", file] => run_line_read(PathBuf::from(file)),
        ["line", "edit", file] => run_line_edit(PathBuf::from(file)),
        ["struct", "edit", file] => run_struct_edit(PathBuf::from(file)),
        ["find", name] => run_find(name, "."),
        ["find", name, dir] => run_find(name, dir),
        ["refs", name] => run_refs(name, "."),
        ["refs", name, dir] => run_refs(name, dir),
        ["format", args @ ..] => run_format(args),
        ["check", file] => run_check(PathBuf::from(file)),
        ["rename", from, to, file] => run_rename(from, to, PathBuf::from(file)),
        ["inline", name, file] => run_inline(name, PathBuf::from(file)),
        ["docstring", name, file] => run_docstring(name, PathBuf::from(file)),
        ["rewrite", file] => run_rewrite(PathBuf::from(file)),
        ["extract", file, anchor, name, params @ ..] => {
            run_extract(PathBuf::from(file), anchor, name, params)
        }
        ["mcp"] => match lisplens::mcp::serve() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("lisplens: mcp: {err}");
                ExitCode::FAILURE
            }
        },
        ["--help"] | ["-h"] | ["help"] => help(),
        _ => usage(),
    }
}

fn run_struct_expand(path: PathBuf, name: &str) -> ExitCode {
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let dialect = resolve_dialect(&path);
    print!("{}", lisplens::expand_text(&source, dialect, name));
    ExitCode::SUCCESS
}

fn run_line_read(path: PathBuf) -> ExitCode {
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    print!(
        "{}",
        lisplens::linehash::read(&path.display().to_string(), &source)
    );
    ExitCode::SUCCESS
}

fn run_struct_read(path: PathBuf) -> ExitCode {
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let dialect = resolve_dialect(&path);
    print!("{}", lisplens::outline_text(&source, dialect));
    ExitCode::SUCCESS
}

fn read_stdin() -> Option<String> {
    let mut input = String::new();
    match std::io::Read::read_to_string(&mut std::io::stdin(), &mut input) {
        Ok(_) => Some(input),
        Err(err) => {
            eprintln!("lisplens: reading patch from stdin: {err}");
            None
        }
    }
}

fn report(result: Result<lisplens::patch::Outcome, lisplens::patch::ApplyError>) -> ExitCode {
    match result {
        Ok(outcome) => {
            println!("ok {}", outcome.new_file_hash);
            for warning in &outcome.warnings {
                eprintln!("warning: {warning}");
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lisplens: {err:?}");
            ExitCode::FAILURE
        }
    }
}

fn run_line_edit(path: PathBuf) -> ExitCode {
    let Some(input) = read_stdin() else {
        return ExitCode::FAILURE;
    };
    let patch = match lisplens::patch::parse_line_patch(&input) {
        Ok(patch) => patch,
        Err(err) => {
            eprintln!("lisplens: patch parse error: {err:?}");
            return ExitCode::FAILURE;
        }
    };
    let dialect = resolve_dialect(&path);
    report(lisplens::patch::apply_line_patch(&path, &patch, dialect))
}

fn run_struct_edit(path: PathBuf) -> ExitCode {
    let Some(input) = read_stdin() else {
        return ExitCode::FAILURE;
    };
    let patch = match lisplens::patch::parse_struct_patch(&input) {
        Ok(patch) => patch,
        Err(err) => {
            eprintln!("lisplens: patch parse error: {err:?}");
            return ExitCode::FAILURE;
        }
    };
    let dialect = resolve_dialect(&path);
    report(lisplens::patch::apply_struct_patch(&path, &patch, dialect))
}

fn run_format(args: &[&str]) -> ExitCode {
    // `--nameless` (Emacs Lisp, ADR-0030) and `--tonsky` (Clojure fixed style,
    // ADR-0040) are flags; the remaining argument is the file. Filenames never
    // begin with `--`.
    let nameless = args.contains(&"--nameless");
    let tonsky = args.contains(&"--tonsky");
    let Some(file) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("lisplens: format: no file given");
        return ExitCode::FAILURE;
    };
    let path = PathBuf::from(*file);
    let dialect = resolve_dialect(&path);
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let mut config = lisplens::config::resolve(&path, &source);
    // `--tonsky` forces the Clojure fixed style on (a `clojure-ts-indent-style:
    // fixed` file-/dir-local resolves it too).
    config.clojure_fixed_indent |= tonsky;
    // Nameless emulation is Emacs Lisp-only (ADR-0030); `--nameless` forces it
    // on, a `nameless-mode` file-/dir-local resolves it too. Every other dialect
    // (Common Lisp, and the generic fallback for the rest) formats by dialect.
    let formatted = if (nameless || config.nameless) && dialect == lisplens::Dialect::EmacsLisp {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let nl = lisplens::nameless::Nameless::for_file(file_name);
        lisplens::format::format_elisp_nameless(&source, &config, &nl)
    } else {
        lisplens::format::format(&source, &config, dialect)
    };
    if formatted != source {
        if let Err(err) = lisplens::write::write_atomically(&path, &formatted) {
            eprintln!("lisplens: {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn run_check(path: PathBuf) -> ExitCode {
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let dialect = resolve_dialect(&path);
    let diagnostics = lisplens::check(&source, dialect);
    // Silent success (exit 0); parse diagnostics to stderr + non-zero on errors,
    // so the check composes in CI and agent pipelines (ADR-0032).
    if diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        let path = path.display().to_string();
        eprint!("{}", lisplens::diagnostics_text(&path, &diagnostics));
        ExitCode::FAILURE
    }
}

fn run_rename(from: &str, to: &str, path: PathBuf) -> ExitCode {
    let dialect = resolve_dialect(&path);
    match lisplens::refactor::rename_symbol_in_file(&path, from, to, dialect) {
        Ok(outcome) => {
            println!(
                "renamed {} occurrence(s) of `{from}` -> `{to}`  {}",
                outcome.renamed, outcome.new_file_hash
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            ExitCode::FAILURE
        }
    }
}

fn run_inline(name: &str, path: PathBuf) -> ExitCode {
    let dialect = resolve_dialect(&path);
    match lisplens::refactor::inline_definition_in_file(&path, name, dialect) {
        Ok(outcome) => {
            println!(
                "inlined {} call site(s) of `{name}`  {}",
                outcome.inlined, outcome.new_file_hash
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            ExitCode::FAILURE
        }
    }
}

fn run_docstring(name: &str, path: PathBuf) -> ExitCode {
    let Some(text) = read_stdin() else {
        return ExitCode::FAILURE;
    };
    let dialect = resolve_dialect(&path);
    match lisplens::refactor::set_docstring_in_file(&path, name, &text, dialect) {
        Ok(outcome) => {
            let verb = match outcome.action {
                lisplens::refactor::DocstringAction::Inserted => "set",
                lisplens::refactor::DocstringAction::Replaced => "replaced",
            };
            println!("{verb} docstring on `{name}`  {}", outcome.new_file_hash);
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            ExitCode::FAILURE
        }
    }
}

fn run_rewrite(path: PathBuf) -> ExitCode {
    let Some(spec) = read_stdin() else {
        return ExitCode::FAILURE;
    };
    let dialect = resolve_dialect(&path);
    match lisplens::refactor::rewrite_in_file(&path, &spec, dialect) {
        Ok(outcome) => {
            println!(
                "rewrote {} site(s)  {}",
                outcome.rewritten, outcome.new_file_hash
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            ExitCode::FAILURE
        }
    }
}

fn run_extract(path: PathBuf, anchor: &str, name: &str, args: &[&str]) -> ExitCode {
    let dialect = resolve_dialect(&path);
    let opts = match parse_extract_opts(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("lisplens: extract: {msg}");
            return ExitCode::FAILURE;
        }
    };
    // `--also` (generalizing multi-anchor) is a distinct site-selection mode.
    if !opts.also.is_empty() {
        if opts.all {
            eprintln!("lisplens: extract: --also cannot be combined with --all");
            return ExitCode::FAILURE;
        }
        if opts.count != 1 {
            eprintln!("lisplens: extract: --also cannot be combined with --count");
            return ExitCode::FAILURE;
        }
        let result = lisplens::refactor::extract_generalized(
            &path,
            anchor,
            &opts.also,
            name,
            &opts.params,
            opts.kind.as_deref(),
            dialect,
        );
        return report_extract(&path, name, result);
    }
    let extract = if opts.all {
        lisplens::refactor::extract_multi_site
    } else {
        lisplens::refactor::extract_block_into_function
    };
    let result = extract(
        &path,
        anchor,
        name,
        &opts.params,
        opts.count,
        opts.kind.as_deref(),
        dialect,
    );
    report_extract(&path, name, result)
}

/// Print the outcome of an extraction, mapping it to an exit code.
fn report_extract(
    path: &std::path::Path,
    name: &str,
    result: Result<lisplens::refactor::ExtractOutcome, lisplens::refactor::ExtractError>,
) -> ExitCode {
    match result {
        Ok(outcome) => {
            println!(
                "extracted `{name}` at {} site(s)  {}",
                outcome.sites, outcome.new_file_hash
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            ExitCode::FAILURE
        }
    }
}

/// The parsed `extract` options.
struct ExtractOpts {
    count: usize,
    kind: Option<String>,
    all: bool,
    also: Vec<String>,
    params: Vec<String>,
}

/// Split `--count N` / `--count=N` (default 1), `--kind HEAD` / `--kind=HEAD`
/// (default: dialect's defun/define/defn), `--all` (default off), and `--also
/// ANCHOR` / `--also=ANCHOR` (repeatable, additional generalizing sites) out of
/// `extract`'s trailing args; everything else is a parameter symbol. Params are
/// Lisp symbols, so none begins with `--`, so this never swallows a real parameter.
fn parse_extract_opts(args: &[&str]) -> Result<ExtractOpts, String> {
    let mut count = 1usize;
    let mut kind = None;
    let mut all = false;
    let mut also = Vec::new();
    let mut params = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if let Some(n) = a.strip_prefix("--count=") {
            count = n.parse().map_err(|_| format!("invalid --count `{n}`"))?;
        } else if a == "--count" {
            let n = args
                .get(i + 1)
                .ok_or_else(|| "--count needs a value".to_string())?;
            count = n.parse().map_err(|_| format!("invalid --count `{n}`"))?;
            i += 1;
        } else if let Some(h) = a.strip_prefix("--kind=") {
            kind = Some(h.to_string());
        } else if a == "--kind" {
            let h = args
                .get(i + 1)
                .ok_or_else(|| "--kind needs a value".to_string())?;
            kind = Some(h.to_string());
            i += 1;
        } else if let Some(anchor) = a.strip_prefix("--also=") {
            also.push(anchor.to_string());
        } else if a == "--also" {
            let anchor = args
                .get(i + 1)
                .ok_or_else(|| "--also needs an anchor".to_string())?;
            also.push(anchor.to_string());
            i += 1;
        } else if a == "--all" {
            all = true;
        } else {
            params.push(a.to_string());
        }
        i += 1;
    }
    Ok(ExtractOpts {
        count,
        kind,
        all,
        also,
        params,
    })
}

fn run_find(name: &str, dir: &str) -> ExitCode {
    match lisplens::search::find_definitions(std::path::Path::new(dir), name) {
        Ok(hits) => {
            print!("{}", lisplens::search::hits_text(&hits));
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lisplens: {dir}: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_refs(name: &str, dir: &str) -> ExitCode {
    match lisplens::search::find_symbol(std::path::Path::new(dir), name) {
        Ok(occurrences) => {
            print!("{}", lisplens::search::occurrences_text(&occurrences, name));
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lisplens: {dir}: {err}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
usage:
  lisplens struct read <file> [name]   Outline, or expand a definition by name
  lisplens line read <file>     line-hash read ([path#hash] + N:hash|content)
  lisplens line edit <file>     apply a Line-hash patch from stdin
  lisplens struct edit <file>   apply a Structural patch from stdin
  lisplens find <name> [dir]    find definitions by name (default dir: .)
  lisplens refs <name> [dir]    find symbol occurrences (code/data tagged)
  lisplens format [--nameless] [--tonsky] <file>  reindent a Lisp file (native, by dialect; --tonsky = Clojure fixed style)
  lisplens check <file>         parse-check a Lisp file (diagnostics; non-zero on errors)
  lisplens rename <old> <new> <file>   rename a symbol across a file (symbol-exact, safe)
  lisplens inline <name> <file>        inline a function at its call sites (safe subset)
  lisplens docstring <name> <file>     set/replace a function-like def's docstring (text on stdin)
  lisplens rewrite <file>       structural pattern->template rewrite (spec on stdin)
  lisplens extract <file> <anchor> <name> [param...] [--count N] [--kind HEAD] [--all] [--also ANCHOR]  pull a form (or a run of N) into a new function
  lisplens mcp                  run the MCP server over stdio

  --dialect NAME   force the dialect for a single-file command (kebab-case,
                   e.g. islisp / common-lisp / clojure) instead of guessing
                   from the extension — useful for ambiguous ones like .lsp

Patch DSL, examples, and MCP setup: https://github.com/rigortype/lisplens";

/// Explicit help request (`--help`/`-h`/`help`): usage to stdout, success exit.
fn help() -> ExitCode {
    println!("{USAGE}");
    ExitCode::SUCCESS
}

/// Misuse (unknown/absent command): usage to stderr, failure exit.
fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialect_flag_space_form_is_stripped_and_parsed() {
        let mut args = vec![
            "format".to_string(),
            "--dialect".to_string(),
            "islisp".to_string(),
            "x.lsp".to_string(),
        ];
        assert_eq!(
            take_dialect_flag(&mut args).unwrap(),
            Some(lisplens::Dialect::Islisp)
        );
        assert_eq!(args, ["format", "x.lsp"]);
    }

    #[test]
    fn dialect_flag_equals_form_is_stripped_and_parsed() {
        let mut args = vec![
            "check".to_string(),
            "--dialect=common-lisp".to_string(),
            "y.lsp".to_string(),
        ];
        assert_eq!(
            take_dialect_flag(&mut args).unwrap(),
            Some(lisplens::Dialect::CommonLisp)
        );
        assert_eq!(args, ["check", "y.lsp"]);
    }

    #[test]
    fn no_flag_leaves_args_untouched() {
        let mut args = vec!["check".to_string(), "z.el".to_string()];
        assert_eq!(take_dialect_flag(&mut args).unwrap(), None);
        assert_eq!(args, ["check", "z.el"]);
    }

    #[test]
    fn unknown_dialect_is_an_error() {
        let mut args = vec!["--dialect".to_string(), "klingon".to_string()];
        assert!(take_dialect_flag(&mut args).is_err());
    }

    #[test]
    fn value_less_dialect_flag_is_an_error() {
        let mut args = vec!["check".to_string(), "--dialect".to_string()];
        assert!(take_dialect_flag(&mut args).is_err());
    }
}
