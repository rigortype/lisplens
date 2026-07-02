# lispexp features wanted by lisplens

lisplens is built on [lispexp](https://crates.io/crates/lispexp). This document collects the backend capabilities lisplens needs from lispexp, each tied to the lisplens ADR that drives it, with the current lispexp state and a proposed API direction. Each item is written to be filed as a lispexp issue.

Guiding constraint (from lisplens [ADR-0003](adr/0003-semantic-ceiling-form-annotator-level.md)): everything here stays at the **form-annotator level** — purely syntactic, no binding resolution, no macro expansion, no evaluation. That keeps these asks consistent with lispexp's reader-only scope (lispexp ADR-0001).

---

## P1 — Polyglot definition annotation

**What.** Bundled definition-form recognition for every dialect, not just Emacs Lisp: given a top-level (or nested) form, report that it *is* a definition, its **name**, its **kind**, and its role slots (name / arglist / docstring / body).

**Why.** lisplens's Lens / Outline ([ADR-0013](adr/0013-one-canonical-result-terse-text-default-json-opt-in.md)) and Structural address ([ADR-0016](adr/0016-structural-address-is-an-s-expression-hash-shorthand.md)) are polyglot. The Outline (`hash line kind name`) requires knowing "is this a definition, and what is its name/kind" for Common Lisp, Scheme, Clojure, etc.

**Current lispexp.** `annotate.rs` provides only `emacs_lisp_builtins()` and an Emacs-focused `harvest_source`. The `Registry` / `FormSpec` / `Role` machinery is general, but no other dialect is populated.

**Proposed.** Bundled `Registry` builders per dialect, and a `Dialect → default Registry` accessor:
- Common Lisp: `defun`, `defmacro`, `defvar`, `defparameter`, `defconstant`, `defclass`, `defgeneric`, `defmethod`, `defpackage`, `defstruct`, `define-condition`, …
- Scheme: `define`, `define-syntax`, `define-record-type`, `define-library`, `define-values`, …
- Clojure: `def`, `defn`, `defn-`, `defmacro`, `defmulti`, `defmethod`, `defprotocol`, `defrecord`, `deftype`, `ns`, …
- Ideally the remaining dialects too (Fennel, Hy, Janet, LFE, Phel, …).

---

## P2 — Method qualifier + specializer exposure (Dispatch signature)

**What.** For generic-function methods, model the optional **qualifier** slot and expose the **specializer tokens** (verbatim), so a caller can build a stable per-method handle.

**Why.** lisplens disambiguates same-named methods by a syntactic **Dispatch signature** ([ADR-0009](adr/0009-structural-falls-back-to-line-hash-dispatch-signature-progressive.md)) — e.g. `cl-defmethod foo :around ((x integer)) …` → `foo :around (integer …)`; Clojure `(defmethod area :circle …)` → dispatch value `:circle`. Read as tokens only; never resolved to types.

**Current lispexp.** `cl-defmethod` is registered as `[Name, Arglist]` (`annotate.rs`) — it captures the name but mis-handles the optional qualifier (it would treat `:around` as the arglist) and does not expose specializers.

**Proposed.** A form-spec shape allowing an optional qualifier position and a "specialized arglist" role whose specializer sub-tokens are retrievable, covering the `cl-defmethod` / CL `defmethod` / `defgeneric` family and Clojure `defmethod` dispatch values.

---

## P3 — Indent-spec exposure

**What.** Expose per-symbol **indent metadata** (an Emacs `lisp-indent-function` / `declare` indent-style spec) through the registry.

**Why.** lisplens owns formatting via a native, spec-driven Rust indenter ([ADR-0011](adr/0011-formatting-is-lisplens-responsibility-pluggable-formatter.md)); it needs indent specs to indent faithfully without evaluating code. This is form-annotator-level metadata — the same class lispexp already harvests.

**Current lispexp.** The harvester already reads `declare` (indent, doc-string) signals for role inference, but does not expose an indent spec as consumable output.

**Proposed.** Attach an optional indent spec to `FormSpec` (bundled for well-known builtins; harvested from `declare (indent …)`), and expose it on the registry so lisplens's indenter can read it. Custom/overridden specs come from lisplens's Project profile.

---

## P4 — Parse-error identity + diff (validate-then-write)

**What.** Stable, comparable `ParseError` identities so a caller can diff the error set before and after an edit and reject only **newly-introduced** errors.

**Why.** lisplens's write safety ([ADR-0005](adr/0005-validate-then-write-reject-only-new-parse-errors.md)) is "never make a file's syntax worse": it re-parses after an edit and blocks only if the edit adds errors, using the pre-edit errors as a baseline (lispexp is fault-tolerant, so files may already contain errors).

**Current lispexp.** `parse` returns `Parsed { data, errors }`; `ParseError` positions exist, but there is no documented notion of stable identity for set-diffing across two parses.

**Proposed.** A `ParseError` shape (kind + normalized position) suitable for equality/diffing, and — nice-to-have — an incremental or region-scoped re-parse so validation stays cheap on large files.

---

## P5 — Line/byte index utility

**What.** A public index mapping byte offset ↔ (1-based line, column) and line number → byte range, computed once per source.

**Why.** Line-hash mode ([ADR-0006](adr/0006-mode-first-command-surface-with-batch-edits.md), [ADR-0013](adr/0013-one-canonical-result-terse-text-default-json-opt-in.md)) is line-centric (hashline-style `[path#FILEHASH]` + `LINE:hash|content`) and diagnostics report line/column; lisplens needs line↔byte mapping independent of the datum tree.

**Current lispexp.** `Span` is byte-only; a 1-based line is attached per `Datum`; column is "derived on demand" (`span.rs`). There is no whole-file line index exposed.

**Proposed.** A public `LineIndex` (or equivalent) over a `&str`, with `offset_to_line_col`, `line_col_to_offset`, and `line_range(n)`.

---

## P6 — Manifest-format coverage (zero-config scanning)

**What.** Ensure the reader can parse the repo-local manifests lisplens scans for zero-config inference.

**Why.** lisplens infers dialect / source roots / dependencies by parsing manifests with lispexp itself ([ADR-0015](adr/0015-zero-config-scans-repo-local-manifests.md)): `.asd` (CL), `Cask` / `Eask` / `Eldev` (elisp), `Akku.manifest` (R6RS), `clpmfile` (CL), `info.rkt` (`#lang info`, Racket reader), and `deps.edn` (**EDN**).

**Current lispexp.** Dialect presets cover the Lisps involved, but there is **no EDN preset**; `deps.edn` is EDN (a data subset of Clojure), not full Clojure.

**Proposed.** Either an `Options::edn()` preset, or documented confirmation that `Options::clojure()` reads `deps.edn` safely (tagged literals `#:`, no reader functions). Also confirm `#lang info` files parse as Racket data (the restricted-grammar interpretation stays lisplens's concern).

---

## P7 — Code-vs-data aware traversal helper

**What.** A convenience walker that yields each subtree with a **code / data** flag.

**Why.** lisplens's Project search is "syntactic, code-vs-data aware" ([ADR-0010](adr/0010-edits-single-file-atomic-discovery-project-wide.md)): descend into code, skip quoted data. lispexp already models this (quote → data; quasiquote → data except nested unquote), so lisplens should not reimplement the flip logic.

**Current lispexp.** Quote/quasiquote structure is preserved as `Prefixed` datums; classification is documented but there is no ready-made traversal that surfaces the flag.

**Proposed.** An iterator/visitor over a `Parsed` tree that tags each node code-vs-data per the documented rules.

---

## Priority summary

| Item | Drives | Blocker for |
| --- | --- | --- |
| P1 Polyglot definition annotation | ADR-0013, ADR-0016 | The polyglot Outline/Lens — core value |
| P2 Method qualifier/specializer | ADR-0009 | Stable method handles (else Mode fallback) |
| P3 Indent-spec exposure | ADR-0011 | Native spec-driven formatter |
| P4 Parse-error diff | ADR-0005 | validate-then-write safety |
| P5 Line/byte index | ADR-0006, ADR-0013 | Line-hash mode + diagnostics |
| P6 Manifest coverage (EDN) | ADR-0015 | Zero-config manifest scan |
| P7 Code-vs-data walker | ADR-0010 | Project search accuracy |

P1 and P2 are the true blockers for Structural mode's value; P5 unblocks Line-hash mode; the rest are enhancements lisplens can partially work around in the interim (e.g. hashing raw span bytes, computing its own line index) but wants upstreamed.
