# Shared core edit verbs with structural-only extras

The three core edit verbs — Replace, Insert, Delete — are shared across both Structural mode and Line-hash mode with identical names and meaning, so an agent learns one set of intents regardless of how it addresses code. Structural mode adds structure-specific operations that have no Line-hash equivalent: Wrap (enclose a node in a new form), Splice (remove an enclosing form and hoist its children), and Rename (see ADR-0003 for its deliberately narrow, occurrence-based semantics).

This bounds ADR-0001: the two modes are separated in their addressing and read output, **not** in their core edit vocabulary.

## Status

accepted

## Consequences

- Line-hash mode implements only the shared core (Replace / Insert / Delete).
- Wrap / Splice / Rename are Structural-mode-only.
