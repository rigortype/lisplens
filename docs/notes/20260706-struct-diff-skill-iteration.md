# Enhancing the lisplens skill for `diff` — an experiment + bottleneck-driven iteration

Date: 2026-07-06. Context: `lisplens diff` (Structural diff, ADR-0047/0048, plus
the #44 added/removed-body Lens) had just landed. This note records how the
in-repo `skills/lisplens/` skill was extended to expose it, and — more usefully —
the measurement loop that shaped the skill text. The headline lesson is about
*method*: behavior traces beat self-report, per-eval beats the mean, and a
self-documenting binary confounds the obvious benchmark.

## Part 1 — a framing experiment: raw diff vs structural diff for summarization

Before touching the skill, we ran a controlled experiment (its own artifact
report): seven agent runs summarized what changed in Emacs's `cc-engine.el`
between emacs-30 and emacs-31 (~16k lines each; note emacs-31 and master are
byte-identical for this file, so master was dropped). Each run got either a raw
`git diff` (~34k tokens) or a lisplens structural diff (map + `--deep`, ~16.5k
tokens), across Claude Sonnet, MiMo-V2.5, and DeepSeek V4 Flash (the latter two
via the acp-agent-runner / OpenCode). A Claude judge scored all seven against the
21-commit ground-truth log.

Findings:
- **Structural diff ≈ half the context and far fewer turns**, consistently across
  all three models (peak input MiMo 113k→64k, DeepSeek 92k→58k; assistant turns
  29/23 → 9).
- **Raw diff won coverage** (A-runs 26/25/23 vs best structural 22). The raw diff
  exposes *added-function bodies*; the structural diff named which definitions
  were added but not their internals, so structural runs missed body-internal
  detail (C23 `_BitInt`, `c-in-id-arglist`, `class-field-cont`).
- **Accuracy partly inverted**: thin structured input made the honest model
  (Sonnet) *more* careful and self-aware of its blind spots, but made a
  confabulating model (DeepSeek on the structural input) hallucinate mechanisms
  it could not have seen.

That coverage gap was the direct motivation for **#44** — `diff --deep` now
renders added/removed definitions' bodies as their `expand` Lens, closing most of
the gap while keeping the token win.

## Part 2 — the skill benchmark was a wash, and *why* that's informative

We drafted the skill enhancement (a "Compare two versions — `lisplens diff`"
section: the map, `--deep`/`--unit`, `--json`, exit-0 semantics, formatting-
insensitivity, anchor→edit chaining), then benchmarked new-skill vs old-skill
(snapshot) across three evals: a small "which defs changed" map, a small "how did
this defun change" drill, and the large cc-engine release summary.

Result: **100% vs 100% correctness, delta +0.00.** The reason is the important
part: `lisplens --help` self-advertises `diff`, so the *baseline* agent (old
skill, no diff mention) discovered and used it anyway. A self-documenting binary
means the skill text's marginal value cannot show up as *correctness* — it lives
in (a) efficiency (going straight to the right command vs a discovery detour),
(b) the non-obvious semantics `--help` doesn't convey, and (c) triggering (does
the skill fire at all). So we stopped chasing the correctness number and measured
*behavior* instead.

## Part 3 — bottleneck analysis from the tool-call traces

We parsed each subagent transcript into a tool-call timeline (extracting only tool
names + truncated args + inter-step gaps, never dumping content, to avoid
context blowup). Three frictions were visible and skill-fixable:

- **A — the `--help` detour (self-inflicted).** Every run ran `lisplens --help`
  first (+3–8s, a wasted round-trip) — because the skill *told it to* ("confirm
  installed with `lisplens --help`").
- **B — re-reading raw source to verify the diff.** Agents didn't trust
  `diff`/`--unit`/`--deep` and re-opened the `.el` files (or ran `struct read`) to
  double-check what the diff had already shown completely.
- **C — drill granularity on a big change.** On the 16k-line file agents drilled
  ad hoc (looping `--unit` over many names) with no decision rule.

A fourth "cost" — the model composing the final summary (~30% of wall-time on the
big case) — is the deliverable itself and correctly *not* a skill target.

## Part 4 — three revision rounds, and one that backfired

Baseline is v0.2.0 (the first diff-enhanced skill). Each round changed the skill
and re-measured the tool behavior (candidate-only runs; v0.2.0 numbers reused).

- **v0.2.1** — Fix A (drop the mandated `--help`; only report `command not
  found`), Fix B (state the diff tree is *complete* for "how did it change", so
  don't re-open the source), Fix C-first (**"for a whole-file summary, use
  `--deep`"**).
- **v0.2.2** — Fix C revised after v0.2.1 backfired (below).

Per-eval tool count / wall-time:

| eval | v0.2.0 | v0.2.1 | v0.2.2 |
| --- | --- | --- | --- |
| 0 · which-defs map | 7 / 54s | 5 / 37s | **4 / 28s** |
| 1 · how-did-defun drill | 7 / 67s | **4 / 44s** | 5 / 51s |
| 2 · large-file summary | 11 / 142s | 18 / 262s | **8 / 122s** |

- **A and B were pure wins**: v0.2.1/v0.2.2 eliminated the `--help` call and the
  raw-source re-reads on every run (eval-1 dropped from 7 to 4 tools).
- **C-first backfired on the large file.** "Use `--deep`" made the agent pull the
  *entire* ~16k-token deep diff at once and then still read added-def sources —
  18 tools / 262s, worse than v0.2.0's targeted `--unit` (11 / 142s). The mistake:
  the eval-2 question is "*which* definitions changed", which **the map alone
  answers** — no drill needed.
- **C revised (v0.2.2)**: "match the drill depth to the question — if it's *which*
  changed, the map is the answer; drill only for *how*; `--deep` expands the whole
  changeset so it's a big pull on a large change — prefer the map + a few `--unit`."
  Eval-2 fell to 8 tools / 122s (no `--deep`, targeted `--unit`), the best of the
  three, while keeping A/B.

**v0.2.2 is Pareto-best or tied on every eval, at 100% correctness throughout, and
near the floor** (4 tools = read-skill + one `diff` + write). We stopped there:
further tuning would overfit three evals for diminishing returns. Adopted v0.2.2.

## Method lessons (the durable part)

1. **Behavior trace > self-report.** We never asked the agent "why did you
   re-read the source?" — an LLM's "why" is post-hoc confabulation, not
   introspection (the same failure the Part-1 experiment caught DeepSeek doing).
   The reliable loop is: read the *tool trace* (what it did), hypothesize the
   trigger, edit the skill, and *re-measure the behavior*. Fix B was confirmed
   when re-reads went to zero — stronger evidence than any narrated rationale.
2. **Per-eval beats the mean.** The v0.2.1 aggregate ("time −27s") was dominated
   by the eval-2 outlier and hid that A/B helped while C hurt. Always read the
   per-eval breakdown and outliers, not just mean ± stddev.
3. **A self-documenting binary confounds skill-vs-baseline correctness.** Because
   `--help` advertises `diff`, the baseline matched on correctness; the skill's
   value had to be measured as efficiency/behavior (and, separately, triggering).
4. **Skill guidance can backfire — always re-measure after a change.** "Prefer
   `--deep`" read as sensible but doubled the cost on the case it was meant to
   help. A one-line rule of thumb interacts with real inputs; verify it.

## Artifacts

Skill: `skills/lisplens/SKILL.md` (v0.2.2) + `references/patch-dsl.md`; eval set
`skills/lisplens/evals/`. The iteration workspace (`skills/lisplens-workspace/`,
transcripts, and the large cc-engine fixtures) is scratch and gitignored.
