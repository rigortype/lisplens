# `extract --also` — generalizing multi-site extraction (anti-unification)

## Context

ADR-0037's `--all` folds occurrences that are **structurally identical** (`struct_eq`) into one function. The recurring shape it does *not* address: the same code appears at several sites but differs in a few sub-terms — a validation guard on a different field, an API call to a different endpoint, an arithmetic step with a different constant. A human extracts a **parameterized** helper, reading the varying sub-terms as arguments. This ADR automates that with **anti-unification**: given a set of occurrences, compute their least general generalization — the common skeleton is the function body, the positions that vary become parameters, and each site's call passes that site's own sub-terms.

This is the first `extract` opt-in that **infers structure** rather than replaying it verbatim, so the design is deliberately conservative about *what* it generalizes and *which* sites it touches.

## Decision

Add **`--also ANCHOR`** (repeatable; MCP `also: [anchor, …]`) to `extract`. The primary `anchor` plus every `--also` anchor are the **sites**; lisplens anti-unifies them into one function and replaces each site with its own call.

```
lisplens extract <file> <anchor> <name> [param...] [--kind HEAD] --also <anchor> [--also <anchor> …]
```

### Explicit sites, not discovery

The sites are exactly the anchors the caller names — lisplens does **not** search the file for same-shaped forms. This is the deliberate safety choice for an operation that crosses the ADR-0003 ceiling: the caller has looked at each occurrence and asserts it is foldable, and the parameter set is determined only by the sites chosen (anti-unification's parameter count grows with site heterogeneity, so precise site selection is what keeps the output clean). Skeleton auto-discovery is a possible later opt-in, once the core is proven.

### What is generalized — standard anti-unification, list-structured

Compare all sites position-by-position:

- Where all sites are **structurally equal** (`struct_eq`) → kept verbatim (part of the skeleton).
- Where all sites are **lists of the same delimiter and arity** → recurse into each child.
- At the **first point of divergence otherwise** → the whole differing sub-terms become one **parameter** (a "hole"); each site contributes its own sub-term as that argument. Holes do not nest and are ordered left-to-right in the skeleton.

Two guards make the result trustworthy, both **refusals** (`NotGeneralizable`, no write):

- **Operators must agree.** The head (first element) of a list is never generalized — if two sites' operators differ (`(foo …)` vs `(bar …)`), the extraction is refused rather than passing an operator as a first-class value (wrong for macros/special forms).
- **A common skeleton must exist.** If the sites share no fixed structure (the whole form would be a single hole), the extraction is refused — that is not an extraction, just an alias.

Only **list** nodes are recursed into; any other differing node (atoms, quoted/prefixed data, vectors of unequal shape) becomes a whole hole rather than being taken apart. This keeps generalization out of quoted data internals and off improper-list tails (differing tails are refused).

### The ceiling is held, not crossed deeper — no binding analysis

Anti-unification generalizes **only what differs across the given sites**. It performs **no binding analysis**: a symbol that is *common* to every site (a free variable of the selection, like a captured `conn` or `user`) is baked into the body unchanged, exactly as ADR-0034 single-site extract bakes in free locals. Whether such a symbol is a global or is identically bound at every call site is the caller's assertion — the same contract, not a deeper one. What lisplens adds is purely syntactic (which sub-terms vary), so it stays parse-safe by construction: the body and every call are built from verbatim source spans.

The residual hazard is that a *differing* sub-term is actually a binding occurrence (a `let`/`dolist` variable that happens to differ across sites), which anti-unification would wrongly parameterize. Because sites are **explicit**, the caller has seen each and asserts the varying positions are value expressions — the ADR-0003 division of labor. Free-variable *inference* (parameterizing the common free locals too) still requires binding analysis and remains deferred.

### Parameters and the body

The number of parameters is the number of holes. Names are **generated** (`arg1`, `arg2`, … skipping any symbol present in the skeleton, so a generated name never captures a body symbol); a caller-supplied `[param…]` list overrides them positionally when its length matches the hole count, and is a length-mismatch error otherwise. The body is the primary site's verbatim text with each hole's sub-span replaced by its parameter name; each site's call is `(name arg…)` with that site's sub-terms. The def is inserted once, before the earliest site's enclosing top-level form; unsupported dialects and (for now) `--count > 1` sites are refused.

## Status

accepted — implemented in `src/refactor.rs`: `extract_generalized` resolves every anchor, runs `anti_unify` (the position-wise walk above) to collect holes, generates parameter names, renders the body by splicing the primary site's text, and builds a per-site call; it shares the `finish_extraction` splice → reindent → validate → write tail with the single- and identical-multi-site paths (now taking per-site call text). Wired through CLI (`--also`, repeatable) and MCP (`also` array). `NotGeneralizable` is the new refusal.

## Consequences

- Reuses `struct_eq` and the extraction tail; the only genuinely new logic is the anti-unification walk and body rendering.
- Explicit sites mean predictable output and no over-matching; the cost is that the caller supplies each anchor.
- `--also` is a distinct site-selection mode from `--all`; combining them (or `--also` with `--count > 1`) is refused rather than silently reconciled.
- Still deferred (ADR-0034/0035/0036/0037 list, minus block, kinds, multi-site, and now generalization): free-variable **inference** (binding analysis), skeleton auto-discovery, multi-*file* extraction, generalizing over sibling *runs* (`--also` + `--count`), and non-linear hole merging (reusing one parameter for positions equal at every site).
