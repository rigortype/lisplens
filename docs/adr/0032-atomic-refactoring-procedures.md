# Atomic, parse-safe refactoring procedures: rename, inline, extract, check

## Context

The agent-skill benchmark (`docs/notes/20260704-skill-benchmark-agent-edit-procedures.md`)
ran lisplens-with-a-skill against a strong plain-tools baseline (grep/sed/Read+Edit)
on real Lisp edits. What it tells us about the *tool surface*:

- **Correctness was a tie.** A capable model with plain tools is careful. lisplens's
  value is that the safe, symbol-exact, parse-validated path is the *default and
  effortless* one — not that the baseline fails.
- Agents converged on one **multi-step idiom** for every symbol edit:
  `find`/`struct read` (map) → `refs` (enumerate symbol sites) → `line edit` /
  `struct edit` (patch) → `refs` (re-verify). That is 3–4 steps to do what
  `sed`/`perl -i` do in one — the baseline's ergonomic edge, and the thing that
  undercuts the "safety is worth a deliberate reach" pitch.
- The refactors agents actually performed are **higher-level than the primitive
  verbs** (ADR-0012). Iteration 4 was a `defsubst` **inline-expand ⇄ re-inline**
  round-trip, hand-assembled as a 10-op `line edit` batch after locating the def
  and its call sites.
- `refs` (code/data-tagged symbol resolution) is the differentiator every time:
  each baseline rename/count had to compensate for grep's text-blindness — subtract
  comment lines, or engineer a `(?!-)` lookahead against sibling symbols. But `refs`
  covers only *parse-tree* occurrences; comment/string/docstring mentions need a
  separate pass (the trap eval was "26 code + 4 comment refs" — two patches).

The primitives are sound; the gap is that the recurring **workflows** — rename a
symbol, inline a definition, fold a repeated pattern into a call — are left to the
agent to assemble step by step, which taxes exactly the safety-over-ergonomics
value story.

## Decision

Add a tier of **atomic, parse-safe, self-verifying refactoring procedures** that
internalize these idioms. Each one:

- resolves targets by lispexp's structural / symbol model, never by text;
- applies all sites in a single edit through the existing pipeline — drift gate
  (ADR-0017) → splice → dialect reindent (ADR-0025/0031) → validate-then-write
  (reject new parse errors, ADR-0005) → atomic write;
- returns a **post-condition summary** (e.g. old-symbol residual 0, N sites
  rewritten, siblings untouched) so the agent needs no separate `refs` re-verify.

CLI verbs mirror into MCP tools (ADR-0020). Members, simplest first:

1. **`rename <old> <new>`** — symbol-exact rename across a file. Collapses the
   `refs → line edit batch → refs` idiom into one call, and is the direct, *safe*
   competitor to `perl -i -pe 's/\bX\b(?!-)/Y/g'`: `refs`-grade resolution never
   touches siblings. Optional `--comments`/`--strings`/`--docstrings` also update
   textual mentions (word-boundary), reported separately — closing the two-patch
   gap. (Widens `structural::rename`, today a single anchored subtree, ADR-0003.)

2. **`inline <name>`** — replace each call of a function / `defsubst` / simple macro
   with its body, substituting parameters (the benchmark's inline-expand). Safe only
   for *substitutable* bodies — see hygiene below.

3. **`extract` / `fold <pattern> <call>`** — the inverse: find sub-forms matching a
   structural pattern and replace them with a rewrite (the benchmark's re-inline,
   generalized). Requires an s-expr pattern language — see below. This member
   generalizes past fold-to-a-call: the same pattern→rewrite engine covers other
   structural refactors, e.g. **removing a variable guard**
   (`(when flag (foo))` → `(foo)`), unwrapping a `progn`, or `(if c a nil)` →
   `(when c a)`. The pattern language is the shared substrate; each such rewrite
   is a `(pattern, template)` pair over it.

4. **`check`** — standalone parse/validate: report lispexp `ErrorKind`, non-zero
   exit on errors. Cheap; replaces the `emacs -Q --batch check-parens` the baseline
   shelled out to repeatedly for a guarantee lisplens already gives on every edit.

## Hard problems (open — they gate the richer members)

- **Inline hygiene.** Naive body-substitution is unsafe when an argument is used
  more than once (duplicates side effects), evaluates with a fixed order, or a body
  free variable is captured at the call site. A safe `inline` must either restrict
  to trivial bodies (each parameter used once, side-effect-free arguments — which
  covers `defsubst` accessors like `(nth 3 (syntax-ppss))`) or introduce
  `let`-bindings to preserve single-evaluation and order. Start restricted, and
  **refuse** (never corrupt) the unsafe cases with a clear reason.
- **Pattern language for `extract`.** Needs metavariables (`?x` binding a sub-form),
  non-linear matching (the same metavar matches equal forms), literal-vs-wildcard,
  and structural equality modulo formatting. This is `el-search` / `comby`-class
  work; scope it as its own design step before building.
- **Cross-file scope.** Discovery is project-wide (`find`/`refs`) but edits are
  single-file (ADR-0010). A project-wide `rename`/`inline` needs multi-file
  atomicity (all-or-nothing) — a deliberate extension of ADR-0010, not a silent one.
- **Dialect coverage.** These refactors are dialect-agnostic where they lean on the
  parse tree; reindent-on-edit is Emacs Lisp + Common Lisp + the Scheme family
  (`has_native_engine`, ADR-0031), others stay verbatim (ADR-0027).

## Status

proposed

## Consequences

- The primitive verbs (ADR-0012) stay; these procedures are **compositions** over
  them plus the safety pipeline, so they add surface without new edit machinery —
  `rename` is `structural::rename` past a single subtree; `inline`/`extract` are
  span→edits like every other structural op.
- The value story matches the benchmark's conclusion (*sell safety, not tokens*):
  each procedure is one deliberate, safe reach a `sed` one-liner can't match on
  siblings/parens, and the built-in post-condition removes the agent's manual
  re-verify step.
- Ship order by cost/unblocking: **`check`**, **`rename`**, and **`inline`** have
  landed (`src/refactor.rs`); **`extract`** needs the pattern-language design
  first. `inline` ships the restricted-safe subset above (niladic direct
  substitution, params via `let`, everything unsafe refused with a reason).
- Re-benchmark the inline round-trip against an `inline`/`extract`-equipped build to
  quantify the step-count drop (iter-4 was a 10-op hand-assembled `line edit`).
