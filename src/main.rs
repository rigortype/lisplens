//! lisplens CLI (skeleton).
//!
//! One subcommand so far: `outline <file>`. The mode-first command surface and
//! MCP server described in ADR-0006 are not built yet.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mode = args.next();
    let verb = args.next();
    let file = args.next();
    match (mode.as_deref(), verb.as_deref(), file) {
        (Some("struct"), Some("read"), Some(file)) => run_struct_read(PathBuf::from(file)),
        (Some("line"), Some("read"), Some(file)) => run_line_read(PathBuf::from(file)),
        (Some("line" | "struct"), Some("edit"), _) => {
            eprintln!("lisplens: `edit` is not implemented yet");
            ExitCode::FAILURE
        }
        _ => usage(),
    }
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
    for entry in lisplens::outline(&source, dialect) {
        let name = entry.name.as_deref().unwrap_or("-");
        // ADR-0013 Outline columns: line, hash, kind, name (name last). Nesting
        // is shown by indenting the name column (name can contain spaces, so it
        // must stay last).
        let indent = "  ".repeat(entry.depth as usize);
        println!("{:>5}  {}  {}  {indent}{name}", entry.line, entry.hash, entry.kind);
    }
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    eprintln!("usage:");
    eprintln!("  lisplens struct read <file>   structural Outline (line hash kind name)");
    eprintln!("  lisplens line read <file>     line-hash read ([path#hash] + N:hash|content)");
    eprintln!("  lisplens {{line|struct}} edit    (not implemented yet)");
    eprintln!();
    eprintln!("Skeleton stage — see CONTEXT.md and docs/adr/ for the full design.");
    ExitCode::FAILURE
}
