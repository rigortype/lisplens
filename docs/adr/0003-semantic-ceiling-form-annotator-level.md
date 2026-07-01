# Semantic ceiling: form-annotator level, no scope or macro analysis

lisplens understands only as much of each dialect's semantics as lispexp's Form annotator provides — the roles of a definition form (name, arglist, docstring, body). It does **not** resolve variable bindings or scopes, and it does **not** expand macros. The product's stance is "S-expression editing that understands just a little of the language's semantics" — no more.

Consequently, Rename is occurrence-based within an anchored subtree, not a scope-aware refactor.

## Status

accepted

## Considered Options

- **Scope-aware rename / macro-expansion-aware refactoring** — rejected. Resolving bindings or expanding macros widens the semantic gap (and the maintenance surface) across the polyglot dialect set far beyond what a reader-only backend can honor, and is inconsistent with lispexp's reader-only scope (lispexp ADR-0001).

## Consequences

- Structural edits are shape- and role-accurate but not binding-accurate; agents must not assume Rename respects shadowing or macro-introduced names.
- Refactors that genuinely require language semantics are out of scope, or belong to a future, separate layer above lispexp.
