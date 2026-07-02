# Nameless-aware indentation (opt-in)

Some Emacs Lisp is indented as it *looks* under [Nameless](https://github.com/Malabarba/Nameless), which hides a package's namespace prefix. With `nameless-affect-indentation-and-filling` at its default `'outside-strings`, Nameless composes the prefix region to a shorter glyph, so Emacs's `current-column` — and therefore every alignment column — is measured against the **displayed** width, not the literal characters. Files edited this way (e.g. `php-mode`'s `lisp/`) are checked in with that narrower alignment; formatting them without modelling Nameless produces spurious diffs.

## What is modelled

Per file, a set of **composed prefixes** shrink the column measurement:

- **Current name** — `nameless-current-name`, auto-discovered from the file name the way Nameless does: strip `(-mode)?(-tests?)?\.[^.]*$` from the package name (so `php-mode.el` → `php`, `php-project.el` → `php-project`). A symbol `NAME-rest` displays as `:rest`; the matched region `NAME-` (with `nameless-separator` `-`) collapses to `nameless-prefix` `:` (1 column).
- **Global aliases** — `nameless-global-aliases`, defaulting to `(("fl" . "font-lock"))`. `font-lock-rest` displays as `fl:rest`; the region `font-lock-` (10) collapses to `fl:` (3).

Each occurrence at a symbol start contributes a saving of `region_len - display_len` to every column measured to its right on the same line. Column measurement (`Cols::col`) subtracts, for the target offset, the savings of all composed prefixes that begin earlier on its line. Savings sit inside token text, never in leading whitespace, so they are stable under reindentation and the [reindent invariant](../dev/formatter.md) is preserved.

`nameless-private-prefix` is **not** modelled, because it has no effect on width: a private symbol `foo--bar` collapses `foo-`→`:` (save 3) when nil and `foo--`→`::` (save 3) when `t` — the extra separator character is matched by an extra prefix glyph, so the saving is identical either way. It only changes the displayed glyph, which indentation never sees.

## Enablement

Off by default — enabling it globally would corrupt the non-Nameless corpora (magit, lem, Emacs core). It is an explicit opt-in: `lisplens format --nameless FILE`. Nameless is a property of the author's editor (an `emacs-lisp-mode-hook`), not of the file, so nothing in the file itself can trigger it. Current name is derived from the file name and aliases are the default set; reading `nameless-current-name` / `nameless-aliases` as file-/dir-local variables is desirable but deferred (the real corpora set none).

## Status

accepted

## Consequences

- `format_elisp` gains a Nameless context; the fidelity harness must enable Nameless on the Emacs side (`nameless-mode` + a forced `font-lock-ensure`, since batch redisplay never applies the composition on its own) to compare fairly.
- The model is column-measurement only: specform/body offsets (`open_col + N`) shift exactly when earlier text on the open line composes shorter; align-under-first-arg shifts by the prefixes between the open paren and the first argument. This mirrors Emacs.
- Unicode is treated as it is elsewhere in the formatter: byte columns, fine for the ASCII-heavy prefixes Nameless targets.
