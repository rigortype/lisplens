//! A minimal MCP server over stdio (ADR-0020), hand-rolled on newline-delimited
//! JSON-RPC 2.0 with `serde_json` — no async runtime.
//!
//! Exposes the same surface as the CLI as tools: `struct_read`, `line_read`,
//! `struct_edit`, `line_edit`, `find`, `refs`. Tool failures are returned as a
//! result with `isError: true` (per MCP), not a JSON-RPC error.

use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{json, Value};

use crate::patch::{apply_line_patch, apply_struct_patch, parse_line_patch, parse_struct_patch};
use crate::search::{find_definitions, find_symbol, hits_text, occurrences_text};
use crate::{dialect_for_path, expand_text, linehash, outline_text};

/// Run the MCP server, reading requests from stdin and writing responses to
/// stdout until EOF.
pub fn serve() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // Only requests (with an id) get a response; notifications are ignored.
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let response = handle(
            msg.get("method").and_then(Value::as_str).unwrap_or(""),
            &msg,
            id,
        );
        writeln!(stdout, "{response}")?;
        stdout.flush()?;
    }
    Ok(())
}

fn handle(method: &str, msg: &Value, id: Value) -> Value {
    match method {
        "initialize" => ok(id, initialize_result(msg)),
        "tools/list" => ok(id, json!({ "tools": tools() })),
        "tools/call" => call_tool(id, msg.get("params").unwrap_or(&Value::Null)),
        "ping" => ok(id, json!({})),
        other => error(id, -32601, &format!("method not found: {other}")),
    }
}

fn initialize_result(msg: &Value) -> Value {
    // Echo the client's protocol version when given, else a known one.
    let version = msg
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("2024-11-05");
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "lisplens", "version": env!("CARGO_PKG_VERSION") }
    })
}

fn tools() -> Value {
    let file = json!({ "type": "string", "description": "path to the file" });
    let patch = json!({ "type": "string", "description": "a patch (stdin DSL) to apply" });
    let name = json!({ "type": "string" });
    let dir = json!({ "type": "string", "description": "directory to search (default: .)" });
    json!([
        tool(
            "struct_read",
            "Structural Outline; with `name`, expand that definition",
            json!({ "file": file, "name": name }),
            &["file"]
        ),
        tool(
            "line_read",
            "Line-hash view ([path#hash] + N:hash|content)",
            json!({ "file": file }),
            &["file"]
        ),
        tool(
            "struct_edit",
            "Apply a Structural patch",
            json!({ "file": file, "patch": patch }),
            &["file", "patch"]
        ),
        tool(
            "line_edit",
            "Apply a Line-hash patch",
            json!({ "file": file, "patch": patch }),
            &["file", "patch"]
        ),
        tool(
            "find",
            "Find definitions by name across a project",
            json!({ "name": name, "dir": dir }),
            &["name"]
        ),
        tool(
            "refs",
            "Find symbol occurrences (code/data tagged)",
            json!({ "name": name, "dir": dir }),
            &["name"]
        ),
        tool(
            "format",
            "Reindent an Emacs Lisp file in place",
            json!({ "file": file }),
            &["file"]
        ),
    ])
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
        }
    })
}

fn call_tool(id: Value, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    match run_tool(name, &args) {
        Ok(text) => tool_result(id, text, false),
        Err(text) => tool_result(id, text, true),
    }
}

/// Dispatch a tool call, returning either its text output or an error message.
fn run_tool(name: &str, args: &Value) -> Result<String, String> {
    match name {
        "struct_read" => {
            let file = arg(args, "file")?;
            let source = read(file)?;
            let dialect = dialect_for_path(Path::new(file));
            Ok(match args.get("name").and_then(Value::as_str) {
                Some(def) => expand_text(&source, dialect, def),
                None => outline_text(&source, dialect),
            })
        }
        "line_read" => {
            let file = arg(args, "file")?;
            let source = read(file)?;
            Ok(linehash::read(file, &source))
        }
        "line_edit" => {
            let file = arg(args, "file")?;
            let text = arg(args, "patch")?;
            let patch = parse_line_patch(text).map_err(|e| format!("patch parse error: {e:?}"))?;
            let outcome =
                apply_line_patch(Path::new(file), &patch, dialect_for_path(Path::new(file)))
                    .map_err(|e| format!("{e:?}"))?;
            Ok(edit_text(&outcome))
        }
        "struct_edit" => {
            let file = arg(args, "file")?;
            let text = arg(args, "patch")?;
            let patch =
                parse_struct_patch(text).map_err(|e| format!("patch parse error: {e:?}"))?;
            let outcome =
                apply_struct_patch(Path::new(file), &patch, dialect_for_path(Path::new(file)))
                    .map_err(|e| format!("{e:?}"))?;
            Ok(edit_text(&outcome))
        }
        "format" => {
            let file = arg(args, "file")?;
            if dialect_for_path(Path::new(file)) != crate::Dialect::EmacsLisp {
                return Err("format currently supports Emacs Lisp (.el) only".to_string());
            }
            let source = read(file)?;
            let config = crate::config::resolve(Path::new(file), &source);
            let formatted = crate::format::format_elisp(&source, &config);
            if formatted != source {
                crate::write::write_atomically(Path::new(file), &formatted)
                    .map_err(|e| format!("{file}: {e}"))?;
            }
            Ok("ok".to_string())
        }
        "find" => {
            let name = arg(args, "name")?;
            let dir = args.get("dir").and_then(Value::as_str).unwrap_or(".");
            let hits = find_definitions(Path::new(dir), name).map_err(|e| e.to_string())?;
            Ok(hits_text(&hits))
        }
        "refs" => {
            let name = arg(args, "name")?;
            let dir = args.get("dir").and_then(Value::as_str).unwrap_or(".");
            let occ = find_symbol(Path::new(dir), name).map_err(|e| e.to_string())?;
            Ok(occurrences_text(&occ, name))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Render an edit outcome: `ok <hash>` plus any warnings on following lines.
fn edit_text(outcome: &crate::patch::Outcome) -> String {
    let mut text = format!("ok {}", outcome.new_file_hash);
    for warning in &outcome.warnings {
        text.push_str(&format!("\nwarning: {warning}"));
    }
    text
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string argument `{key}`"))
}

fn read(file: &str) -> Result<String, String> {
    std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tool_result(id: Value, text: String, is_error: bool) -> Value {
    ok(
        id,
        json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_reports_server_info_and_tools() {
        let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" } });
        let resp = handle("initialize", &init, json!(1));
        assert_eq!(resp["result"]["serverInfo"]["name"], "lisplens");
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");

        let list = handle("tools/list", &json!({}), json!(2));
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"struct_read"));
        assert!(names.contains(&"struct_edit"));
        assert!(names.contains(&"refs"));
    }

    #[test]
    fn struct_read_tool_returns_the_outline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.scm");
        std::fs::write(&path, "(define x 1)\n").unwrap();
        let text = run_tool("struct_read", &json!({ "file": path.to_str().unwrap() })).unwrap();
        assert!(text.contains("define"));
        assert!(text.contains(" x"));
    }

    #[test]
    fn struct_edit_tool_applies_a_patch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.scm");
        std::fs::write(&path, "(define x 1)\n").unwrap();
        let fh = crate::hash::file_hash("(define x 1)\n".as_bytes());
        let h = crate::hash::anchor_hash("(define x 1)".as_bytes());
        let patch = format!("@ {fh}\nreplace 1:{h} <<END\n(define x 2)\nEND\n");
        let text = run_tool(
            "struct_edit",
            &json!({ "file": path.to_str().unwrap(), "patch": patch }),
        )
        .unwrap();
        assert!(text.starts_with("ok "));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(define x 2)\n");
    }

    #[test]
    fn a_missing_argument_is_a_tool_error() {
        let resp = call_tool(json!(9), &json!({ "name": "line_read", "arguments": {} }));
        assert_eq!(resp["result"]["isError"], true);
    }
}
