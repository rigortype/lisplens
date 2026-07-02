# Dispatch signature in the Outline: readability, not addressing

The Structural Outline shows a method's **Dispatch signature** (ADR-0009) — verbatim qualifier(s), a Clojure dispatch value, and specializer tokens — extracted via lispexp's `Role::{Qualifier, DispatchValue, SpecializedArglist}` and `specialized_params`.

Because addressing is **hash-first** (ADR-0018), the anchor `line:hash` already identifies each method uniquely, so the signature is a **readability aid, not an addressing key**:

- In **terse text** it is appended after the name. Under hash addressing the name column is free-form (nothing parses it mechanically), so ADR-0013's "name is the last column" rule is preserved — e.g. `… cl-defmethod area :circle` / `… cl-defmethod foo :around (integer)`.
- In **JSON** it is a structured `signature` field.
- Non-method definitions have no signature.

## Status

accepted

## Consequences

- An agent can tell same-named methods apart from a single read.
- The name column stays free-form because addressing uses the hash, not the name.
