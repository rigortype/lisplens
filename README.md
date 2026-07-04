# lisplens

Token-efficient, polyglot Lisp editing for AI agents — a CLI and MCP tool built
on the [lispexp](https://crates.io/crates/lispexp) reader.

lisplens lets an agent **read a file's shape cheaply**, get a stable **anchor**
for any target, and **edit by anchor** without resending the whole file. Every
edit is drift-checked, validated (never makes a file's syntax worse), and
written atomically. It works across Common Lisp, Scheme, Emacs Lisp, Clojure,
Racket, Fennel, Janet, Hy, LFE, Phel, and more — the dialect is guessed from the
file extension (zero-config).

The design is recorded in [`CONTEXT.md`](CONTEXT.md) (domain glossary) and
[`docs/adr/`](docs/adr/) (architecture decisions).

## Two modes

- **Structural mode** — address code by definition, and drill into any inner
  node, via lispexp's parse tree. Structure-aware paredit-style edits.
- **Line-hash mode** — address code by line, hashline-style. Line-oriented,
  dialect-agnostic.

Both anchor on a short content hash: a read emits the hash, an edit hands it
back, and a mismatched or drifted file is refused (you re-read).

## Install

```sh
cargo install lisplens          # from crates.io — puts `lisplens` on your PATH
```

Or download a pre-built binary for your platform from the
[latest release](https://github.com/rigortype/lisplens/releases/latest).

As an MCP server, point your client at the `lisplens mcp` command.

## CLI

```
lisplens struct read <file> [name]   Outline of definitions; with a name,
                                     expand it to list inner-node anchors
lisplens line   read <file>          line-hash view ([path#hash] + N:hash|content)
lisplens struct edit <file>          apply a Structural patch from stdin
lisplens line   edit <file>          apply a Line-hash patch from stdin
lisplens find <name> [dir]           find definitions by name across a project
lisplens refs <name> [dir]           find symbol occurrences (code/data tagged)
lisplens check  <file>               parse-check; diagnostics, non-zero on errors
lisplens rename <old> <new> <file>   rename a symbol file-wide (symbol-exact, safe)
lisplens inline <name> <file>        inline a function at its call sites (safe subset)
lisplens rewrite <file>              structural pattern->template rewrite (spec on stdin)
lisplens extract <file> <anchor> <name> [param...]
                                     pull a form (or a run of forms) into a new function
lisplens format <file>               reindent by dialect (native engines; honors
                                     Emacs file-local vars, .dir-locals.el, .editorconfig)
lisplens mcp                         run the MCP server over stdio
```

### Reading

```sh
$ lisplens struct read foo.el
   12  a3f2  defun  my-func
   40  b7e1  cl-defmethod  area :around (circle)

$ lisplens struct read foo.el my-func    # expand: inner nodes get anchors
   12  a3f2  (defun my-func (x) (when (ready? x) (go x)))
   12  2f94    defun
   ...
   14  2857        (go x)
```

### Editing

An edit is a **patch** on stdin: a `@ <file-hash>` header (the snapshot it was
built against) then one op per line, with heredoc payloads. The file-hash gates
the whole batch; a drifted file is refused.

```sh
# Structural: replace a definition by its anchor (line:hash from the read)
$ printf '@ %s\nreplace 40:b7e1 <<END\n(defmethod area ((s circle)) (round (* pi (r s) (r s))))\nEND\n' \
    "$FILEHASH" | lisplens struct edit foo.el
ok 9f3c1a2b4d5e6f70    # the new file-hash
```

Structural ops: `replace`, `delete`, `wrap`, `raise`, `splice`, `slurp-fwd`,
`slurp-back`, `barf-fwd`, `barf-back`, `split @<index>`, `join <anchor2>`,
`rename <from> <to>`, `format` (reindent the anchored form in place). Line-hash
ops: `replace`, `delete`, `insert-after`, `insert-before`.

lisplens owns whitespace: a `replace` keeps a line's terminator; an `insert`
gets one. Agents supply content, not spacing.

### Refactoring

Higher-level transforms as single, parse-safe operations — each is symbol-exact
(never touches strings, comments, or same-named substrings), reindents what it
touched, and refuses any edit that would break the parse:

- `rename <old> <new>` — rename a symbol across a file; siblings like `foo-bar`
  survive when you rename `foo`.
- `inline <name>` — expand a function at its call sites, over the provably safe
  subset (non-recursive, required-params); anything unsafe is refused with a reason.
- `rewrite` — a structural pattern→template "sed" (metavariables, classes,
  non-linear repeats), read as a spec on stdin. See [`docs/rewrite.md`](docs/rewrite.md).
- `extract <anchor> <name>` — pull a form into a new function and replace it with a
  call; `--count N` (a run of forms), `--kind HEAD` (name the def form), `--all`
  (fold every identical occurrence), `--also` (generalize differing sites).
- `check` — a standalone parse-check for scripts and CI (silent when clean,
  non-zero on parse errors).

## Languages

The dialect is guessed from the file extension — zero config. **Reading, editing,
`find`/`refs`, `check`, and the refactoring commands work on every supported
dialect**, since lispexp parses each natively. Indentation (`format`) has two tiers.

**Native indent engines** — a faithful port of that language's own formatter,
validated byte-exact against it, and auto-applied to the touched region on a
Structural edit:

| Dialect | Extensions | Indent oracle |
| --- | --- | --- |
| Emacs Lisp | `.el` | Emacs `lisp-indent-function` |
| Common Lisp | `.lisp` `.lsp` `.cl` `.asd` | Emacs `common-lisp-indent-function` |
| Scheme family | `.scm` `.ss` `.sls` `.sps` `.sld`, Racket `.rkt` | Emacs `scheme-indent-function` |
| Clojure | `.clj` `.cljs` `.cljc` | `cljfmt` (semantic, or `--tonsky` fixed style) |
| Phel | `.phel` | `phel format` |

**Fallback** — Fennel (`.fnl`), Janet (`.janet`), Hy (`.hy`), LFE (`.lfe`), and EDN
(`.edn`) have no Emacs indenter to port, so they ride the generic Emacs Lisp engine
on an explicit `format` (and are not auto-formatted on edit).

### Formatting config

The native engines resolve indentation settings the way Emacs does, from the file
and its project:

- **File-local variables** — a `-*- … -*-` header or a `Local Variables:` block
  (`indent-tabs-mode`, `tab-width`, `lisp-body-indent`, `comment-column`, …).
- **`.dir-locals.el`** — directory-local variables for the file's mode (including
  `clojure-ts-indent-style: fixed` to select Clojure's Tonsky style, and
  `nameless-mode` for Emacs Lisp).
- **`.editorconfig`** — `indent_style` / `indent_size` / `tab_width`.
- Optional [Nameless](https://github.com/Malabarba/Nameless) awareness for Emacs
  Lisp (`--nameless`, or the `nameless-mode` local above).

## Status

Stable CLI and MCP server: both addressing modes (read, expand, edit), project
queries (`find`, `refs`), a standalone `check`, and the refactoring commands
(`rename`, `inline`, `rewrite`, `extract`) — all validate-then-write and
drift-checked. `format` has native indent engines for Emacs Lisp, Common Lisp, the
Scheme family, Clojure, and Phel (each byte-exact against its oracle), with the
remaining dialects on the generic Emacs Lisp fallback. See [`docs/adr/`](docs/adr/)
for what's decided and [`docs/lispexp-integration.md`](docs/lispexp-integration.md)
for how the backend is used.

## License

Licensed under the Mozilla Public License 2.0 (MPL-2.0).
