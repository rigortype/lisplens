# lisplens — status snapshot

Ephemeral snapshot for continuing toward first release. **Durable knowledge is
in the dev docs** (see `AGENTS.md` → Codebase): `docs/dev/architecture.md`,
`docs/dev/formatter.md`, `CONTEXT.md`, `docs/adr/`.

## Now

- 92 tests pass, `cargo clippy --all-targets` clean; tree clean. 30 ADRs.
- **Touched-region auto-format on Structural edit (ADR-0025/0028) is wired**:
  `apply_struct_patch` reindents the top-level forms an edit fell within
  (`format::reindent_range` + `edit::splice_tracked`), Emacs Lisp only, others
  byte-identical; Line-hash stays literal (ADR-0027).
- On **lispexp 0.5** (`dot_span` for improper-list dots — our upstream ask,
  shipped).
- **`lisp-body-indent` / EditorConfig `indent_size` overrides** now resolved
  through `FormatConfig.body_indent` (ADR-0029), scaling every structural step;
  byte-exact vs Emacs with `lisp-body-indent` 4.
- **Lone `;` own-line comments → `comment-column`** (`FormatConfig.comment_column`,
  default 40) matching Emacs `indent-for-comment`. High-value: emacs `lisp/`
  sweep improved 17 files, 0 regressions (ansi-color 11→0, woman 23→2, …).
- **First-release goal: a faithful Emacs Lisp formatter.**
- **Long-tail closed** (all verified byte-exact vs Emacs, 0 regressions across
  emacs `lisp/` + magit/lem sweeps): data lists vs function calls
  (`lisp-indent-function`'s non-symbol-head path), `progn`-style body forms that
  start on the open line, dotted-tail sublists (`(a . (b c))`), `;;;` comment
  lines left in place, and `whitespace-after-open-paren` (`( a b` aligns under
  the first element). `php-mode/lisp` is effectively 100% faithful: 12/13 files
  byte-exact, and the 13th (php-mode-debug.el) is a harness artifact.
- **Harness caveat drives the apparent remaining diffs.** batch Emacs doesn't
  evaluate a file, so it misses that file's own `(declare (indent N))` macros
  (mpc-select-save, jsonrpc-lambda, define-icon, …). lisplens *harvests* those,
  so where the harness "differs" lisplens actually matches the checked-in file —
  confirmed on mpc.el, tab-bar.el, jsonrpc.el, php-mode-debug.el. Real fidelity
  is far above the raw byte-exact count. See [[formatter-harness-declare-caveat]].
- **Nameless-aware indentation (ADR-0030)**: `format --nameless` models
  Nameless's namespace-prefix composition (`php-`→`:`, `font-lock-`→`fl:`).

## Next steps (priority order)

1. **Remaining true long tail (niche)** — nested specforms (long `if-let`
   condition; Emacs's `(COLUMN . start)` return semantics), and package-local
   macros absent from the bundled/harvested specs. The dotted-pair `.`-alignment
   quirk (`'(eval . FORM)`) is now handled via lispexp 0.5's `dot_span`
   (dired.el 53→35 harness diffs). To measure real fidelity, compare against the
   original file, not batch Emacs (which lacks the file's own indent specs).
2. **Explicit block-level `format <anchor>` op** (ADR-0028 point 3) — expose
   `reindent_range` as a Structural patch verb so an agent can tidy one form on
   demand (the reindent component already exists).
3. **More real-world elisp validation** — header/footer and tab-mode files;
   config resolution end-to-end on real repos.
4. Other dialects' formatters; MCP edit JSON op-array; S-expr structural
   addresses.
