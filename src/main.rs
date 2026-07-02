//! lisplens CLI (skeleton).
//!
//! One subcommand so far: `outline <file>`. The mode-first command surface and
//! MCP server described in ADR-0006 are not built yet.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["struct", "read", file] => run_struct_read(PathBuf::from(file)),
        ["struct", "read", file, name] => run_struct_expand(PathBuf::from(file), name),
        ["line", "read", file] => run_line_read(PathBuf::from(file)),
        ["line", "edit", file] => run_line_edit(PathBuf::from(file)),
        ["struct", "edit", file] => run_struct_edit(PathBuf::from(file)),
        ["find", name] => run_find(name, "."),
        ["find", name, dir] => run_find(name, dir),
        ["refs", name] => run_refs(name, "."),
        ["refs", name, dir] => run_refs(name, dir),
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
    print!("{}", lisplens::linehash::read(&path.display().to_string(), &source));
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
    eprintln!("  lisplens mcp                  run the MCP server over stdio");
    eprintln!();
    eprintln!("Skeleton stage — see CONTEXT.md and docs/adr/ for the full design.");
    ExitCode::FAILURE
}
