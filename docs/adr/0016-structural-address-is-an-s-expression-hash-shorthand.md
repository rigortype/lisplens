# Structural address is an S-expression; hash shorthand is first-class

A Structural address is expressed as an **S-expression**, not a delimiter-joined string. Lisp symbols admit `:`, `/`, `.`, `+`, `*`, and more, so any `name:role/index` scheme collides with real symbol names. Putting the name in a **string literal** inside an S-expression isolates it completely — even names containing spaces or pipes.

The address carries optional disambiguation (`:nth` / `:kind` / `:dispatch` for methods, per ADR-0009), a role (name / arglist / docstring / body), and child indices for descent. A node's **4-hex content hash from the latest read is a first-class shorthand anchor** — the token-cheapest reference, identical to the drift mechanism (ADR-0007, ADR-0008).

The read output uses **space-separated columns** (`hash line kind name`, indentation for nesting; ADR-0013). Space is the one character that cannot appear unescaped in a Lisp symbol, so it is a safe column separator; the S-expression string form is the escape hatch for the rare name that needs it. Line-hash reads keep hashline's `[path#FILEHASH]` + `LINE:hash|content`.

Exact keyword spellings (`:nth` / `:dispatch` / `in` / `def`) are refinable.

## Status

accepted

## Considered Options

- **Delimiter-joined string `name:role/index`** — rejected: collides with permissive Lisp symbol characters. Space alone could separate (symbols must escape spaces), but a structured S-expression is more robust and native to the tool.
- **Hash-only addressing** — rejected as the sole scheme: cheapest, but gives no stable by-name or human-readable address across reads. Kept as the shorthand.

## Consequences

- Addresses are themselves S-expressions, which lisplens already parses with its own backend.
- Two ways to point: a hash shorthand (cheap, per-read) and an S-expression name address (stable, readable).
