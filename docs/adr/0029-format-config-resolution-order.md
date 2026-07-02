# Format config resolution order

Formatting parameters — for now `indent-tabs-mode` (tabs vs spaces) and `tab-width` — are resolved from several sources in an Emacs-faithful precedence (highest wins):

1. **File-local variables** — the `-*- … -*-` header line and the footer `Local Variables:` … `End:` block. Most specific.
2. **Directory-local variables** — `.dir-locals-2.el` over `.dir-locals.el`, merged up the directory tree with nearer directories winning.
3. **EditorConfig** — `.editorconfig`, walked up until `root = true`, with nearer files and later sections winning; sections matched by glob.
4. **Defaults** — `indent-tabs-mode` = **nil** (spaces; lisplens's default, not Emacs's `t`), `tab-width` = 8, `lisp-body-indent` = 2.

Resolution applies sources low-to-high so higher overrides: defaults → EditorConfig → dir-locals → file-locals.

## Scope

Variables consumed: `indent-tabs-mode` and `tab-width`. EditorConfig maps `indent_style` → `indent-tabs-mode`, `tab_width` → `tab-width`. Overriding the structural indent unit (`lisp-body-indent` / EditorConfig `indent_size`) is future work.

## Status

accepted

## Consequences

- The indenter computes indentation as visual columns (unchanged); only the rendering of a line's leading whitespace depends on `indent-tabs-mode` / `tab-width`.
- Default output is space-indented, matching the formatter's prior behavior and common Lisp style.
- `.dir-locals.el` is parsed with lispexp; EditorConfig globs support the common subset (`*`, `**`, `?`, `[…]`, `{…}`).
