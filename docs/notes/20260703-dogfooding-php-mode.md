# Dogfooding: fixing php-mode's Emacs-32 build with lisplens (2026-07-03)

Used lisplens's own edit tools to fix `make` (== `eask compile`) failing on Emacs
32.0.50 in `~/repo/emacs/php-mode`. Three real fixes landed **entirely through
lisplens patches** (no hand-editing), and the session surfaced two lisplens
limitations — one now fixed, one standing.

## What was fixed in php-mode (via lisplens)

| Warning (Emacs 31/32) | Fix | lisplens op |
| --- | --- | --- |
| `(any)` in `rx` is obsolete (means `not-newline`) — php.el ×4, php-mode.el ×1 | bare `any` → `nonl` (preserves not-newline) | Line-hash `replace` |
| `font-lock-*-face` obsolete as variables (31.1) — 6 sites | wrap the `setq-local`s in `with-suppressed-warnings` (behaviour-preserving; the vars are still consulted, font-lock.el:582) | Line-hash `replace` + `delete` |
| `c-after-brace-list-decl-kwds` "got no (prior) value … cyclic reference" | add the paired `(c-lang-defconst c-after-brace-list-decl-kwds php nil)` php-mode was missing | Line-hash `insert-after` |

The cc-mode message is pre-existing: cc-mode renamed `c-brace-list-decl-kwds` →
`c-enum-list-kwds` (and the `after-` variant); its transitional lookup
(`c-after-enum-list-key`) tries the old name and, finding none for PHP, printed
the message to stderr, which `eask` counts as a failure. php-mode already defined
the old `c-brace-list-decl-kwds` but not the `after-` twin — defining it silences
the lookup. `make clean && make` → exit 0, zero warnings, php-mode loads.

## Why Line-hash, not Structural, edits

Structural edits auto-format the touched region (ADR-0025/0028) — and that is
exactly the first limitation below — so they would have mangled php-mode's
Nameless indentation. Line-hash edits are literal (ADR-0027), so they were the
safe tool here. This is itself the finding: **the natural tool for a Nameless
file is the literal one.**

## Finding 1 (standing) — Structural-edit auto-format is not Nameless-aware

`apply_struct_patch` reindents the touched top-level form with `format::reindent`
and **no Nameless context**. `php-mode/lisp` is indented under Nameless
(`php-`→`:`), so auto-formatting any touched form would shift every line in it to
non-Nameless columns — corruption. Root cause: Nameless is a CLI opt-in
(`format --nameless`), not part of the resolved `FormatConfig`, so the edit path
cannot know to apply it.

**Direction:** make Nameless a *resolvable* setting (a dir-local / project signal
in `config::resolve`) so both the `format` command and the edit auto-format honor
it; or add an auto-format opt-out. Until then, edit Nameless files with Line-hash
patches. (Confirmed the hazard live: a stray `format --nameless` on php-mode.el
reindented its whole file — see Finding 2 — and had to be reverted.)

## Finding 2 (fixable) — external-package indent macros are unknown

`format` (and `--nameless`) is **not idempotent** on php-mode.el: lisplens
harvests `declare`/`put` specs from the file and bundles *core-Emacs* specs, but
macros defined by a required *package* are unknown, so their forms reindent as
plain calls. The big offender is cc-mode's `c-lang-defconst` (indent spec `1`),
which php-mode.el uses heavily — every `(c-lang-defconst NAME LANG VAL…)` body
shifted. cc-mode ships with Emacs, so its specs are a fair thing to bundle.

**Fixed (this session):** added `(require 'cc-mode)` to the dump recipe and
bundled the 16 new specs it surfaces — `c-lang-defconst 1`, `c-with-syntax-table
1`, `c-save-buffer-state 1`, … — into `NUMBER_SPECS`/`DEFUN_SPECS` (342 entries
now). `format --nameless` on php-mode.el no longer misindents any
`c-lang-defconst` form; `c-make-keywords-re` bodies now match the checked-in
file. The residual `format --nameless` diff (≈66 lines) is the ordinary long
tail (char literals, arg-alignment nuances, and spots where the checked-in file
itself isn't `indent-region`-clean), not the wholesale corruption from before.
