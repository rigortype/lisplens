# lisplens — status snapshot

Ephemeral snapshot for continuing toward first release. **Durable knowledge is
in the dev docs** (see `AGENTS.md` → Codebase): `docs/dev/architecture.md`,
`docs/dev/formatter.md`, `CONTEXT.md`, `docs/adr/`.

## Now

- 94 tests pass, `cargo clippy --all-targets` clean; tree clean. 30 ADRs.
- **Touched-region auto-format on Structural edit (ADR-0025/0028) is wired**:
  `apply_struct_patch` reindents the top-level forms an edit fell within
  (`format::reindent_range` + `edit::splice_tracked`), Emacs Lisp only, others
  byte-identical; Line-hash stays literal (ADR-0027).
- **`format <anchor>` Structural verb (ADR-0028 point 3)**: reindent exactly one
  anchored form in place — even nested, in full context (`format::reindent_block`,
  the `exact` scope of `Touched`). Carried as an identity edit so it shares the
  splice/conflict path. 13 Structural verbs now.
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

## Deferred (future work — not blocking first release)

The Emacs Lisp formatter is effectively complete; what remains is deliberately
parked. In rough priority for whenever it is picked up again:

1. **Formatter's true long tail (niche).** Nested specforms where Emacs's
   `(COLUMN . start)` list-return semantics differ from the plain column (e.g. a
   long `if-let` condition), and package-local macros absent from the
   bundled/harvested specs. Hard to even *locate*: the batch harness buries them
   under declare-artifacts (see the harness caveat above), so finding them needs
   a fair reference — compare against the original file, not batch Emacs. Low
   value, high effort; parked.
2. **More real-world elisp validation.** Header/footer and tab-mode files;
   config resolution end-to-end on real repos. Easy to start, open-ended; run the
   harness on new corpora when convenient.
3. **Single `;` inline (not own-line) comment alignment** — the own-line case is
   done; inline comments would need the `lex` trivia layer (lispexp-feedback/0002).
4. **Other dialects' formatters**, **MCP edit JSON op-array** (ADR-0019), and
   **S-expr structural addresses** (ADR-0018 defers these). Each is its own
   design-first chunk on a separate surface.
