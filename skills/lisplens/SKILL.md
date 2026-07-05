---
name: lisplens
description: >-
  Read and edit Lisp-family source safely and symbol-accurately with the
  `lisplens` CLI — Emacs Lisp, Common Lisp, Scheme, Racket, Clojure (.el .lisp
  .cl .lsp .scm .ss .rkt .clj .cljs .edn). USE FOR: editing a
  defun/defvar/defmacro, renaming a symbol, finding where a name is defined or
  actually called, or reindenting — even phrased casually ("edit this function",
  "rename this variable everywhere", "where is X defined", "how many times is it
  really called", "clean up the indentation"). Prefer it over grep/sed/awk and
  over hand-editing with Read+Edit, which match text and so miscount references
  and corrupt siblings like `foo-bar` when you meant `foo`; lisplens resolves
  real symbols, edits by structural anchor, and refuses parse-breaking changes.
  DO NOT USE FOR: non-Lisp files, dependency/build errors (e.g. deps.edn
  resolution), or merely explaining Lisp syntax.
license: MPL-2.0
metadata:
  version: 0.1.0
---

# lisplens

`lisplens` works on Lisp the way a structured editor does: read a file's
**shape** cheaply, address any form by a stable **anchor**, and edit that form
in place — snapshot-checked (nobody changed the file under you) and
parse-checked (a change that would add a syntax error is refused, never
written). Confirm once that it's installed with `lisplens --help`; if it's
missing, say so rather than silently falling back to grep/sed.

## Why prefer it over grep/sed/Read+Edit

Text tools don't understand Lisp, and on real code that bites:

- **References.** `grep -c foo` counts characters — comment mentions, string
  contents, and longer symbols like `foo-bar` that merely contain your name — so
  "how many times is X called" comes out wrong. `lisplens refs` counts real
  symbol occurrences, each tagged **code vs data**.
- **Renames.** `sed s/foo/…/g` also rewrites `foo-bar`, `foo-baz` — corrupting
  siblings you never meant to touch. lisplens renames the *symbol*.
- **Structure.** Hand-edits can leave an unbalanced paren offscreen; lisplens
  edits whole forms and **re-parses before writing**, so a broken file is never
  committed.
- **Context size.** On a large file, `struct read` gives a compact outline; you
  fetch only the form you touch instead of reading thousands of lines.

## The loop: read shape → anchor → edit → confirm

**1. Read the shape.**
```
lisplens struct read <file>          # outline: <line>  <hash>  <kind>  <name>
lisplens struct read <file> <name>   # zoom into one definition's inner forms
lisplens line read <file>            # line view; its first line is the file-hash
```
`line read`'s header is `[<path>#<file-hash>]`; each row is `<line>:<hash>|<text>`.

Symbol-accurate project queries (note: they take a **directory**, default `.`,
not a file — read the path column):
```
lisplens find <name> [dir]   # where <name> is DEFINED
lisplens refs <name> [dir]   # every occurrence, tagged code vs data
```

**2. Anchor.** An anchor is `line:hash` — the first two columns of a read. On a
same-line hash collision a read emits `line:hash:ordinal`; use it verbatim.

**3. Edit.** Pipe a **patch** into `struct edit` or `line edit`:
```
@ <file-hash>                       # the snapshot you built against; stale → rejected
<verb> <anchor> [args] [<<TAG]      # text payloads use a heredoc closed by TAG
  ...payload...
TAG
```
Get the `@` hash from `line read`'s header or the `ok <hash>` a prior edit
printed. Shared verbs: `replace <anchor> <<TAG…`, `delete <anchor>`,
`insert-after`/`insert-before <anchor> <<TAG…`.

- **`struct edit`** — replace/restructure a *whole form*; on Emacs Lisp it also
  reindents the touched top-level forms, preserving the file's existing
  tabs/spaces. Adds paredit verbs (`wrap`, `raise`, `splice`, slurp/barf,
  `split`, `join`, `rename`, `format`).
- **`line edit`** — touch *lines* verbatim, leaving surrounding text
  byte-for-byte (no reindent). Use it when byte-fidelity matters.

Default to `struct edit` for form-level work; drop to `line edit` for
line-precise or fidelity-critical tweaks.

**4. Confirm.** Success prints `ok <new-file-hash>` (written + re-validated);
that hash is now current. Failure prints the reason (`Drift {…}`, a parse error)
and writes nothing — re-read if it drifted, then retry.

### Example (struct edit)

`struct read s.el` → `1  a406  defun  my-increment`; `line read` header
→ `[s.el#2e0ad73aecc00598]`. Replace the whole form:
```
lisplens struct edit s.el <<'PATCH'
@ 2e0ad73aecc00598
replace 1:a406 <<LISP
(defun my-increment (n &optional step)
  "Increment N by STEP (default 1)."
  (+ n (or step 1)))
LISP
PATCH
```
→ `ok <new-hash>`, reindented and re-validated.

**Renaming across a whole file** (a symbol used in several top-level forms):
`struct edit`'s `rename` verb is subtree-scoped, so instead enumerate the exact
sites with `lisplens refs <name>` and apply a `line edit` (or per-form `rename`)
at each — you rename only real occurrences, never the siblings a blind `sed`
would clobber.

## Guardrails / troubleshooting

- **`Drift {expected, actual}`** — the file changed under your snapshot. Re-read
  (`line read`/`struct read`), rebuild the patch against the new hash, retry.
- **Parse-error rejection** — your edit would break the parens; nothing was
  written. Fix the payload and re-apply.
- **One snapshot per patch.** All ops share the one `@` header and address the
  same pre-edit snapshot; don't let one op's anchor depend on an earlier op in
  the same patch — apply, take the new hash, build the next.
- **`format` is Emacs Lisp only.** Other dialects parse and edit fine, but don't
  run `format` on `.lisp`/`.scm`/`.clj`.

Full verb grammar, `lisplens format`, dialect coverage, and the MCP server:
[references/patch-dsl.md](references/patch-dsl.md).
