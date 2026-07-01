# Structural mode falls back to Line-hash; Dispatch signature is a progressive enhancement

When Structural mode cannot produce a stable, unambiguous handle — the form is not in lispexp's spec registry, several methods of a generic function cannot yet be distinguished, or the target is anonymous/nested — it does **not** hard-fail. It returns the Line-hash anchor for the same span together with a hint to switch to Line-hash mode (the **Mode fallback**). This is a **one-way bridge** (Structural → Line-hash), so ADR-0001's separation of the two modes still stands.

Consequently, method disambiguation by **Dispatch signature** (verbatim qualifier + specializer tokens; a Clojure multimethod's dispatch value) is a **progressive enhancement, not a prerequisite**. Until lispexp gains qualifier-aware method form specs and exposes specializer tokens, methods fall back to Line-hash; once it lands, methods gain first-class stable handles and the fallback fires only for genuinely ambiguous cases.

## Status

accepted

## Requires (lispexp)

- Qualifier-aware form specs for the `cl-defmethod` / `defmethod` family, and exposure of specializer / dispatch-value tokens. Today `cl-defmethod` is modeled as `[Name, Arglist]` (`src/annotate.rs`), which captures the name but not the optional qualifier or the specializers.

## Consequences

- Structural mode can ship incrementally; anything it cannot address stably degrades to Line-hash with a nudge, rather than blocking or guessing with fragile occurrence indices.
- The fallback produces real data on where Structural mode is insufficient and Line-hash mode wins — directly serving ADR-0001's best-practice-accumulation goal.
