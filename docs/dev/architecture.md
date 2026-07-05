# Architecture

How the crate is laid out and how an edit flows. Decisions live in `docs/adr/`;
domain terms in `CONTEXT.md`. This file is the durable map — it changes slowly.

lisplens is a CLI + MCP tool for token-efficient, polyglot Lisp editing by AI
agents, built on the **lispexp** reader (crates.io `lispexp` 0.2.x; local
checkout at `../lispexp`). Agents read a file's shape cheaply, get a
content-hash **anchor** for any target, and edit by anchor.

## Modules (`src/`)

| module | responsibility |
| --- | --- |
| `hash` | xxh3-64 anchor (4-hex) + file (16-hex) hashes (ADR-0008) |
| `linehash` | Line-hash read: `[path#FILEHASH]` + `N:hash|content`, via lispexp `LineIndex` |
| `edit` | `splice` (non-overlapping byte-range replace) + `LineOp`/`apply_line_ops` (ADR-0006) |
| `structural` | paredit ops as span→edits: wrap/raise/splice/slurp±/barf±/split/join/rename/format (ADR-0012) |
| `resolve` | `line:hash[:ordinal]` anchor → node (+parent/index) in the parse tree (ADR-0018) |
| `write` | `verify_and_write`: drift gate + validate-then-write + atomic (perms/symlink-safe) (ADR-0005/0017); `write_atomically` is pub |
| `apply` | end-to-end: read→drift→splice→verify_and_write (`apply_*_to_file`) |
| `patch` | Line-hash + Structural **patch DSL** parse/apply; `Outcome{new_file_hash, warnings}` (ADR-0021) |
| `refactor` | Semantic refactoring procedures (ADR-0032): `rename_symbol_in_file` (symbol-exact), `inline_definition_in_file` (expand a function at its call sites), `rewrite_in_file` (structural pattern→template "sed", ADR-0033), and `extract_into_function` (pull a form into a new function, ADR-0034 — pure cut+wrap, user-supplied params) over `structural` + the safety pipeline |
| `search` | `find_definitions` / `find_symbol` (code-vs-data via lispexp `walk`) + text renderers (ADR-0010) |
| `sexpr` | shared structural-comparison primitives — `struct_eq` (Structural equality, modulo formatting, ADR-0033) + `opt_eq`; the basis of `refactor`'s matching and `diff` |
| `diff` | Structural diff. `diff_files` compares two versions by top-level definition (ADR-0047: added/removed/changed via `struct_eq`, keyed `(kind, name, dispatch?)`); `diff_forms`/`diff_files_deep`/`diff_source_forms` are the intra-form tree diff (ADR-0048: struct_eq-LCS child alignment + positional gap pairing, category-gated recursion, four statuses; deep mode also renders added/removed defs as their `expand` Lens, #44). `*_text`/`*_json` + `deep_*` renderers |
| `config` | resolve `indent-tabs-mode`/`tab-width` from file-local/dir-locals/EditorConfig (ADR-0029); file-local + dir-local *parsing* delegated to `lispexp_emacs::{local_vars,dir_locals}` (lispexp ADR-0033), EditorConfig stays in-tree |
| `format` | native Lisp indenter, dialect-dispatched: shared driver + an `Engine` per bundled Emacs indenter (Emacs Lisp `lisp-indent-function`; Common Lisp `common-lisp-indent-function` in `format/commonlisp.rs`; Scheme family `scheme-indent-function` in `format/scheme.rs`; generic fallback for the rest) — see `formatter.md`; Emacs Lisp bundled table from `lispexp_emacs::indent::bundled_table` (ADR-0011/0025-0028/**0031**; crate split lispexp ADR-0033) |
| `parinfer` | native parinfer-style whole-buffer transform (ADR-0045): `run(Request)->Answer`, stateless, stdin→stdout. Paren mode = balance-checked faithful reindent (reuses `format`); Indent mode = infer close-parens from indentation over a tolerant `lex()` token scan (own `col_at`/`display_col` columns; Nameless-aware via `Nameless::saving`, ADR-0030; minimal cursor-line trail protection, #31). balance-*generating* safety. `run_json`/`run_json_line` drive the MCP tool and the persistent `parinfer --server` (line-delimited JSON, ADR-0046) |
| `mcp` | minimal stdio JSON-RPC MCP server (ADR-0020) |
| `lib` | Lens: `outline`/`expand` (+ `_text`), `dialect_for_path`/`recognized_dialect`, `disappeared_definitions`; re-exports `Dialect` |
| `main` | CLI dispatch |

## Surface

CLI (`lisplens …`):
```
struct read <file> [name]   Outline (line hash kind name [signature]); with name, expand inner nodes
line   read <file>          hashline-style line view
struct edit <file>          apply Structural patch from stdin (13 verbs)
line   edit <file>          apply Line-hash patch from stdin (replace/delete/insert±)
find <name> [dir]           definitions by name
refs <name> [dir]           symbol occurrences (code/data tagged)
format <file>               reindent a Lisp file (native, by dialect; honors config)
parinfer <mode>             parinfer-style transform, stdin→stdout (paren/indent; ADR-0045)
parinfer --server           persistent line-delimited JSON server for editors (ADR-0046)
check  <file>               parse-check (diagnostics; non-zero exit on errors, ADR-0032)
diff <old> <new> [--json] [--deep|--unit NAME]   structural diff: definition map (ADR-0047), or drill a changed def (ADR-0048); exit 0 regardless of differences
rename <old> <new> <file>   rename a symbol across a file (symbol-exact, safe, ADR-0032)
inline <name> <file>        inline a function at its call sites (safe subset, ADR-0032)
rewrite <file>              structural pattern→template rewrite, spec on stdin (ADR-0033)
extract <file> <anchor> <name> [param...]   pull a form into a new function (ADR-0034)
mcp                         MCP server over stdio
```
MCP tools mirror the CLI: `struct_read`/`line_read`/`struct_edit`/`line_edit`/
`find`/`refs`/`format`/`parinfer`/`check`/`diff`/`rename`/`inline`/`rewrite`/`extract`. Edit tools
take a `patch`/`spec` string (ADR-0019's JSON op-array is a future option).

## Patch DSL (ADR-0021)

`@ <file-hash>` header, then one op per line; heredoc payloads `<<TAG … TAG`
(content only — lisplens owns terminators). Anchor = `line:hash[:ordinal]`.
Addressing is **hash-first** (ADR-0018); S-expr structural addresses are
deferred. Structural verbs: replace / delete / wrap / raise / splice /
slurp-fwd / slurp-back / barf-fwd / barf-back / `split @<index>` /
`join <anchor2>` / `rename <from> <to>` / `format` (reindent the anchored form
in place, ADR-0028). Line-hash verbs: replace / delete / insert-after /
insert-before.

## Safety pipeline (both modes)

drift (strict file-hash, ADR-0017) → splice → **auto-format the touched region**
(Structural + dialects with a faithful native engine — Emacs Lisp, Common Lisp,
and the Scheme family; `format::has_native_engine`, ADR-0031) → validate-then-write (reject edits that add parse
errors, compared by lispexp `ErrorKind` multiset, ADR-0005) → atomic write (temp
+ rename, preserves mode, follows symlinks). Success returns the new file-hash
(of the *formatted* content) + warnings (disappeared definitions, ADR-0024).
Structural mode reindents the top-level forms the edits fell within via
`format::reindent_range` (ADR-0025/0028), using the post-splice edit spans from
`edit::splice_tracked`; Line-hash stays verbatim (ADR-0027).

## Backend

lispexp gives the parse tree (`Datum` with byte `span` + 1-based `line`),
`LineIndex`, the definition-form annotator (`bundled_registry` + `annotate_tree`,
roles incl. Dispatch signature), the `indent` module (`IndentTable`,
`harvest_indent_specs`), `walk` (code-vs-data), and position-stable `ErrorKind`.
What lisplens uses from it, and outstanding asks, are in
`docs/lispexp-integration.md` and `docs/lispexp-feedback/`.
