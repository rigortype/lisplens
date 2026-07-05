# lisplens reference: patch DSL, format, dialects, MCP

Read this when you need a structural verb beyond replace/delete/insert, the
exact grammar, or want to run lisplens as an MCP server.

## Patch grammar

```
@ <file-hash>                     # snapshot assertion; drift → whole patch rejected
<verb> <anchor> [args] [<<TAG]
  ...payload lines...
TAG
```

- **Header** `@ <file-hash>`: the 16-hex file-hash from `line read`'s
  `[path#hash]` header, or the `ok <hash>` printed by the previous edit.
- **Anchor** `line:hash` (from `struct read` / `line read`); on a same-line hash
  collision, `line:hash:ordinal` with a 1-based ordinal.
- **Payload**: for text-carrying verbs, end the op line with `<<TAG`; the
  payload runs until a line exactly equal to `TAG`. Choose a tag absent from the
  payload. No-payload verbs are a single line.
- **Success** prints `ok <new-file-hash>`; **failure** prints the reason
  (`Drift {expected, actual}`, a parse error, etc.) and writes nothing.

Multiple ops can appear in one patch, but they all address the **same pre-edit
snapshot** under the one `@` header. If an edit's anchor would depend on an
earlier op in the same patch, split it: apply, take the new hash, build the next
patch.

## Verbs

### Shared — `line edit` and `struct edit`
| verb | payload | effect |
| --- | --- | --- |
| `replace <anchor> <<TAG…` | yes | replace the form/line with the payload |
| `delete <anchor>` | no | remove the form/line |
| `insert-after <anchor> <<TAG…` | yes | insert payload after the anchored form/line |
| `insert-before <anchor> <<TAG…` | yes | insert payload before it |

### Structural-only — `struct edit`
| verb | payload | effect |
| --- | --- | --- |
| `wrap <anchor> <<TAG…` | yes | wrap the form in an enclosing prefix (payload = the wrapper, e.g. `(when cond …)`) |
| `raise <anchor>` | no | replace the parent form with this form |
| `splice <anchor>` | no | remove the anchored form's own delimiters, splicing its children into the parent |
| `slurp-fwd <anchor>` | no | pull the next sibling into the form |
| `slurp-back <anchor>` | no | pull the previous sibling in |
| `barf-fwd <anchor>` | no | push the last child out to the right |
| `barf-back <anchor>` | no | push the first child out to the left |
| `split <anchor> @<index>` | no | split the form at child index `<index>` |
| `join <anchor> <anchor2>` | no | join two adjacent forms |
| `rename <anchor> <from> <to>` | no | rename occurrences of `<from>` to `<to>` within the anchored subtree |
| `format <anchor>` | no | reindent the anchored form in place, no other change (Emacs Lisp) |

These are the paredit operations you'd otherwise do by careful hand-editing;
here they're single, parse-checked ops. `struct edit` on Emacs Lisp also
reindents the touched top-level forms after any edit.

## `lisplens format`

```
lisplens format <file>              # reindent an Emacs Lisp file in place (native indenter)
lisplens format --nameless <file>   # reindent without consulting name-based indent specs
```
Emacs Lisp only — it honors the file's `indent-tabs-mode`/`tab-width` config.
Other dialects parse and edit fine, but don't run `format` on them.

## Dialect coverage

`struct read` / `line read` / `find` / `refs` / structural + line edits work
across the Lisp family lispexp parses: Emacs Lisp (`.el`), Common Lisp
(`.lisp .cl .lsp`), Scheme (`.scm .ss`), Racket (`.rkt`), Clojure
(`.clj .cljs .cljc .edn`). `struct read` recognizes definition forms per dialect
(`defun`, `defvar`, `defmacro`, `define`, `defn`, `def`, …). Automatic reindent
after structural edits, and the standalone `format` command, are Emacs
Lisp–specific; on other dialects your edits are written as given.

## Running as an MCP server

`lisplens mcp` runs a stdio JSON-RPC MCP server exposing the same operations as
tools: `struct_read`, `line_read`, `struct_edit`, `line_edit`, `find`, `refs`,
`format`. The edit tools take the patch as a `patch` string argument (same
grammar as above; payloads are passed as JSON strings, so the `<<TAG` heredoc
fencing is a CLI-only concern).

Register it with Claude Code (project-local example) by adding to `.mcp.json`:
```json
{
  "mcpServers": {
    "lisplens": { "command": "lisplens", "args": ["mcp"] }
  }
}
```
Or with the CLI: `claude mcp add lisplens -- lisplens mcp`. The CLI path (piping
patches into `lisplens struct edit` / `line edit`) needs no server and is the
simplest option for one-off edits; the MCP server is worth it when a session
does a lot of Lisp editing and you want the operations as first-class tools.
