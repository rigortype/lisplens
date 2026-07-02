//! lisplens CLI (skeleton).
//!
//! One subcommand so far: `outline <file>`. The mode-first command surface and
//! MCP server described in ADR-0006 are not built yet.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("outline") => match args.next() {
            Some(file) => run_outline(PathBuf::from(file)),
            None => usage(),
        },
        _ => usage(),
    }
}

fn run_outline(path: PathBuf) -> ExitCode {
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("lisplens: {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let options = lisplens::options_for_path(&path);
    for entry in lisplens::outline(&source, &options) {
        let name = entry.name.as_deref().unwrap_or("-");
        println!("{:>5}  {}  {}", entry.line, entry.kind, name);
    }
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    eprintln!("usage: lisplens outline <file>");
    eprintln!();
    eprintln!("Skeleton stage — see CONTEXT.md and docs/adr/ for the full design.");
    ExitCode::FAILURE
}
