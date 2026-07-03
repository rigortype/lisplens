# Clojure indentation — survey toward a native lisplens engine

Status: research note (no code yet). Feeds the decision to give Clojure a
*faithful* native indenter instead of riding the generic Emacs Lisp fallback
(`engine_for` → `Engine::Elisp`; `has_native_engine` is false for Clojure, so
today it reindents only on an explicit `format`, never on a Structural edit).

Sources surveyed: the community **style guide** (guide.clojure.style), **clojure-mode**
(Emacs), **clojure-ts-mode** (Emacs/tree-sitter), **cljfmt** (weavejester), **cljstyle**
(greglook).

## Headline finding: the ecosystem has converged on one model

All four *tools* now describe indentation with the **same two rule types**,
`:inner` and `:block`, over a **near-identical default table**:

- **cljfmt** introduced `:inner`/`:block`.
- **cljstyle** descends from cljfmt (adds `:stair`, unused by default).
- **clojure-ts-mode** adopts cljfmt's `:block`/`:inner` verbatim as its rule data.
- **clojure-mode** `master` now *authors* specs in the modern `(:block N)` /
  `(:inner D)` tuple format too (internally converting to its legacy
  integer/`:defn`/positional-list representation that the backtracking engine
  consumes).

So there is effectively **one Clojure indentation model** to port, not five. The
style guide (guide.clojure.style) is normative *prose* for the same model — it
mandates "semantic indentation" (the Emacs/lisp philosophy: body forms get a fixed
+2, plain calls align under the first argument) and explicitly does **not** hand
out a per-symbol table; the tools supply the table.

### The one real fork: semantic vs fixed ("Tonsky") style

Orthogonal to the table, there are two whole-file styles, and the community is
genuinely split (the guide says so outright):

- **semantic** (default everywhere): body-forms +2, everything else aligns under
  the first argument — driven by the `:inner`/`:block` table.
- **fixed / Tonsky** (cljfmt's alternative, clojure-ts-mode's `'fixed`): *any*
  symbol-led list indents a flat +2; only data collections align. No per-symbol
  table needed. clojure-ts-mode ships this as `clojure-ts-indent-style 'fixed`.

Recommendation: build **semantic** first (it needs the table and matches the guide
default + clojure-mode); `fixed` is a cheap add-on later (a config flag, no table).

## The model, precisely

Indexing: within a list `(head a b c …)`, `head` is unindexed; `a` is **arg index
0**, `b` is index 1, … "Depth" is nesting relative to the form the rule is on.
Indent unit is **2** (`#(...)` fn-literals: 3, because the `#(` opener is 2 chars).

- **`[:inner D]`** / **`[:inner D idx]`** — a *constant* +2 body indent applied to
  elements at depth `D` below the form (optionally only within argument `idx`).
  `[:inner 0]` is the classic "whole body is body-indented" (`defn`, `fn`, `def`).
  The `idx`-restricted and deeper-`D` forms express nested bodies clojure-mode's
  single integer can't: `reify` = `[:inner 0] [:inner 1]` (method bodies one level
  down), `letfn` = `[:block 1] [:inner 2 0]` (fn bodies two levels down, only in
  the binding vector).
- **`[:block N]`** — hybrid. The first `N` args behave like a normal call
  (alignment); if the `(N+1)`-th arg *starts a line*, the rest of the tail
  switches to +2 body indent. This is the "N special args then body" pattern:
  `let`/`when` = `[:block 1]`, `do`/`cond`/`try` = `[:block 0]`, `condp`/`catch` =
  `[:block 2]`.
- **default** (no rule): plain lisp alignment — if an arg shares the head's line,
  continuation args align under the **first argument**; if the head is alone on its
  line, indent **+1** (collection-literal alignment). Threading macros `->`/`->>`
  have *no* table entry and use this default (clojure-ts-mode adds a special
  matcher aligning threaded stages under the previous stage; clojure-mode/cljfmt
  just arg-align).

clojure-mode's legacy encoding maps 1:1: integer `N` ⇔ `[:block N]`; `:defn` ⇔
`[:inner 0]`; positional/nested lists ⇔ the `[:inner D idx]` combinations. Its
extra knobs — `clojure-indent-style` (`always-align` | `always-indent` |
`align-arguments`) and `special-arg-indent-factor` (double-indent for the first `N`
special args) — modulate only the default/alignment path, not body forms.

## Default table — the consensus, with divergences flagged

Grouping by rule (symbols agree across cljfmt / clojure-ts-mode / clojure-mode /
cljstyle unless a ⚠ notes a divergence):

- **`[:inner 0]`** (defn-like, always +2 body): `def defn defn- defmacro defmethod
  defmulti defonce deftest fn bound-fn use-fixtures`; `reify` = `[:inner 0] [:inner
  1]`. ⚠ `fdef`: `[:inner 0]` in cljfmt/clojure-ts/cljstyle, but `[:block 1]` in
  clojure-mode.
- **`[:block 0]`** (whole tail body): `cond do delay future comment try finally
  alt! alt!! go thread`. ⚠ `with-out-str`: `[:block 0]` in cljfmt/clojure-ts/
  cljstyle, but clojure-mode has no entry → its `with-` regex makes it `[:inner
  0]`.
- **`[:block 1]`** (one special arg then body): `ns if if-not if-let if-some when
  when-not when-let when-some when-first while case cond-> cond->> let let* binding
  loop for doseq dotimes doto locking testing go-loop match struct-map defstruct
  with-open with-local-vars with-precision with-redefs extend`; with a nested
  inner: `letfn`=`[:block 1][:inner 2 0]`, `defprotocol`/`definterface`=`[:block
  1][:inner 1]`, `extend-protocol`/`extend-type`=`[:block 1][:inner 1]`
  (⚠ clojure-mode uses `[:inner 0]` for the second rule here).
- **`[:block 2]`** (two special args then body): `condp are catch`; `proxy`=`[:block
  2][:inner 1]`. ⚠ `as->`, `defrecord`, `deftype`: `[:block 2]`(+`[:inner 1]` for
  the deftype-likes) in cljfmt/clojure-ts/clojure-mode, but `[:block 1]` in
  cljstyle.
- **regex fallbacks** (all): `^def…` (not `default`) → `[:inner 0]`; `^with-` →
  `[:inner 0]`. Plus clojure-mode's `:`-headed forms use a keyword style.

The divergences are few and small (`as->`, `defrecord`/`deftype`, `fdef`,
`with-out-str`). Whichever oracle we pick, we adopt *that* oracle's table verbatim
and match it byte-exact; the divergences just mean cljstyle-vs-clojure-mode output
differs on those handful of forms.

## Fit with lisplens' existing formatter

The `:inner`/`:block` algorithm is a **pure function of the s-expression tree** —
no tree-sitter, no Emacs buffer required. Everything it needs, lisplens' reader
already exposes:

- the enclosing form's open-delimiter column;
- the head symbol (namespace-stripped: `clojure.core/when` → `when`);
- the **value-only** child index of the line being indented (comments, whitespace,
  `#_` discards, and `^meta` must **not** consume an index slot — this is the main
  porting hazard, same "logical sexp" idea the CL/Scheme engines already handle);
- a parent chain up to depth 3 (`clojure-max-backtracking`) for `[:inner D]`.

Integration is the same shape as the CL and Scheme engines (ADR-0031):

1. add `Engine::Clojure`, route `Dialect::Clojure` in `engine_for`, add it to
   `has_native_engine` (so `.clj/.cljs/.cljc/.edn` auto-reindent on edit);
2. new `src/format/clojure.rs` implementing the `:inner`/`:block` resolver + the
   default table (bundled in-crate, like the Scheme table in `scheme.rs`);
3. reuse the shared driver, `Cols` column arithmetic, touched-region masking, and
   the reindent invariant unchanged.

Note this is Clojure-family only (`edn` is data — likely default/collection
indentation, no macro table). Fennel/Janet/Hy/LFE stay on the generic fallback.

### The oracle question — the one thing that differs from prior engines

Every existing engine ports an *Emacs* indenter and validates byte-exact against
**Emacs** via the fidelity harness. Clojure has **no bundled Emacs indenter**, so
the oracle is a choice. Environment on this machine: **Emacs present, no Java
runtime** (the `clojure` CLI is installed but also needs a JVM), and **`cljfmt`
0.16.4 is installed and runs** — it is a native GraalVM binary, so it needs no
Java. Verified it reindents correctly (`defn`/`when` bodies → +2). Options:

- **cljfmt CLI** *(recommended, now available)* — the canonical Clojure formatter
  developers actually run, and the **origin of the `:inner`/`:block` model** we are
  porting, so matching it *is* matching the standard. Pure, reproducible,
  data-driven; no Emacs package, no syntax-table coupling. Isolate indentation with
  a config that disables its other passes:
  `{:remove-surrounding-whitespace? false :remove-trailing-whitespace? false
    :insert-missing-whitespace? false :remove-consecutive-blank-lines? false
    :indentation? true}` — then `cljfmt fix` changes only indentation, apples to
  apples with lisplens' reindent. New harness path (run a binary, diff), but
  simpler than the Emacs batch harness.
- **Emacs + clojure-mode** — consistent with the existing harness; clojure-mode
  `master` implements the same `:block`/`:inner` model. Cost: a third-party Emacs
  package to install, and its arg-counting leans on Emacs' Clojure syntax table.
- **clojure-ts-mode** — same model, tree-sitter; awkward as a batch oracle.

With cljfmt installed and native, the friction argument that favored clojure-mode
is gone: **cljfmt is the recommended oracle** — it is both the real-world standard
and a clean data-driven target, and we port *its* default table verbatim.

## Recommendation

1. Port the **semantic `:inner`/`:block` model** — the ecosystem's converged
   standard — as `Engine::Clojure`, table bundled in `src/format/clojure.rs`.
2. **Oracle: `cljfmt fix`** with a non-indentation-passes-disabled config; add a
   Clojure fidelity-harness path that diffs against it. (cljfmt is the model's
   origin, so this targets the true standard, Java-free.)
3. Ship **semantic** first; add the **fixed/Tonsky** flag afterward (no table).
4. Scope to the Clojure family (`.clj/.cljs/.cljc`); `edn` = collection indent only.
5. Write an ADR for the engine — it introduces the first **non-Emacs-origin rule
   model** (`:inner`/`:block`) and the first **non-Emacs oracle** (cljfmt).

Adopt cljfmt's default table verbatim (`indents/clojure.clj`); the cljstyle/
clojure-mode divergences on `as->`/`defrecord`/`deftype`/`fdef` then simply don't
apply — cljfmt is the reference.
