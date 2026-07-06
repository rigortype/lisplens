//! A minimal MCP server over stdio (ADR-0020), hand-rolled on newline-delimited
//! JSON-RPC 2.0 with `serde_json` — no async runtime.
//!
//! Exposes the same surface as the CLI as tools: reads (`struct_read`,
//! `line_read`), edits (`struct_edit`, `line_edit`), queries (`find`, `refs`),
//! `format`/`check`, and the refactoring procedures (`rename`, `inline`,
//! `docstring`, `rewrite`, `extract`). Tool failures are returned as a result
//! with `isError: true` (per MCP), not a JSON-RPC error.

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
    let from = json!({ "type": "string", "description": "the symbol to rename" });
    let to = json!({ "type": "string", "description": "the new symbol name" });
    let rspec = json!({ "type": "string", "description": "rewrite spec: pattern <<TAG … TAG / template <<TAG … TAG (ADR-0033)" });
    let docstring_text = json!({ "type": "string", "description": "the docstring text (raw; escaped into a string literal)" });
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
            "Reindent a Lisp file in place (native, by dialect)",
            json!({ "file": file }),
            &["file"]
        ),
        tool(
            "check",
            "Parse-check a Lisp file; report diagnostics (empty = clean)",
            json!({ "file": file }),
            &["file"]
        ),
        tool(
            "diff",
            "Structural diff of two Lisp versions, modulo formatting (whitespace/comment churn is never a change). File mode (`old`+`new` paths): the definition-level attention map (ADR-0047) — which definitions were added/removed/changed. Add `deep` to drill every changed definition's internals, or `unit` to drill one by name (ADR-0048). Form mode (`oldForm`+`newForm` snippet strings, `dialect` optional): the intra-form tree diff of the two forms directly. `json` returns structured output instead of text",
            json!({
                "old": json!({ "type": "string", "description": "path to the old/base version (file mode)" }),
                "new": json!({ "type": "string", "description": "path to the new version (file mode)" }),
                "deep": json!({ "type": "boolean", "description": "drill each changed definition's internals (ADR-0048)" }),
                "unit": json!({ "type": "string", "description": "drill only the definition(s) with this name (implies deep)" }),
                "oldForm": json!({ "type": "string", "description": "old form snippet (form mode; with newForm)" }),
                "newForm": json!({ "type": "string", "description": "new form snippet (form mode; with oldForm)" }),
                "dialect": json!({ "type": "string", "description": "dialect for form mode, kebab-case (default emacs-lisp)" }),
                "json": json!({ "type": "boolean", "description": "return the structured JSON diff instead of text (default false)" }),
                "html": json!({ "type": "boolean", "description": "return a self-contained HTML page visualizing the diff, for a human to open (default false; mutually exclusive with json)" })
            }),
            &[]
        ),
        tool(
            "parinfer",
            "Parinfer-style transform of Lisp *text* (not a file). paren = require balance then reindent faithfully (Emacs-native, Nameless-aware); indent = indentation is authoritative, infer close-parens from it. Returns a JSON answer {text, success, error, cursorX, cursorLine}; on failure text is the unchanged input",
            json!({
                "mode": json!({ "type": "string", "enum": ["paren", "indent"], "description": "paren = balance-checked faithful reindent; indent = infer close-parens from indentation" }),
                "text": json!({ "type": "string", "description": "the buffer text to transform" }),
                "dialect": json!({ "type": "string", "description": "dialect, kebab-case (default emacs-lisp)" }),
                "nameless": json!({ "type": "boolean", "description": "enable the Nameless overlay (Emacs Lisp only)" }),
                "name": json!({ "type": "string", "description": "filename hint for Nameless current-name discovery" }),
                "cursorLine": json!({ "type": "integer", "description": "0-based cursor line (optional)" }),
                "cursorX": json!({ "type": "integer", "description": "0-based cursor column (optional)" })
            }),
            &["mode", "text"]
        ),
        tool(
            "rename",
            "Rename a symbol across a file (symbol-exact, safe)",
            json!({ "file": file, "from": from, "to": to }),
            &["file", "from", "to"]
        ),
        tool(
            "inline",
            "Inline a function at its call sites (safe subset)",
            json!({ "file": file, "name": name }),
            &["file", "name"]
        ),
        tool(
            "docstring",
            "Set or replace a function-like definition's docstring (defun/defsubst/defmacro/cl-*/Scheme define); text is raw and escaped into a string literal",
            json!({ "file": file, "name": name, "text": docstring_text }),
            &["file", "name", "text"]
        ),
        tool(
            "rewrite",
            "Structural pattern->template rewrite (structural sed; not behaviour-preserving)",
            json!({ "file": file, "spec": rspec }),
            &["file", "spec"]
        ),
        tool(
            "extract",
            "Pull the form at `anchor` (or the run of `count` siblings from it) into a new function `name` with `params`. `kind` overrides the defining operator (e.g. defsubst, cl-defun, defn-); the dialect's default (defun/define/defn) is used when omitted. With `all` true, every occurrence structurally equal to the selection is replaced by a call to the one new function. With `also` (a list of extra anchors), the anchored form and those sites are anti-unified: their common skeleton becomes the body and the positions that differ become inferred parameters, each site calling with its own sub-terms",
            json!({
                "file": file,
                "anchor": json!({ "type": "string", "description": "line:hash[:ordinal] of the form" }),
                "name": name,
                "params": json!({ "type": "array", "items": { "type": "string" }, "description": "parameter symbols (default none); with `also`, names the inferred params (must match the inferred count)" }),
                "count": json!({ "type": "integer", "minimum": 1, "description": "number of contiguous sibling forms to extract (default 1)" }),
                "kind": json!({ "type": "string", "description": "defining operator head (default: dialect's defun/define/defn)" }),
                "all": json!({ "type": "boolean", "description": "extract every structurally-equal occurrence, not just the anchored one (default false)" }),
                "also": json!({ "type": "array", "items": { "type": "string" }, "description": "extra site anchors to anti-unify with the anchored form (generalizing extraction); differing sub-terms become parameters" })
            }),
            &["file", "anchor", "name"]
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
            let dialect = dialect_for_path(Path::new(file));
            let source = read(file)?;
            let config = crate::config::resolve(Path::new(file), &source);
            let formatted = crate::format::format(&source, &config, dialect);
            if formatted != source {
                crate::write::write_atomically(Path::new(file), &formatted)
                    .map_err(|e| format!("{file}: {e}"))?;
            }
            Ok("ok".to_string())
        }
        "check" => {
            let file = arg(args, "file")?;
            let dialect = dialect_for_path(Path::new(file));
            let source = read(file)?;
            let diagnostics = crate::check(&source, dialect);
            Ok(if diagnostics.is_empty() {
                "ok".to_string()
            } else {
                crate::diagnostics_text(file, &diagnostics)
            })
        }
        // Shares the request shape and engine with `parinfer --server` via
        // `run_json` — `{mode, text, dialect?, nameless?, name?, cursor*}` in, the
        // `{text, success, error, cursor*}` answer out (bad input → success:false).
        "parinfer" => Ok(crate::parinfer::run_json(args).to_string()),
        "diff" => {
            let json = args.get("json").and_then(Value::as_bool).unwrap_or(false);
            let html = args.get("html").and_then(Value::as_bool).unwrap_or(false);
            // Form-string mode: compare two form snippets directly (ADR-0048's
            // general two-form primitive), no files.
            if let (Some(of), Some(nf)) = (
                args.get("oldForm").and_then(Value::as_str),
                args.get("newForm").and_then(Value::as_str),
            ) {
                let dialect = args
                    .get("dialect")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(crate::Dialect::EmacsLisp);
                return Ok(match crate::diff::diff_source_forms(of, nf, dialect) {
                    None => String::new(),
                    Some(d) if html => crate::diff::form_diff_html(&d),
                    Some(d) if json => crate::diff::form_diff_json(&d).to_string(),
                    Some(d) => crate::diff::form_diff_text(&d),
                });
            }
            let old = arg(args, "old")?;
            let new = arg(args, "new")?;
            let old_src = read(old)?;
            let new_src = read(new)?;
            let dialect = dialect_for_path(Path::new(new));
            let unit = args.get("unit").and_then(Value::as_str);
            let deep = unit.is_some() || args.get("deep").and_then(Value::as_bool).unwrap_or(false);
            Ok(if deep {
                let d = crate::diff::diff_files_deep(&old_src, &new_src, dialect, unit);
                if html {
                    crate::diff::deep_html(&d)
                } else if json {
                    crate::diff::deep_json(&d).to_string()
                } else {
                    crate::diff::deep_text(&d)
                }
            } else {
                let d = crate::diff::diff_files(&old_src, &new_src, dialect);
                if html {
                    crate::diff::diff_html(&d)
                } else if json {
                    crate::diff::diff_json(&d).to_string()
                } else {
                    crate::diff::diff_text(&d)
                }
            })
        }
        "rename" => {
            let file = arg(args, "file")?;
            let from = arg(args, "from")?;
            let to = arg(args, "to")?;
            let dialect = dialect_for_path(Path::new(file));
            let outcome =
                crate::refactor::rename_symbol_in_file(Path::new(file), from, to, dialect)
                    .map_err(|e| e.to_string())?;
            Ok(format!(
                "renamed {} occurrence(s): {from} -> {to}  {}",
                outcome.renamed, outcome.new_file_hash
            ))
        }
        "inline" => {
            let file = arg(args, "file")?;
            let name = arg(args, "name")?;
            let dialect = dialect_for_path(Path::new(file));
            let outcome =
                crate::refactor::inline_definition_in_file(Path::new(file), name, dialect)
                    .map_err(|e| e.to_string())?;
            Ok(format!(
                "inlined {} call site(s): {name}  {}",
                outcome.inlined, outcome.new_file_hash
            ))
        }
        "docstring" => {
            let file = arg(args, "file")?;
            let name = arg(args, "name")?;
            let text = arg(args, "text")?;
            let dialect = dialect_for_path(Path::new(file));
            let outcome =
                crate::refactor::set_docstring_in_file(Path::new(file), name, text, dialect)
                    .map_err(|e| e.to_string())?;
            let verb = match outcome.action {
                crate::refactor::DocstringAction::Inserted => "set",
                crate::refactor::DocstringAction::Replaced => "replaced",
            };
            Ok(format!(
                "{verb} docstring on {name}  {}",
                outcome.new_file_hash
            ))
        }
        "rewrite" => {
            let file = arg(args, "file")?;
            let spec = arg(args, "spec")?;
            let dialect = dialect_for_path(Path::new(file));
            let outcome = crate::refactor::rewrite_in_file(Path::new(file), spec, dialect)
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "rewrote {} site(s)  {}",
                outcome.rewritten, outcome.new_file_hash
            ))
        }
        "extract" => {
            let file = arg(args, "file")?;
            let anchor = arg(args, "anchor")?;
            let name = arg(args, "name")?;
            let params: Vec<String> = args
                .get("params")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let count = args
                .get("count")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(1);
            let kind = args.get("kind").and_then(Value::as_str);
            let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
            let also: Vec<String> = args
                .get("also")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let dialect = dialect_for_path(Path::new(file));
            // `also` (generalizing multi-anchor) is a distinct site-selection mode.
            let outcome = if !also.is_empty() {
                if all {
                    return Err("`also` cannot be combined with `all`".into());
                }
                if count != 1 {
                    return Err("`also` cannot be combined with `count`".into());
                }
                crate::refactor::extract_generalized(
                    Path::new(file),
                    anchor,
                    &also,
                    name,
                    &params,
                    kind,
                    dialect,
                )
            } else {
                let extract = if all {
                    crate::refactor::extract_multi_site
                } else {
                    crate::refactor::extract_block_into_function
                };
                extract(Path::new(file), anchor, name, &params, count, kind, dialect)
            }
            .map_err(|e| e.to_string())?;
            Ok(format!(
                "extracted `{name}` at {} site(s)  {}",
                outcome.sites, outcome.new_file_hash
            ))
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
        assert!(names.contains(&"check"));
        assert!(names.contains(&"rewrite"));
        assert!(names.contains(&"extract"));
        assert!(names.contains(&"parinfer"));
    }

    #[test]
    fn parinfer_tool_reindents_text_and_returns_json_answer() {
        let out = run_tool(
            "parinfer",
            &json!({ "mode": "paren", "text": "(defun f (x)\n(+ x\n1))\n" }),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["success"], json!(true));
        assert_eq!(v["error"], Value::Null);
        assert_eq!(v["text"], json!("(defun f (x)\n  (+ x\n     1))\n"));
    }

    #[test]
    fn parinfer_tool_refuses_imbalance_unchanged() {
        let out = run_tool("parinfer", &json!({ "mode": "paren", "text": "(a\n" })).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["success"], json!(false));
        assert_eq!(v["text"], json!("(a\n"), "input returned unchanged");
        assert_eq!(v["error"]["name"], json!("unclosed-paren"));
    }

    #[test]
    fn parinfer_tool_indent_mode_infers_closers() {
        let out = run_tool(
            "parinfer",
            &json!({ "mode": "indent", "text": "(defun f (x)\n  (+ x\n     1" }),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["success"], json!(true));
        assert_eq!(v["text"], json!("(defun f (x)\n  (+ x\n     1))"));
    }

    #[test]
    fn extract_tool_pulls_a_form_into_a_function() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.el");
        std::fs::write(&path, "(defun foo (x)\n  (* x 2))\n").unwrap();
        let anchor = format!("2:{}", crate::hash::anchor_hash("(* x 2)".as_bytes()));
        let text = run_tool(
            "extract",
            &json!({ "file": path.to_str().unwrap(), "anchor": anchor, "name": "dbl", "params": ["x"] }),
        )
        .unwrap();
        assert!(text.starts_with("extracted `dbl`"), "{text}");
        let r = std::fs::read_to_string(&path).unwrap();
        assert!(r.starts_with("(defun dbl (x) (* x 2))\n\n"), "{r}");
        assert!(r.contains("  (dbl x))"), "{r}");
    }

    #[test]
    fn rewrite_tool_applies_a_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.el");
        std::fs::write(&path, "(when flag (foo))\n").unwrap();
        let spec = "pattern <<P\n(when $c $b)\nP\ntemplate <<T\n$b\nT\n";
        let text = run_tool(
            "rewrite",
            &json!({ "file": path.to_str().unwrap(), "spec": spec }),
        )
        .unwrap();
        assert!(text.starts_with("rewrote 1 site"), "{text}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "(foo)\n");
    }

    #[test]
    fn check_tool_reports_clean_and_broken() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("ok.el");
        std::fs::write(&ok, "(defun f (x)\n  (+ x 1))\n").unwrap();
        assert_eq!(
            run_tool("check", &json!({ "file": ok.to_str().unwrap() })).unwrap(),
            "ok"
        );
        let bad = dir.path().join("bad.el");
        std::fs::write(&bad, "(defun f (x\n").unwrap();
        let text = run_tool("check", &json!({ "file": bad.to_str().unwrap() })).unwrap();
        assert!(text.contains("bad.el:1: "), "{text:?}");
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
