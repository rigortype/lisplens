---
name: lisplens
description: >-
  Parse-safe, symbol-accurate reading and editing of Lisp-family source (Emacs
  Lisp, Common Lisp, Scheme, Racket, Clojure — .el .lisp .cl .lsp .scm .ss .rkt
  .clj .cljs .edn) with the `lisplens` CLI. Use whenever you read or change Lisp:
  editing a defun/defvar/defmacro, renaming a symbol, finding where a name is
  defined or actually called, or reindenting — even when the user just says "edit
  this function", "rename this variable everywhere", "where is X defined", "how
  many times is it really called", or "clean up the indentation". Prefer it over
  grep/sed/awk and over hand-editing with Read+Edit: those match text, so they
  miscount references (comments, strings, longer names that merely contain yours)
  and, on a rename, silently corrupt siblings like `foo-bar` when you meant `foo`.
  lisplens resolves real symbols, edits by structural anchor, and refuses any
  change that would break the parse — the safe default. On large files it also
  keeps context small by reading shape, not the whole file.
---

# lisplens

`lisplens` works on Lisp the way a structured editor does: read a file's
**shape** cheaply, address any form by a stable **anchor**, and edit that form
in place — with a snapshot (drift) check and a parse-validity check so a bad edit
is rejected instead of silently corrupting the file.

## Why prefer it over grep/sed/Read+Edit

The trap with text tools is that they don't understand Lisp — and on real code
that's not a theoretical risk, it bites:

- **References.** `grep -c 'c-macro-cache'` counts every line the *characters*
  appear on: mentions in comments, occurrences inside strings, and — worst —
  longer symbols like `c-macro-cache-start-pos` that merely contain your name.
  You get an inflated, wrong answer. `lisplens refs` counts real symbol
  occurrences and tags each **code vs data**, so "how many times is X actually
  called" has a correct answer.
- **Renames.** A `sed s/c-macro-cache/.../g` (or a naive multi-file replace)
  rewrites `c-macro-cache-start-pos`, `c-macro-cache-syntactic`, and
  `c-macro-cache-no-comment` too — corrupting three sibling variables you never
  meant to touch. lisplens renames the *symbol*, leaving siblings intact.
- **Balanced structure.** Hand-editing can leave an unclosed paren three
  screens away from where you were looking. lisplens edits whole forms and
  **re-parses before writing** — an edit that would add a syntax error is
  refused, so you never commit a broken file.
- **Context size.** On a large file (a few thousand lines and up), reading the
  whole thing to change one form is wasteful. `struct read` gives you a compact
  outline; you fetch only the form you touch.

A careful text-tool workflow (grep to locate, read just a region, edit) can get
simple single-site tasks right too — so this isn't about raw token savings on
every task. It's that lisplens makes the *correct, safe* path the easy one, and
removes the failure modes above. For any non-trivial read or edit of a Lisp
file, reach for it first.

## Prerequisite

`lisplens` should be on your `PATH`. Confirm once with `lisplens --help` (it
prints its usage). If it's missing, say so rather than silently falling back to
grep/sed — the user installed it on purpose.

## The core loop: read shape → anchor → edit → confirm

### 1. Read the shape

```
lisplens struct read <file>
```
prints one line per top-level form: `<line>  <hash>  <kind>  <name>`. That's
your map. To zoom into one definition's inner forms, pass its name:
```
lisplens struct read <file> <name>
```
For a line-oriented view (and the **file-hash** you'll need to edit):
```
lisplens line read <file>
```
The first line is `[<path>#<file-hash>]`; each following line is
`<line>:<hash>|<content>`.

Project-wide, symbol-accurate, no full-repo grep:
```
lisplens find <name> [dir]   # where <name> is DEFINED (defun/defvar/…)
lisplens refs <name> [dir]   # every occurrence, tagged code vs data
```
Note `find`/`refs` take a **directory** (default `.`), not a file — run them on
the containing dir and read the path column. Their counts are *symbol* counts:
comment/string mentions and longer symbols that contain your name are excluded,
which is exactly why they answer "is it defined / how often is it really used"
correctly where grep can't.

### 2. Understand the anchor

An **anchor** is `line:hash` — the first two columns of `struct read` (and of
`line read`). On the rare same-line hash collision, reads emit a third field
`line:hash:ordinal`; use it verbatim. Anchors name the form to edit without
pasting it back.

### 3. Edit by anchor

Pipe a **patch** into `lisplens struct edit <file>` or `lisplens line edit
<file>`:

```
@ <file-hash>
<verb> <anchor> [args] [<<TAG]
  ...payload lines (for verbs that carry text)...
TAG
```

- `@ <file-hash>` asserts the snapshot you built against. Get it from the
  `line read` header, or from the `ok <file-hash>` printed by your previous
  successful edit. Stale → the whole patch is rejected (drift); re-read and
  rebuild.
- Text payloads use a heredoc: end the op line with `<<TAG`, close with a line
  equal to `TAG`. Pick a tag absent from the payload (e.g. `LISP`, `EOF2`).
- Success prints `ok <new-file-hash>`; failure prints the reason and writes
  nothing.

**Which mode?**
- **`struct edit`** — replacing/deleting/restructuring a *whole form* (a defun, a
  binding, a call). On Emacs Lisp it also **reindents the touched top-level
  forms**, correctly preserving the file's existing tabs/spaces — so you don't
  hand-fix indentation and don't risk whitespace churn.
- **`line edit`** — touching *lines* regardless of form boundaries, leaving
  surrounding text byte-for-byte alone (no reindent).

Default to `struct edit` for form-level work; drop to `line edit` for
line-precise tweaks.

### 4. Confirm

`ok <hash>` means it's written and re-validated; that hash is now current. On
failure, read the message (`Drift {…}`, a parse error), re-read if it drifted,
and retry.

## Worked example (struct edit)

`lisplens struct read s.el` → `1  a406  defun  my-increment`; `line read` header
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

## Common verbs (both modes)

- `replace <anchor> <<TAG…` — swap the form/line for the payload
- `delete <anchor>` — remove it (no payload)
- `insert-after <anchor> <<TAG…` / `insert-before <anchor> <<TAG…`

`struct edit` adds paredit-style verbs (`wrap`, `raise`, `splice`, slurp/barf,
`split`, `join`, `rename`, `format`) — see
[references/patch-dsl.md](references/patch-dsl.md) for the full grammar,
`lisplens format`, dialect coverage, and the MCP server.

## Renaming a symbol

- **Within one form** (a local/parameter, or occurrences under one definition):
  `struct edit` with `rename <anchor> <from> <to>` — it renames the symbol
  inside that anchored subtree only.
- **Across the whole file** (a variable/function used in several top-level
  forms): `rename` is subtree-scoped, so instead enumerate the exact sites with
  `lisplens refs <name>`, then apply a `line edit` (or a per-form `rename`) at
  each. Because `refs` matches the *symbol*, you rename only the real
  occurrences and never the sibling names that merely contain it — the thing a
  blind `sed` gets wrong.

## Guardrails worth knowing

- **One snapshot per patch.** All ops in a patch share the one `@` header and
  address the same pre-edit snapshot. Don't let one op's anchor depend on an
  earlier op in the same patch — apply, take the new hash, build the next patch.
- **Re-read after drift, or after any edit you made outside lisplens.** Anchors
  and the file-hash are only valid against the snapshot you read.
- **`format` is Emacs Lisp only.** Other dialects parse and edit fine, but the
  native reindenter targets Elisp; don't run `format` on `.lisp`/`.scm`/`.clj`.
