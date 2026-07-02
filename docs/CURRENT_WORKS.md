# lisplens — status snapshot

Ephemeral snapshot for continuing toward first release. **Durable knowledge is
in the dev docs** (see `AGENTS.md` → Codebase): `docs/dev/architecture.md`,
`docs/dev/formatter.md`, `CONTEXT.md`, `docs/adr/`.

## Now

- 83 tests pass, `cargo clippy --all-targets` clean; tree clean. 30 ADRs.
- **First-release goal: a faithful Emacs Lisp formatter.**
- Formatter fidelity: 20 random `~/local/src/emacs/lisp` files, stripped and
  reformatted, diffed against Emacs `indent-region` → **12/20 byte-exact,
  ~99.0% of lines match**. (Harness: `docs/dev/formatter.md`.)
- **Nameless-aware indentation (ADR-0030)**: `format --nameless` models
  Nameless's namespace-prefix composition (`php-`→`:`, `font-lock-`→`fl:`) in
  the column model. On `php-mode/lisp` all seven Nameless-affected lines match
  Emacs (2/5 files byte-exact; the rest diff only on the pre-existing long tail).

## Next steps (priority order)

1. **Formatter fidelity long tail (~1%)** — nested specforms (long `if-let`
   condition; Emacs's `(COLUMN . start)` return semantics), and package-local
   macros not in the bundled/harvested specs. Last run still diffing:
   sgml-mode(144/2716), ob-ruby(58), mouse(46/3856), etags-regen(18),
   korean(12), ia-sb(4), texi(2), dframe(2). Close them one at a time with the
   harness.
2. **Wire touched-region auto-format on Structural edit** (ADR-0025/0028) —
   `format_elisp` is whole-file; add a block-range reindent and call it from
   `apply_struct_patch`.
3. **More real-world elisp validation** — header/footer and tab-mode files;
   config resolution end-to-end on real repos.
4. Other dialects' formatters; `lisp-body-indent` / EditorConfig `indent_size`
   overrides; MCP edit JSON op-array; S-expr structural addresses.
5. lispexp asks — `docs/lispexp-integration.md`, `docs/lispexp-feedback/`.
