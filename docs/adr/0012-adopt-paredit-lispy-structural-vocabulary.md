# Adopt paredit/lispy structural-editing vocabulary for Structural mode

Structural mode's structure-specific operations use the established **paredit / lispy** vocabulary — familiar to Lisp editors (and to agents trained on Lisp code), self-descriptive (no internal backend names), and all purely syntactic, so within the semantic ceiling (ADR-0003). The curated initial core is:

**wrap, splice, raise, slurp (forward/backward), barf (forward/backward), split, join** — plus **Rename** (a lisplens-specific, occurrence-based operation that paredit/lispy lack). **move / clone / convolute** are a deferred second tier.

## Refines ADR-0002

This refines and supersedes the structural-extras list in ADR-0002 (which named "Wrap / Splice / Rename"). splice and raise are now distinguished precisely:

- **splice** removes an enclosing list's delimiters but keeps **all** contents — `(foo (bar baz) quux)` → `(foo bar baz quux)`.
- **raise** replaces a node's parent with the node and **discards siblings** — `(when cond x)` → `x`. (An earlier draft mislabeled this as "splice with drop_leading".)

slurp / barf are **boundary moves** (adjust one delimiter) and are the most token-efficient restructuring primitives, directly serving the product goal.

## Insert positions and formatting

The shared-core Insert accepts positions: sibling before / after, and body start / end / index, including into an empty body. Formatting after any operation is handled by lisplens's formatter (ADR-0011); agents never manage whitespace.

## Status

accepted

## Consequences

- Learning cost is low for anyone who knows paredit/lispy.
- The deferred tier-2 operations (move / clone / convolute) can be added later without changing the model.
