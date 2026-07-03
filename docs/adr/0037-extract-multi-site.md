# `extract --all` — multi-site extraction

## Context

ADR-0034 pulls the form at an anchor into a new function and replaces that one occurrence with a call; ADR-0035 generalizes the selection to a run of siblings; ADR-0036 lets the caller pick the definition head. All three replace a **single site**. A recurring shape they do not address: the *same* boilerplate appearing verbatim at many sites — a repeated guard `(unless conn (error "no connection"))`, a repeated setup call — that the caller wants hoisted **once** and called everywhere. Today that is N separate `extract` calls (only the first of which can create the def) or a hand-written `rewrite`.

## Decision

Add an optional **`--all`** flag to `extract` (MCP `all: true`): extract **every occurrence structurally equal to the anchored selection** into one new function, replacing each occurrence with the call. Absent the flag, `extract` is single-site exactly as ADR-0034/0035/0036.

```
lisplens extract <file> <anchor> <name> [param...] [--count N] [--kind HEAD] [--all]
```

- **What counts as "the same".** Structural equality modulo formatting — the existing `struct_eq` (ADR-0033): whitespace and comments are ignored, leaves compare literally, no sugar/number/case normalization. This is the same match relation `rewrite` uses, so the surprise surface is one the caller already knows.
- **Scope.** The whole file, like `rewrite`. Every structurally-equal, non-overlapping occurrence is a site (`keep_outermost`: a self-similar nesting keeps the outermost, and the inner copy vanishes inside the call that replaces the outer).
- **Placement.** The def is inserted once, before the top-level form enclosing the **earliest** site; every site (that one included) is replaced by `(NAME PARAMS)`.
- **Orthogonal to `--kind`.** `--kind` still chooses the head; `--all` only changes multiplicity.

### Sites must be identical *including* their arguments — no generalization (ADR-0003 ceiling)

`--all` does **not** anti-unify. Two occurrences that differ in any sub-term (`(* x 2)` vs `(* y 2)`) are *not* structurally equal, so only the ones identical to the anchored selection are sites. The consequence: `params` do **not** generalize across sites — because every site is textually identical, the same param symbols already appear at each, and the identical call `(NAME PARAMS)` replaces each. The param list still names the def's formals and the call's arguments, but it introduces no per-site variation. Extracting occurrences that differ in an argument is anti-unification / free-variable inference — deferred, and the harder move that actually crosses the semantic ceiling.

This keeps `--all` **parse-safe by construction**: identical source text ⇒ identical call text ⇒ the same guarantee as single-site extract. What lisplens does *not* verify — that a shared symbol denotes the same binding at every site, that no site has a context-dependent non-local exit — is the caller's assertion, exactly the ADR-0003 contract governing single-site `extract`, `rewrite` templates, and `--kind`.

### `--all` composes with `--count` (multi-site blocks)

`--all` applies to whatever the selection is. For `count == 1` a site is any single node structurally equal to the anchored form (the whole-tree `for_each_node` walk). For `count > 1` the selection is a run of N contiguous siblings (ADR-0035), so a site is any **window of N contiguous siblings** — in any sibling group, at any depth — each structurally equal to the corresponding form of the anchored run. Both feed the same site list; only the site-finding differs (single-node walk vs. sliding sibling window). A pattern run that could match two *overlapping* windows in one group keeps the earlier and drops the overlap (`keep_outermost`), because two overlapping runs cannot both be spliced.

## Status

accepted — implemented in `src/refactor.rs`: `extract_multi_site` finds sites with `struct_eq` — over `for_each_node` for a single form, over a new `for_each_sibling_group` sliding window for a run — dedups with the outermost, non-overlapping rule, then reuses the `rewrite`-style multi-edit splice to insert the def once (before the earliest site's enclosing top-level form) and replace each site with the call. The single-site path (`extract_block_into_function`) is the `all = false` case and shares the splice/reindent/validate tail. Wired through CLI (`--all` in `parse_extract_opts`) and MCP (`all` boolean).

## Consequences

- Reuses proven pieces — `struct_eq` + tree walk (rewrite), the outermost-non-overlapping dedup (rewrite), `def_form`/`call_form` (extract) — so the new surface is small and the match semantics are already documented.
- Zero *other* sites is not an error: `--all` on a form that appears once degrades to plain single-site extract, and the reported site count says so.
- `--all` composes with both `--count` (multi-site runs) and `--kind` (the emitted head); the three knobs are orthogonal.
- Still deferred (ADR-0034/0035/0036 list, minus block, kinds, and now multi-site): free-variable inference / anti-unification (the generalizing multi-site, where sites differ in an argument), multi-*file* extraction, non-default placement, and the distinct fold-repeats-into-a-loop transform.
