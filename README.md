# lisplens

Token-efficient, polyglot Lisp editing for AI agents — a CLI (and, later, MCP)
tool built on the [lispexp](https://crates.io/crates/lispexp) reader.

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

## CLI

```
lisplens struct read <file> [name]   Outline of definitions; with a name,
                                     expand it to list inner-node anchors
lisplens line   read <file>          line-hash view ([path#hash] + N:hash|content)
lisplens struct edit <file>          apply a Structural patch from stdin
lisplens line   edit <file>          apply a Line-hash patch from stdin
lisplens find <name> [dir]           find definitions by name across a project
lisplens refs <name> [dir]           find symbol occurrences (code/data tagged)
lisplens format <file>               reindent an Emacs Lisp file (honors
                                     indent-tabs-mode / tab-width from file-local
                                     vars, .dir-locals.el, and .editorconfig)
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
`rename <from> <to>`. Line-hash ops: `replace`, `delete`, `insert-after`,
`insert-before`.

lisplens owns whitespace: a `replace` keeps a line's terminator; an `insert`
gets one. Agents supply content, not spacing.

## Status

Working CLI and MCP server for both modes (read, expand, edit, find, refs),
validate-then-write warnings, and a native Emacs Lisp `format` (byte-exact with
Emacs on common code; other dialects and touched-region auto-format are future
work). See
[`docs/adr/`](docs/adr/) for what's decided and
[`docs/lispexp-integration.md`](docs/lispexp-integration.md) for how the backend
is used.

## License

Licensed under the Mozilla Public License 2.0 (MPL-2.0).
