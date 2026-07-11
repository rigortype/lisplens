---
name: lisplens
description: >-
  Read and edit Lisp-family source safely and symbol-accurately with the
  `lisplens` CLI — Emacs Lisp, Common Lisp, Scheme, Racket, Clojure (.el .lisp
  .cl .lsp .scm .ss .rkt .clj .cljs .edn). USE FOR: editing a
  defun/defvar/defmacro, renaming a symbol, finding where a name is defined or
  actually called, reindenting, or **comparing two versions of Lisp code to see
  what changed structurally** — even phrased casually ("edit this function",
  "rename this variable everywhere", "where is X defined", "how many times is it
  really called", "clean up the indentation", "what changed between these two
  files", "how did this defun change across releases", "summarize this diff
  structurally"). Prefer it over grep/sed/awk (and over `git diff`/`diff` for
  understanding a Lisp change) and
  over hand-editing with Read+Edit, which match text and so miscount references
  and corrupt siblings like `foo-bar` when you meant `foo`; lisplens resolves
  real symbols, edits by structural anchor, and refuses parse-breaking changes.
  DO NOT USE FOR: non-Lisp files, dependency/build errors (e.g. deps.edn
  resolution), or merely explaining Lisp syntax.
license: MPL-2.0
metadata:
  version: 0.2.3
---

# lisplens

`lisplens` works on Lisp the way a structured editor does: read a file's
**shape** cheaply, address any form by a stable **anchor**, and edit that form
in place — snapshot-checked (nobody changed the file under you) and
parse-checked (a change that would add a syntax error is refused, never
written). There's no setup step — just run the command you need. Only if the
shell reports `lisplens: command not found` should you stop and tell the user
it isn't installed, rather than silently falling back to grep/sed. (Don't spend
a turn on `lisplens --help` first — it's a wasted round-trip; the subcommands
below are all you need, and a missing binary announces itself.)

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

## Refactoring primitives — reach for these first

Common refactors have a one-shot command that resolves every site structurally,
applies them through the safety pipeline, and prints a post-condition summary —
so you skip the `refs` → `line edit` batch → `refs` re-verify assembly:

- `lisplens rename <old> <new> <file>` — symbol-exact rename across the file.
  Never touches siblings (`foo-bar` when you meant `foo`) or comment/string
  mentions; prints `renamed N occurrence(s)` + the new hash. The safe one-shot
  answer to `sed`/`perl -i`.
- `lisplens inline <name> <file>` — replace each call of a function/`defsubst`
  in this file with its body, `let`-binding args to keep single-evaluation and
  order. The definition must be in the same file; unsafe bodies are refused, not
  corrupted.
- `lisplens docstring <name> <file>` — set (or replace) a definition's docstring;
  text is read from stdin (raw — lisplens escapes and quotes it). Covers
  function-like defs (defun/defsubst/defmacro/cl-*, Scheme `(define (name …))`)
  and Elisp variable defs (defvar/defconst/defcustom/…, docstring after the
  value). Docstrings often contain backticks and apostrophes (`` `foo' ``); feed
  the text via a quoted heredoc or a file (`lisplens docstring x f.el < ds.txt`),
  not an interpolated `printf '…' | …`, so your shell can't mangle it before it
  reaches lisplens.
- `lisplens rewrite <file>` — structural pattern→template rewrite (spec on
  stdin): exact s-expr matching, parse-safe, but *not* behaviour-preserving (a
  "structural sed" — you assert the semantics).
- `lisplens extract <file> <anchor> <name> [param…]` — pull a form into a new
  named function.
- `lisplens check <file>` — parse/validate; non-zero exit on errors. Use it
  instead of shelling out to `emacs --batch check-parens`.

Drop to the loop below for edits these don't cover — a one-off form change or a
cross-file inline (the def in another file).

## Compare two versions — `lisplens diff`

To understand how Lisp code changed between two versions — across releases,
before/after a refactor, or two variants of a file — reach for `lisplens diff`
rather than `git diff`/`diff`. It compares by **structure, modulo formatting**,
which is what makes it worth preferring: a reindent or a comment tweak never
shows up as a change; definitions are matched by *name* (and a method's dispatch
signature), not line position, so a function that merely moved isn't reported as
"everything changed"; and the output is token-lean — a compact map first, then
you drill only where it matters, instead of paging through a wall of textual
diff.

**1. What changed — the map.**
```
lisplens diff <old> <new>          # added / removed / changed top-level definitions
```
Rows group under `changed:` / `added:` / `removed:` as `<marker> <kind> <name>
<line>:<hash>`; a trailing `! other top-level forms changed` flags churn in
non-definition forms (`require`/`provide`/…), which aren't diffed individually.
**Empty output means no structural change.** The exit status is **0 whether or
not there are differences** — read the output (or `--json`) to tell, don't test
the exit code; a non-zero exit means a real error (missing file, parse failure,
mismatched dialect).

**2. How a definition changed — drill in.**
```
lisplens diff <old> <new> --deep         # every changed def, drilled into its internals
lisplens diff <old> <new> --unit NAME    # just the definition(s) named NAME
```
A *changed* definition renders as a pruned tree showing only what moved: `~ OLD
⇒ NEW` is a replaced atom/form (a changed head like `when`→`unless` surfaces
here), `+ form` / `- form` is a subform added/removed, and `…` stands for
unchanged siblings elided. An *added* or *removed* definition renders as its
**Lens** — its inner nodes with an anchor + preview each — so you see what a new
function actually contains, not just that its name appeared. Reorders aren't
tracked as moves: a shuffled form reads as an add plus a remove.

Add `--verbose` (implies `--deep`) to render added/removed definitions as their
**full source** instead of the token-bounded Lens preview — for when you want to
read a whole new definition's body, not skim it. It only changes added/removed
rendering; changed defs still show their pruned tree.

This tree is **complete for the "how did it change" question** — every subform
that differs is shown as `~`/`+`/`-`, and everything else is provably unchanged
(the diff is structural). So once you've drilled a definition, you can answer
from the tree directly; **don't re-open the raw source to double-check** — that's
the redundant step the diff exists to save. (Re-read only when you genuinely need
surrounding code the diff didn't touch, e.g. to understand a *caller*.)

**Match the drill depth to the question — don't over-drill.** If the question is
*which* definitions changed (added / removed / changed), the **map alone is the
answer** — stop there, don't drill. Only drill when the question is *how*
something changed inside: `--unit NAME` for the specific definitions you care
about (small, targeted output), or `--deep` for every changed definition's
internals at once. Be aware `--deep` *expands* the whole changeset, so on a large
change it's a big pull; there, answer the "what changed" from the map and drill
just a few key definitions with `--unit`, and reserve `--deep` for small-to-medium
changes or when you genuinely need every internal. The common mistake is drilling
(either `--deep` or a long `--unit` loop) for a question the map already answered.

**3. Machine-readable — `--json`.** Add `--json` to any form above for the same
classification (or tree) as structured data, with an editing **anchor**
(`line:hash`) on each change. Reach for it when you'll act on the result
programmatically or hand it to another step. The MCP `diff` tool mirrors all of
this and adds a form-snippet mode (compare two forms passed as strings, no
files) — see the reference below.

Because every change carries a `line:hash` anchor, `diff` chains straight into
the edit loop: spot a changed definition, then `struct edit` it by that anchor.

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
`insert-after`/`insert-before <anchor> <<TAG…`. `insert-*` place the payload as a
new sibling next to the anchored node — the anchor may be an *inner* node (say a
defun's arglist), so this is how you add a form *inside* another form; the touched
top-level form is reindented so the new line lands at the right column.

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

**Renaming?** Use the `lisplens rename` primitive above — one call, symbol-exact,
whole file. (The `struct edit` `rename` verb is only for renaming within a single
anchored form, e.g. a local inside one defun.)

## Guardrails / troubleshooting

- **`Drift {expected, actual}`** — the file changed under your snapshot. Re-read
  (`line read`/`struct read`), rebuild the patch against the new hash, retry.
- **Parse-error rejection** — your edit would break the parens; nothing was
  written. Fix the payload and re-apply.
- **One snapshot per patch.** All ops share the one `@` header and address the
  same pre-edit snapshot; don't let one op's anchor depend on an earlier op in
  the same patch — apply, take the new hash, build the next.
- **`format` is native for every recognized dialect** (Emacs Lisp, Common Lisp,
  the Scheme family, Clojure/Phel, Fennel/Janet/Hy/LFE, ISLisp) and matches each
  one's own tool (Emacs, `cljfmt`, `phel format`); only EDN *data* rides a
  generic fallback. Auto-reindent-on-edit follows the same coverage.

Full verb grammar, `lisplens format`, dialect coverage, and the MCP server:
[references/patch-dsl.md](references/patch-dsl.md).
