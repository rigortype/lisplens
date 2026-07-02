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
| `structural` | paredit ops as span→edits: wrap/raise/splice/slurp±/barf±/split/join/rename (ADR-0012) |
| `resolve` | `line:hash[:ordinal]` anchor → node (+parent/index) in the parse tree (ADR-0018) |
| `write` | `verify_and_write`: drift gate + validate-then-write + atomic (perms/symlink-safe) (ADR-0005/0017); `write_atomically` is pub |
| `apply` | end-to-end: read→drift→splice→verify_and_write (`apply_*_to_file`) |
| `patch` | Line-hash + Structural **patch DSL** parse/apply; `Outcome{new_file_hash, warnings}` (ADR-0021) |
| `search` | `find_definitions` / `find_symbol` (code-vs-data via lispexp `walk`) + text renderers (ADR-0010) |
| `config` | resolve `indent-tabs-mode`/`tab-width` from file-local/dir-locals/EditorConfig (ADR-0029) |
| `format` | native Emacs Lisp indenter — see `formatter.md` (ADR-0011/0025-0028) |
| `mcp` | minimal stdio JSON-RPC MCP server (ADR-0020) |
| `lib` | Lens: `outline`/`expand` (+ `_text`), `dialect_for_path`/`recognized_dialect`, `disappeared_definitions`; re-exports `Dialect` |
| `main` | CLI dispatch |

## Surface

CLI (`lisplens …`):
```
struct read <file> [name]   Outline (line hash kind name [signature]); with name, expand inner nodes
line   read <file>          hashline-style line view
struct edit <file>          apply Structural patch from stdin (12 verbs)
line   edit <file>          apply Line-hash patch from stdin (replace/delete/insert±)
find <name> [dir]           definitions by name
refs <name> [dir]           symbol occurrences (code/data tagged)
format <file>               reindent Emacs Lisp (native; honors config)
mcp                         MCP server over stdio
```
MCP tools mirror the CLI: `struct_read`/`line_read`/`struct_edit`/`line_edit`/
`find`/`refs`/`format`. Edit tools take a `patch` string (ADR-0019's JSON
op-array is a future option).

## Patch DSL (ADR-0021)

`@ <file-hash>` header, then one op per line; heredoc payloads `<<TAG … TAG`
(content only — lisplens owns terminators). Anchor = `line:hash[:ordinal]`.
Addressing is **hash-first** (ADR-0018); S-expr structural addresses are
deferred. Structural verbs: replace / delete / wrap / raise / splice /
slurp-fwd / slurp-back / barf-fwd / barf-back / `split @<index>` /
`join <anchor2>` / `rename <from> <to>`. Line-hash verbs: replace / delete /
insert-after / insert-before.

## Safety pipeline (both modes)

drift (strict file-hash, ADR-0017) → splice → validate-then-write (reject edits
that add parse errors, compared by lispexp `ErrorKind` multiset, ADR-0005) →
atomic write (temp + rename, preserves mode, follows symlinks). Success returns
the new file-hash + warnings (disappeared definitions, ADR-0024). Structural
mode should auto-format the touched region (ADR-0025/0028 — not yet wired);
Line-hash stays verbatim (ADR-0027).

## Backend

lispexp gives the parse tree (`Datum` with byte `span` + 1-based `line`),
`LineIndex`, the definition-form annotator (`bundled_registry` + `annotate_tree`,
roles incl. Dispatch signature), the `indent` module (`IndentTable`,
`harvest_indent_specs`), `walk` (code-vs-data), and position-stable `ErrorKind`.
What lisplens uses from it, and outstanding asks, are in
`docs/lispexp-integration.md` and `docs/lispexp-feedback/`.
