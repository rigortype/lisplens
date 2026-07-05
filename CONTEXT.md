# lisplens

A CLI and MCP tool that lets AI agents edit Lisp-family code (Common Lisp, Scheme, Emacs Lisp, Clojure, and other dialects) with minimal token cost — addressing code structurally or by hashed line anchors instead of resending whole files. Built on the [lispexp](../lispexp) reader.

## Language

**Structural mode**:
The editing surface that addresses code by its s-expression structure — a definition's name, a role within it (arglist, docstring, body), or a node path — resolved through lispexp's parse tree and Form annotator.
_Avoid_: sexp mode, form mode, AST mode

**Line-hash mode**:
The editing surface that addresses code by line number paired with a short content hash, in the style of hashline. Line-oriented, dialect-agnostic, with hash-based drift detection.
_Avoid_: textual mode, hashline mode, raw mode

**Anchor**:
A reference to an edit location that carries a content hash so drift can be detected and a stale edit rejected. Both modes anchor on a content hash and share one drift mechanism; they differ only in how the target is named — a line in Line-hash mode, a definition name / role / node path in Structural mode.
_Avoid_: reference, pointer, locator

**Drift detection**:
Rejecting an edit whose target has changed since the read it was based on, forcing the agent to re-read before retrying. A file-level hash guards the whole snapshot; a per-anchor hash guards each edit site.
_Avoid_: staleness check

**Lens**:
The structural read view lisplens produces of a file: by default an Outline, with any named definition expandable to reveal its inner structure or body on demand. The tool's namesake — you zoom the lens onto only the code you need.
_Avoid_: view, dump

**Outline**:
The compact map the Lens returns by default: one entry per definition (name, kind, line, hash) with bodies omitted, so an agent can grasp a file's shape and obtain edit anchors without paying for its contents.
_Avoid_: summary, index, table of contents

**Rename**:
A Structural mode operation that replaces every occurrence of a symbol within an anchored subtree. Deliberately occurrence-based, not scope-aware — it does not resolve bindings, shadowing, or macro-introduced names.
_Avoid_: refactor-rename, scope-aware rename

**Project profile**:
An optional, persisted per-project configuration file holding only what lisplens cannot infer on its own — e.g. a dialect/implementation override, or library load-path resolution. Zero-config is the default; the profile is an escape hatch, kept minimal, and read by both the CLI and the MCP server.
_Avoid_: config, settings, dotfile

**Batch**:
A set of same-mode edit operations submitted in one call, checked for drift against a single read snapshot and applied all-or-nothing after validation. The unit of token efficiency: one round-trip, one snapshot, one atomic write.
_Avoid_: transaction, changeset, patch set

**Patch**:
The terse-text rendering of a Batch for the CLI (`line edit` / `struct edit`): a `@ file-hash` header plus one op per line, with heredoc payloads. The MCP surface carries the same Batch as a JSON op array instead.
_Avoid_: diff, hunk

**Dispatch signature**:
The verbatim qualifier and specializer tokens that distinguish one method of a generic function from another — e.g. `cl-defmethod`'s `:around` qualifier and `((x integer))` specializers, or a Clojure multimethod's dispatch value. Used as a stable Structural mode handle that survives reordering. Read as syntax only; the tokens are never resolved to types, respecting the form-annotator-level semantic ceiling.
_Avoid_: type signature, overload signature

**Mode fallback**:
Structural mode's graceful degradation: when it cannot produce a stable, unambiguous handle for a target — an unsupported form, still-ambiguous methods, or an anonymous/nested node — it returns a Line-hash anchor for the same span plus a hint to switch to Line-hash mode, rather than failing. A one-way bridge (Structural → Line-hash), not a unification of the two modes.
_Avoid_: mode switch, downgrade

**Project search**:
A read-only, project-wide lookup that returns the locations of definitions or symbol occurrences (not their bodies), so an agent can find a target across files cheaply before editing it file by file. Syntactic only — it aggregates per-file Outlines and token matches and never resolves bindings.
_Avoid_: index, grep

**Structural address**:
How Structural mode names an edit target: an S-expression whose name component is a string literal (so any symbol characters — including `:` `/` `.` or even spaces — are safe), with optional disambiguation (`:nth`, `:kind`, `:dispatch`), a role (name/arglist/docstring/body), and child indices for descent. A node's 4-hex content hash from the latest read is a first-class shorthand anchor. Delimiter-joined string forms like `name:role/index` are rejected because Lisp symbols admit those characters.
_Avoid_: path, selector, locator

**Indent spec**:
The per-symbol metadata that drives how a form is indented — e.g. Emacs's `lisp-indent-function` or a macro's `declare` indent declaration — collected at the form-spec level. The formatter reads indent specs; it never evaluates code, so formatting stays within the semantic ceiling. A custom macro's indent spec is a Project profile persistence candidate.
_Avoid_: indent rule, style

## Structural operations

Structure-specific Structural mode operations, borrowing the established paredit / lispy vocabulary (all purely syntactic — within the semantic ceiling). Shared-core verbs (Replace / Insert / Delete) and Rename are defined above under Language.

**Wrap**:
Enclose a node or a contiguous sibling range in a new list. lisplens accepts an optional prefix so the result can be `(when cond <node>)`, not only `(<node>)`.

**Splice**:
Remove an enclosing list's delimiters, keeping all of its contents in the parent — `(foo (bar baz) quux)` → `(foo bar baz quux)`. Not to be confused with Raise.
_Avoid_: unwrap

**Raise**:
Replace a node's parent form with the node itself, discarding the parent's other children — `(when cond x)` → `x`.
_Avoid_: promote

**Slurp**:
Move a list boundary outward to swallow the adjacent sibling into the list — a boundary move, not a text rewrite. Has a forward and a backward direction. The cheapest restructuring primitive in tokens.

**Barf**:
Move a list boundary inward to expel the edge element out of the list. The inverse of Slurp; forward and backward.

**Split / Join**:
Split one list into two at a point / merge two adjacent lists into one.

## Refactoring procedures

Atomic, self-verifying compositions over the primitives + the safety pipeline (ADR-0032). `check` / `rename` / `inline` have landed; `rewrite` is designed (ADR-0033).

**Rewrite**:
The general structural pattern→template procedure — a "structural sed" (ADR-0033). Match sub-forms by an s-expr **Pattern** and replace each with a **Template**. Unlike Rename and Inline it is **not** behaviour-preserving (guard removal `(when flag (foo))` → `(foo)` drops the guard); lisplens guarantees only parse-safety and exact structural matching, never that the rewrite preserves meaning — the user asserts semantics.
_Avoid_: extract (reserved for the future "extract into a *new* function"), replace, codemod

**Pattern / Template**:
The two s-exprs of a Rewrite spec, parsed in the file's dialect. The Pattern is matched against the tree (literals matched structurally, Metavariables captured); the Template is emitted with each captured Metavariable substituted verbatim. Supplied over stdin (user-tag heredocs, optional `@ <file-hash>` drift gate).
_Avoid_: match/replace, LHS/RHS, rule

**Metavariable**:
A Pattern/Template hole `$name` that captures (and re-emits) the single form at its position; `$_` is a non-capturing wildcard. A literal `$`-symbol is escaped `$$`. Repeated occurrences must bind structurally-equal forms (non-linear matching).
_Avoid_: hole, variable, placeholder, wildcard (that is only `$_`)

**Sequence metavariable**:
`$name...` (≡ `$name ...`) — a Metavariable capturing a contiguous run of sibling forms (e.g. `(progn $body...)`). At most one per list.
_Avoid_: rest, splat, ellipsis, varargs

**Metavariable class**:
`$name:class` — a syntactic filter narrowing what a Metavariable matches: `any` / `atom` / `lit` / `sym` / `list`. The user's tool for duplication safety (e.g. `$n:atom` so a side-effecting call is not duplicated by a fold). Filter only — it does not select among Templates.
_Avoid_: type, constraint, guard, predicate

**Structural equality**:
Equality of two forms **modulo formatting**: recursive comparison of `DatumKind` ignoring `span`/`line` (so whitespace and comments do not matter), with leaf text compared literally (no reader-sugar, number, or CL-case normalization). The basis of literal matching and non-linear matching. (Distinct from `Datum`'s derived `==`, which compares spans.)
_Avoid_: deep equality, sexp equality, structural match

**Structural diff**:
A read-only observation that compares two forms — or two versions of a file — **modulo formatting** (on the Structural equality basis) and reports how the second differs from the first: which subforms were added, removed, or changed, as a tree. Its purpose is to let an agent see *how a unit's logic changed* between versions and focus attention there; pure reindentation or comment churn yields an empty diff. The inverse orientation of a Patch: a Patch *applies* edits to produce a new version, a Structural diff *observes* the difference between two given versions (it never writes). Exposed as the `diff` command.
_Avoid_: patch, delta, changeset, textual diff, line diff
