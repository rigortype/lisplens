# Building a lisplens agent skill, and what edit procedures agents actually chose (2026-07-04)

Built a Claude Code **skill** that teaches an AI agent to reach for the globally
installed `lisplens` (v built 2026-07-04) instead of grep/sed/Read+Edit when
reading or editing Lisp, and benchmarked it: **with-skill vs. a no-skill
baseline**, same prompt, on real files. Four iterations of Claude runs plus one
cross-vendor run (DeepSeek V4 Flash over ACP). The headline is not the pass rate
(everything passed) — it's *how* each configuration chose to do the work. This
note records that procedural data, because it's the load-bearing evidence for
what the skill is actually worth.

Skill lives at `~/.claude/skills/lisplens/` (SKILL.md + references/patch-dsl.md).
Workspace + all run artifacts: `~/.claude/skills/lisplens-workspace/`.

## Setup

- **Baseline = no skill.** Same task prompt, told to use standard tools
  (Read/Edit/grep/sed) and *not* lisplens. Models the "before this skill existed"
  behavior of a capable agent.
- Each run gets a private working copy; edit tasks graded by re-parsing with
  lisplens and (where a canonical answer exists) byte-comparing to a reference.
- Fixtures escalated in realism: `mypkg.el` (54 lines, synthetic) → Emacs's
  `cc-engine.el` (16,733 lines, real) → `php-mode.el` round-trip refactor.

## The core finding

**Correctness was a tie — both configs passed 100% of every eval.** A modern
model with plain tools is careful: it greps to locate, reads only a region (never
the whole 16k-line file), and even recognizes a rename substring-trap and defends
against it with a lookahead regex. So the skill's value is *not* "baseline fails
without it." The value is:

- lisplens makes the **safe, symbol-exact, parse-validated** path the *default
  and effortless* one — no hand-crafted regexes, no separate verify step.
- The relative cost of the skill **shrinks as tasks get more complex**. On tiny
  single-site edits, reading SKILL.md is pure overhead (with-skill spent more
  tokens). On the multi-step php round-trip, tokens were a tie (+215) — and on the
  reverse-inline eval the *baseline* spent more, because it had to run
  `emacs --batch check-parens` itself for the safety lisplens gives by
  construction.

| Iter | Fixture / task | Pass (ws/base) | Δ tokens (ws−base) | Δ time |
| --- | --- | --- | --- | --- |
| 1 | mypkg.el (54 ln): add-arg / rename / locate | 100% / 100% | **+3.4k** | +17.5s |
| 2 | cc-engine.el (16.7k ln): locate / rename-local / docstring | 100% / 100% | **+3.3k** | +11.4s |
| 3 | cc-engine.el ×4 (adds symbol-rename trap) | 100% / 100% | **+4.5k** | +21.5s |
| 4 | php-mode.el defsubst inline-expand ⇄ re-inline | 100% / 100% | **+0.2k** | +24.1s |

## Per-agent edit procedures (the requested data)

What sequence of tools/commands each run chose. "ws" = with-skill, "base" =
baseline. Read the with/base pairs side by side — that contrast *is* the result.

### Iteration 1 — mypkg.el (synthetic, 54 lines)

| Eval | Cfg | Procedure chosen |
| --- | --- | --- |
| add optional arg | ws | `struct read` → `line read` (file-hash) → **`struct edit` replace** whole form (auto-reindent + parse-validate) |
| | base | `Read` whole file → `Edit` the one defun → `cp` |
| rename var `mypkg--cache` | ws | **`refs`** → got exactly 4 var sites, *not* the sibling functions `mypkg--cache-get/-put` → `line read` → **`line edit`** 4-op batch → `refs` re-verify |
| | base | `Read` → `Edit` 4 sites by hand → `grep` to confirm no stragglers |
| locate symbol | ws | `find` + `refs` + `struct read` |
| | base | `Read` whole file (cheap at 54 ln), answered from memory |

### Iteration 2 — cc-engine.el (real, 16,733 lines)

| Eval | Cfg | Procedure chosen |
| --- | --- | --- |
| locate def + count occurrences | ws | `find` (def @319) → `refs` → **37** (symbol-exact, code/data tagged; comment mentions excluded) |
| | base | `grep -n` / `grep -c` / `grep -o \| wc -l` → **40** (textual; includes 3 comment mentions). Different, defensible answer, but *not the same question* |
| rename local `prevstate` | ws | `struct read <fn>` (expand) → `line read` → **`struct edit` replace** whole form. Reindent **preserved the file's tabs** — clean diff |
| | base | `grep` → `Read` region → `Edit` |
| add docstring | ws | `struct read` → `line read` → **`struct edit` replace** (tabs preserved) |
| | base | `grep` → `Read` → `Edit` |

### Iteration 3 — cc-engine.el, 4 evals (adds the rename trap)

| Eval | Cfg | Procedure chosen | Cost |
| --- | --- | --- | --- |
| count code-only refs | ws | `find` → `refs` → **37 directly** (code-tagged) | 4 tools, 41s |
| | base | `grep -n` + `grep -oE\|wc` → 40, then **manually subtract 3 comment lines** → 37; `Read` to confirm def | 8 tools, 67s |
| rename local `prevstate` | ws | `find` → `struct read` → `refs` → **`struct edit` with the `rename` verb** (subtree-scoped) → **cleanest possible 2-line diff** (left the in-fn comment alone) | |
| | base | `grep` → `Read` → `Edit` (also updated the `PREVSTATE` comment) → `grep` verify | |
| add docstring | ws | `struct read` → `line read` → `struct edit` replace | |
| | base | `grep` → `Read` → `Edit` anchored on the unique signature line | |
| **rename symbol trap** `c-macro-cache` (26 real sites; siblings `-start-pos`×14 / `-syntactic`×12 / `-no-comment`×10 must survive) | ws | **`refs`** → 26 exact (code/data), siblings excluded *by construction* → `line read` → **`line edit`** (two patches: 26 code + 4 comment refs) → `find`/`refs` re-verify | 13 tools, 120s, 38k tok |
| | base | `grep` survey → **`perl -i -pe 's/\bc-macro-cache(?!-)/.../g'`** — a negative-lookahead to dodge the siblings → `grep`/`perl` verify | 3 tools, 37s, 30k tok |

The trap eval is the most instructive: **both got it perfect.** The baseline
avoided corruption only because it *recognized the trap* (the prompt warned it)
and hand-built a `(?!-)` lookahead. lisplens needed no such cleverness — `refs`
returns the symbol, never the substring. Warn a weaker model less, or omit the
warning, and the naive `sed s/c-macro-cache/…/g` corrupts 36 sibling sites.

### Iteration 4 — php-mode.el: defsubst inline-expand ⇄ re-inline (round-trip)

Bodies: `php-in-string-p`=`(nth 3 (syntax-ppss))`, `-comment-p`=`(nth 4 …)`,
`-string-or-comment-p`=`(nth 8 …)`, `-poly-php-html-mode`=`(bound-and-true-p
poly-php-html-mode)`; 10 call sites total. Grading = byte-identity to a verified
reference (expand → reference-expanded; re-inline → **original**).

| Eval | Cfg | Procedure chosen |
| --- | --- | --- |
| inline-expand (calls→bodies) | ws | `find` (4 defsubst in php.el) → `Read` php.el bodies → **`refs`** exact call sites → `line read` → **`line edit`** single 10-op patch |
| | base | `grep -nE` → `Read` → **`sed -i ''`** 4 global fixed-string subs → `grep` confirm → **`emacs -Q --batch check-parens`** to self-verify parse |
| re-inline (bodies→calls == original) | ws | `grep`+`Read` contexts → `line read` → **`line edit`** 10 replace ops — **explicitly chose line-mode "to keep surrounding lines byte-for-byte, no reindent"** → `cp` with hash-match check |
| | base | `Read` → `grep -nE` → **`sed -i ''`** 4 literal subs → **`emacs --batch` read-loop** parse verify → `cp`/`cmp` |

All four runs produced **byte-identical** output to the reference; the round-trip
was perfect in both configs.

### Cross-vendor — DeepSeek V4 Flash (OpenCode, over ACP)

Ran the iteration-3 trap rename (`c-macro-cache`) in a sandbox with SKILL.md +
references staged in-cwd. `model_verified: opencode-go/deepseek-v4-flash`.

- Procedure: **read the skill** → `refs` → 26 exact sites → `line read` →
  **`line edit`** single 26-op batch → `find`/`refs` re-verify.
- Independently checked: new symbol 26, old 0, siblings 14/12/10 intact, 0
  sibling corruption, parses. **Perfect**, same idiom Claude used.

That a flash-tier non-Claude model, handed only the skill files, lands the exact
safe procedure is the strongest portability signal: the skill encodes a workflow,
not Claude-specific tricks.

## What the procedure data tells us

1. **with-skill consistently converges on the same idiom:** `struct read`/`find`
   to map → `refs` to enumerate *symbol* sites → `struct edit` (form replace /
   `rename` verb) or `line edit` (verbatim batch) → re-verify with `refs`. Agents
   used it without hand-holding across 4 fixtures and 2 vendors.
2. **The `struct edit` vs `line edit` choice transferred.** SKILL.md frames
   struct=reindents, line=verbatim; in iter-4 the agent *reasoned aloud* that it
   picked `line edit` to preserve byte-identity. The distinction is doing work.
3. **`refs` is the differentiator that shows up every time.** Every baseline that
   counted or renamed used `grep`, and every time it had to compensate for grep's
   text-blindness — subtract comment lines, or engineer a lookahead. lisplens'
   code/data-tagged symbol resolution removes that whole class of manual step.
4. **Baselines are strong, so sell safety not tokens.** The benchmark refuted the
   original "token-efficient vs grep/sed" pitch against a capable baseline (tokens
   were neutral-to-worse until tasks got complex). The skill's description was
   rewritten to lead with parse-safety + symbol-accuracy; token economy is now
   framed as a large-file/many-form secondary benefit. See the SKILL.md diff
   across iterations in the workspace.

## Follow-up: the skill *undertriggers*, and pushier wording doesn't fix it

Ran the description-trigger optimizer (skill-creator's `run_loop.py`, model
`claude-opus-4-8`, 5 iterations) over 20 realistic queries — 10 that should invoke
the skill, 10 near-miss negatives (Rust rename, `deps.edn` resolution, sed on a
log, JSON/YAML formatting, "explain a CL reader macro", etc.).

Result: **recall was 0% on every iteration.** Not one should-trigger query
invoked the skill; every should-not-trigger query correctly abstained
(precision 100%). The numbers were *flat* across five very different
descriptions — including one the optimizer pushed all the way to "Use this skill
for EVERY edit to a file whose path ends in .el …, no matter how ordinary the
change looks." It scored no better, so the optimizer kept the original
safety-first description as best (test 4/8 = exactly the 4 test negatives).

Two things are going on, and both matter:

1. **Structural undertriggering — the same root cause as the tie above.** The
   host only consults a skill for work the model judges it can't already do.
   Claude is confident it can edit Lisp with plain Edit/grep (the benchmark
   proved it *can*), so it never reaches for the skill — and no amount of pushy
   description wording overrides that judgment.
2. **The offline trigger harness under-measures.** `run_loop.py` fires one-shot
   `claude -p` prompts against file paths that don't exist on disk; with nothing
   to act on, the model tends to answer conversationally and never gets to a tool
   call. So 0% is a floor, not necessarily the real in-session rate.

**Decision:** keep the current description (it was the optimizer's own pick) and
**do not chase triggering.** Practical guidance: this skill is most reliable when
*explicitly* invoked ("use lisplens to …" / `/lisplens`), not relied on to
auto-fire. That's an acceptable fit — the payoff (parse-safety, symbol-exact
rename/refs) is worth a deliberate reach, and the failure mode of *not* triggering
is just "Claude edits it the ordinary way," which the benchmark showed is usually
fine anyway.

## Caveats

- 1 run per (eval, config) — directional signal, not tight statistics. Timing/token
  numbers carry real variance (iter-4 baseline ±9.8k tokens).
- Baselines were told not to use lisplens but were otherwise unconstrained; the
  trap eval *warned* about siblings, which flattered the baseline. An unwarned
  naive run is where lisplens' safety margin is widest and wasn't measured here.
- All fixtures are Emacs Lisp. Other dialects parse/edit but weren't benchmarked;
  `format`/auto-reindent was Elisp-only at benchmark time. *(Update, later
  2026-07-04: the multi-dialect formatter, ADR-0031, extends native reindent to
  Common Lisp and the Scheme family (`has_native_engine`). Re-benchmark on those
  dialects once the global binary is reinstalled.)*

## Follow-up: functional improvements this suggests for lisplens (ADR-0032)

The procedural data above says the gap is on the **edit side**: agents hand-assemble
a `refs → line edit batch → refs` idiom for every symbol edit, and iteration 4's
inline-expand ⇄ re-inline was a 10-op patch built by hand. That motivates a tier of
**atomic, self-verifying refactoring procedures** — `rename` (symbol-exact, optional
comment/string mentions), `inline` (expand a def at its call sites), `extract`/`fold`
(structural pattern → call), and a standalone `check` (the `check-parens` the
baseline shelled out to Emacs for). Design + open problems (inline hygiene, the
`extract` pattern language, cross-file atomicity) are in
`docs/adr/0032-atomic-refactoring-procedures.md`.
