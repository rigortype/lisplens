# Format config resolution order

Formatting parameters — for now `indent-tabs-mode` (tabs vs spaces) and `tab-width` — are resolved from several sources in an Emacs-faithful precedence (highest wins):

1. **File-local variables** — the `-*- … -*-` header line and the footer `Local Variables:` … `End:` block. Most specific.
2. **Directory-local variables** — `.dir-locals-2.el` over `.dir-locals.el`, merged up the directory tree with nearer directories winning.
3. **EditorConfig** — `.editorconfig`, walked up until `root = true`, with nearer files and later sections winning; sections matched by glob.
4. **Defaults** — `indent-tabs-mode` = **nil** (spaces; lisplens's default, not Emacs's `t`), `tab-width` = 8, `lisp-body-indent` = 2.

Resolution applies sources low-to-high so higher overrides: defaults → EditorConfig → dir-locals → file-locals.

## Scope

Variables consumed: `indent-tabs-mode`, `tab-width`, `lisp-body-indent` (the structural indent unit, default 2), and `comment-column` (lone-`;` comment alignment, default 40). EditorConfig maps `indent_style` → `indent-tabs-mode`, `tab_width` → `tab-width`, and `indent_size` → `lisp-body-indent`; `comment-column` has no EditorConfig equivalent. The same precedence applies to all.

## Status

accepted

## Consequences

- The indenter computes indentation as visual columns; the *width* of a structural step is `lisp-body-indent` (so it scales body forms and `2×` distinguished args), while `indent-tabs-mode` / `tab-width` only affect how a line's leading whitespace is rendered.
- Default output is space-indented, matching the formatter's prior behavior and common Lisp style.
- `.dir-locals.el` is parsed with lispexp; EditorConfig globs support the common subset (`*`, `**`, `?`, `[…]`, `{…}`).
