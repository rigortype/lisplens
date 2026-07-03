//! lisplens CLI (skeleton).
//!
//! One subcommand so far: `outline <file>`. The mode-first command surface and
//! MCP server described in ADR-0006 are not built yet.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
        ["format", file] => run_format(PathBuf::from(file), false),
        ["format", "--nameless", file] => run_format(PathBuf::from(file), true),
        ["check", file] => run_check(PathBuf::from(file)),
        ["rename", from, to, file] => run_rename(from, to, PathBuf::from(file)),
        ["inline", name, file] => run_inline(name, PathBuf::from(file)),
        ["mcp"] => match lisplens::mcp::serve() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("lisplens: mcp: {err}");
                ExitCode::FAILURE
            }
        },
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
    let dialect = lisplens::dialect_for_path(&path);
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
    let dialect = lisplens::dialect_for_path(&path);
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
    let dialect = lisplens::dialect_for_path(&path);
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
    let dialect = lisplens::dialect_for_path(&path);
    report(lisplens::patch::apply_struct_patch(&path, &patch, dialect))
}

fn run_format(path: PathBuf, nameless: bool) -> ExitCode {
    let dialect = lisplens::dialect_for_path(&path);
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let config = lisplens::config::resolve(&path, &source);
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
    let dialect = lisplens::dialect_for_path(&path);
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
    let dialect = lisplens::dialect_for_path(&path);
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
    let dialect = lisplens::dialect_for_path(&path);
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

fn usage() -> ExitCode {
    eprintln!("usage:");
    eprintln!("  lisplens struct read <file> [name]   Outline, or expand a definition by name");
    eprintln!("  lisplens line read <file>     line-hash read ([path#hash] + N:hash|content)");
    eprintln!("  lisplens line edit <file>     apply a Line-hash patch from stdin");
    eprintln!("  lisplens struct edit <file>   apply a Structural patch from stdin");
    eprintln!("  lisplens find <name> [dir]    find definitions by name (default dir: .)");
    eprintln!("  lisplens refs <name> [dir]    find symbol occurrences (code/data tagged)");
    eprintln!("  lisplens format [--nameless] <file>  reindent a Lisp file (native, by dialect)");
    eprintln!("  lisplens check <file>         parse-check a Lisp file (diagnostics; non-zero on errors)");
    eprintln!("  lisplens rename <old> <new> <file>   rename a symbol across a file (symbol-exact, safe)");
    eprintln!("  lisplens inline <name> <file>        inline a function at its call sites (safe subset)");
    eprintln!("  lisplens mcp                  run the MCP server over stdio");
    eprintln!();
    eprintln!("Skeleton stage — see CONTEXT.md and docs/adr/ for the full design.");
    ExitCode::FAILURE
}
