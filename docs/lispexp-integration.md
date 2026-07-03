# lisplens ↔ lispexp integration map

lisplens is built on [lispexp](https://crates.io/crates/lispexp), plus its companion crate [lispexp-emacs](https://crates.io/crates/lispexp-emacs) for the Emacs-specific data/interpreters (lispexp ADR-0033). This maps each backend capability lisplens needs (with the driving lisplens ADR) to the concrete upstream API that provides it.

**Status: as of lispexp 0.6.0, every capability lisplens needs is available upstream — Structural mode is not blocked on lispexp.** This file was formerly a wishlist ("features wanted from lispexp"); the backend has since landed all of it, so it is now a satisfied-by mapping. 0.5.0 added `Datum::dot_span()` (improper-list dot separator) for the formatter — see [lispexp-feedback/0002](lispexp-feedback/0002-improper-list-dot-span.md). 0.6.0's companion `lispexp-emacs 0.1` now **owns the Emacs-specific data lisplens used to re-implement**: the bundled indent table, the file-local `-*- … -*-`/`Local Variables:` reader, and the `.dir-locals.el` evaluator — lisplens consumes them rather than carrying its own copies.

Guiding constraint (lisplens [ADR-0003](adr/0003-semantic-ceiling-form-annotator-level.md)): everything here stays at the **form-annotator level** — purely syntactic, no binding resolution, no macro expansion, no evaluation — consistent with lispexp's reader-only scope.

| Need | lisplens ADR | lispexp API (0.2.1) |
| --- | --- | --- |
| Polyglot definition annotation (is-a-def, name, kind, role slots) | [0013](adr/0013-one-canonical-result-terse-text-default-json-opt-in.md), [0016](adr/0016-structural-address-is-an-s-expression-hash-shorthand.md) | `annotate::bundled_registry(Dialect)` + `annotate_tree` / `annotate_form` → `Annotated::{first,all}(Role)`; bundled builtins for Scheme/Guile/Gauche/…, Racket, Common Lisp, Emacs Lisp, Clojure, Phel, Fennel, Janet, Hy, LFE |
| Method Dispatch signature (qualifier + specializers) | [0009](adr/0009-structural-falls-back-to-line-hash-dispatch-signature-progressive.md) | `Role::{Qualifier, DispatchValue, SpecializedArglist}` + `Annotated::specialized_params()` / `split_specialized_arglist` (lispexp ADR-0021) |
| Indent-spec exposure (native formatter) | [0011](adr/0011-formatting-is-lisplens-responsibility-pluggable-formatter.md) | `indent::{IndentSpec, IndentTable}` (`get` / `insert` / `iter` / `merge`); `harvest_indent_specs` (`declare`/`put`/`function-put`) |
| Bundled standard indent table (formatter start point) | [0011](adr/0011-formatting-is-lisplens-responsibility-pluggable-formatter.md) | `lispexp_emacs::indent::bundled_table(Dialect::EmacsLisp)` — the byte-identical former in-tree `NUMBER_SPECS`/`DEFUN_SPECS` (342 entries); lisplens `merge`s harvested specs on top |
| File-local variable reader (`-*- … -*-` + `Local Variables:`) | [0029](adr/0029-format-config-resolution-order.md) | `lispexp_emacs::local_vars::{file_locals, FileLocals}` → `.iter()` of `(name, value-text)`, last-wins; lisplens's `set_var` interprets the raw text |
| Dir-local evaluator (`.dir-locals.el`) | [0029](adr/0029-format-config-resolution-order.md) | `lispexp_emacs::dir_locals::{DirLocals, DirLocalEntry}` — `DirLocals::parse(content).entries()`; lisplens keeps its own multi-mode applicability filter and the directory-walk/`.dir-locals-2.el` precedence loop |
| Improper-list dot span (formatter dotted-tail align) | [0011](adr/0011-formatting-is-lisplens-responsibility-pluggable-formatter.md) | `Datum::dot_span()` / `DatumKind::List.dot` (lispexp 0.5.0) |
| Parse-error diff (validate-then-write) | [0005](adr/0005-validate-then-write-reject-only-new-parse-errors.md) | `ErrorKind` (`PartialEq + Eq + Hash`, deliberately position-stable for set-diffing); `Parsed.errors: Vec<ParseError>` |
| Line/byte index (Line-hash + diagnostics) | [0006](adr/0006-mode-first-command-surface-with-batch-edits.md), [0013](adr/0013-one-canonical-result-terse-text-default-json-opt-in.md) | `LineIndex::{new, line_count, offset_to_line_col, line_col_to_offset, line_range}` — `line_range` excludes the terminator, matching the ADR-0008 line policy |
| EDN / manifest coverage (zero-config scan) | [0015](adr/0015-zero-config-scans-repo-local-manifests.md) | `Options::edn()` / `Dialect::Edn`, plus the per-dialect presets; lisplens parses manifests with lispexp itself |
| Code-vs-data traversal (Project search) | [0010](adr/0010-edits-single-file-atomic-discovery-project-wide.md) | `walk::{walk, Class, Walk}` |

## Notes / residual

- **Shared line model.** lispexp's reader numbers `Datum.line` with the same policy as `LineIndex` (both break on `\n`; `\r\n` counts once; a lone `\r` is not a break), so Structural `Datum.line` and Line-hash line numbers agree — which Mode fallback (ADR-0009) requires. lisplens's Line-hash `read` consumes `LineIndex` so the two layers can never diverge. (Internally lispexp's reader keeps its own `line_starts`; reusing `LineIndex` there is a possible DRY cleanup, not a correctness gap.)
- **`Options` → `Dialect`.** `bundled_registry` takes a `Dialect`; lisplens maps a file to a `Dialect` (parallel to its extension→`Options` guess) to select the registry.
- **`#lang info`** (Racket) parses as data; interpreting its restricted grammar (`if` / `string-append` / `getenv`) stays lisplens's concern (ADR-0015).
- **Coverage spot-check.** For any dialect lisplens targets that is absent from `bundled_registry` (e.g. ISLisp, AutoLISP), confirm definition coverage or supply an lisplens-side `Registry` via `FormSpec`.

## Implication

Building the real Structural Lens (replacing the heuristic `outline`, audit #4) is unblocked: derive the `Dialect`, build `bundled_registry(dialect)`, run `annotate_tree`, read `Role::Name` / `Category` for `name` / `kind`, and hash each node's span bytes (ADR-0008) for its anchor.
