# lisplens — status snapshot

Ephemeral snapshot. **Durable knowledge is in the dev docs** (see `AGENTS.md` →
Codebase): `docs/dev/architecture.md`, `docs/dev/formatter.md`, `CONTEXT.md`,
`docs/adr/`.

## Now

- **Common Lisp indenter landed** (ADR-0031, 2026-07-04): the formatter is now
  **one shared driver + a dialect-selected engine**. `src/format.rs` became
  `src/format/mod.rs` (driver + Emacs Lisp engine) plus `src/format/commonlisp.rs`
  — a faithful Rust port of `common-lisp-indent-function` (`cl-indent.el`):
  multi-level backtracking + `path`, the `lisp-indent-259` spec walker, the
  bundled CL table, `tagbody`/`do`/`defmethod`/lambda-hack/`loop`, package-prefix
  stripping, and lambda-list keyword alignment. `format(source, config, dialect)`
  dispatches; `.lisp/.lsp/.cl/.asd` → CL engine, non-bundled dialects
  (Clojure/Fennel/…) → generic Emacs Lisp fallback. Auto-format-on-edit gated to
  `has_native_engine` (Emacs Lisp, Common Lisp). Byte-exact vs Emacs `lisp-mode`
  on `cl-ppcre` + the `gpg`/`gpgme` CL sources (residual diffs are the
  `lisp-indent-defmethod` flat-harness caveat, trailing newlines, or two
  documented gaps). **Next: the Scheme-family engine (`scheme-indent-function`),
  then the remaining dialects.**
- **Released 0.1.0** (2026-07-03) — on [crates.io](https://crates.io/crates/lisplens)
  (`cargo install lisplens`) and as pre-built binaries on the GitHub Release for
  x86_64/aarch64 Linux + macOS and x86_64 Windows. Tag `vX.Y.Z` → GitHub Actions
  publishes (`.github/workflows/release.yml`); next bump via the
  `lisplens-release-prep` skill. No pinned MSRV (binary tool; deps track recent
  stable Rust).
- **Released 0.1.1** (2026-07-04) — a dependency-only release consuming
  `lispexp-emacs` 0.1 (on `lispexp` 0.6): the bundled indent table and the
  file-local / dir-local **parsers** moved out of lisplens into the companion
  crate (lispexp ADR-0033, commit `02a293a`) — table verified byte-identical,
  −78 net lines, output unchanged. On crates.io + GitHub Release binaries.
- **Delegation boundary reviewed** (`docs/notes/20260704-delegation-boundary-review.md`):
  the current split (lispexp-emacs = Emacs *data + parsers*) is right but
  incomplete — the highest-reuse Emacs *behavior*, the `calculate-lisp-indent`
  indent algorithm in `src/format.rs` (+ `nameless.rs`), is the top remaining
  candidate to move into lispexp-emacs; Emacs config resolution is a smaller
  follow-up. Not started — a roadmap item for lispexp-emacs.
- 101 tests pass, `cargo fmt --check` / `cargo clippy --all-targets` clean; tree
  clean. 31 ADRs.
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
- **Dogfooded on php-mode** (fixed its Emacs-32 build via lisplens patches;
  `docs/notes/20260703-dogfooding-php-mode.md`). Both findings now **fixed**:
  the bundle includes `cc-mode` specs (`c-lang-defconst` etc., 342 entries); and
  Structural-edit auto-format is **Nameless-aware when configured** — a
  `nameless-mode` file-/dir-local resolves `FormatConfig.nameless` and flows into
  the edit path (ADR-0029/0030). Also fixed a dir-locals parser bug (only read
  the dotted mode-entry form, not php-mode's `(MODE (VAR . VAL) …)` form).

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
