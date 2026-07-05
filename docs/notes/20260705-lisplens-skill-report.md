# lisplens agent-skill report: post-brush-up validation + refactor primitives (2026-07-05)

Sequel to [20260704-skill-benchmark-agent-edit-procedures.md](20260704-skill-benchmark-agent-edit-procedures.md)
(iterations 1–4 + the DeepSeek cross-vendor run + the trigger-optimization
finding). This note records: (1) the skill now lives in-repo and was polished
with `waza`; (2) an iteration-5 regression check confirming the polish didn't
break anything; (3) the load-bearing forward finding — the three benchmark
refactors are *universal* operations that lisplens should expose as
**primitives**; (4) two small agent-facing frictions fixed here.

Skill location: `skills/lisplens/` (SKILL.md + references/patch-dsl.md),
installable via the vercel-labs/skills CLI. Same content mirrored to
`~/.claude/skills/lisplens/` for local use.

## Brush-up with `waza dev`: Low → Medium-High

Ran `waza dev` (v0.33.0) on the skill. It scored **Low**, then **Medium-High**
after targeted edits. What moved the needle, and what we deliberately left:

| Change | Why |
| --- | --- |
| Added `license: MPL-2.0` + `metadata.version` | spec best-practice (were flagged missing) |
| Description leads with a verb; explicit `USE FOR:` / `DO NOT USE FOR:` (near-miss negatives: non-Lisp files, dep/build errors, explaining syntax) | waza reads triggers/anti-triggers from those literal markers; also genuine routing clarity |
| Body 11 → 3 H2 sections, ~2.2k → ~1.6k tokens; added a guardrails/troubleshooting block | SkillsBench "2–3 modules optimal"; progressive disclosure |
| **Not** pursued: "High" adherence | needs workflow-router markers (`INVOKES:` / `FOR SINGLE OPERATIONS`) meant for orchestrator skills like waza itself — noise on a single-capability skill |
| **Not** pursued: <60-word description | would drop the anti-grep rationale + trigger phrases; cross-model use already validated (DeepSeek) with a longer description |

Takeaway: waza's scorer is a useful checklist, but it encodes a Copilot/workflow
convention. Apply the parts that are real quality signals (license/version,
module count, explicit routing); don't contort a capability skill into
orchestrator syntax to chase the top grade.

## Iteration-5: regression check (post-brush-up)

The rewrite consolidated the body and moved the verb list to references — a real
risk that the leaner body would stop guiding agents. Re-ran three
`cc-engine.el` tasks, with-skill (brushed-up) vs no-skill baseline, 1 run each.

| Eval | Result (both configs) |
| --- | --- |
| symbol-rename trap (`c-macro-cache` → `c-cpp-macro-cache`, siblings intact) | ✅ new 26 / old 0 / siblings 14·12·10 / 0 corruption / parses |
| local rename (`prevstate` → `previous-state`, in-function only) | ✅ both sites + in-form comment, confined, parses |
| add docstring (`c-query-macro-start`, neighbor untouched) | ✅ docstring added, neighbor byte-identical, parses |

**100% both configs; no regression.** With-skill agents still used the idiom
(`refs` → `line edit`; `struct read` → `line read` → `struct edit`), so the
consolidated body carries the workflow. Tokens/time unchanged in character
(with-skill +~4.5k tokens on these single-target tasks; the value remains
safety, not token economy — see the prior note).

## Forward finding: these three refactors are *primitives*

The benchmark tasks weren't picked to be exotic — they're the everyday shape of
Lisp editing, and they recur across every fixture. That argues they should be
**first-class lisplens operations**, not something an agent re-derives from
`refs` + `line edit` each time. Status today:

| Operation (benchmark) | Primitive today | Gap |
| --- | --- | --- |
| **File-wide symbol rename** (`c-macro-cache`) | ✅ `lisplens rename <old> <new> <file>` (symbol-exact, validate-then-write) — in the repo (v0.2.0) | Not yet in the *installed* 7-command binary the skill was validated against, so the skill still teaches the `refs`+`line edit` longhand |
| **Scoped / in-function rename** (`prevstate`) | ◐ `struct edit`'s `rename <anchor> <from> <to>` verb (subtree-scoped) | No CLI-level "rename within this form"; only reachable through a patch |
| **Add / edit a docstring** (`c-query-macro-start`) | ✗ none — done by `replace`-ing the whole enclosing form | A common, safe, mechanical edit with no dedicated op; also the `insert-*`-into-a-form gap below |

Recommendations, in priority order:

1. **Ship `rename`/`inline`/`extract` to the installed binary + a release**, then
   update the skill to teach `lisplens rename …` as the one-shot path for
   file-wide renames (replacing the `refs`+`line edit` recipe). This is the
   single biggest agent-ergonomics win and the repo already has the code.
2. **Add a docstring primitive** (e.g. `struct edit` verb `docstring <anchor>
   <<TAG` that inserts/replaces the leading docstring of the anchored def, or a
   CLI `lisplens docstring <anchor> <file>`). It's mechanical, safe, and one of
   the three universal shapes.
3. **Close the "edit inside a form" gap** that `insert-*` exposes (below); a
   general "insert at inner position / replace inner slot" would subsume both the
   docstring op and other in-form edits.
4. Consider a CLI-level scoped rename (`rename <old> <new> <file> --in <anchor>`)
   so in-function renames don't require hand-writing a structural patch.

The through-line: lisplens already chose this direction (`rename`/`inline`/
`extract` exist); the benchmark independently confirms these are the right
primitives to invest in, and points at **docstring** + **in-form editing** as the
next gaps.

## Frictions fixed here

Both surfaced when agents drove the tool in iteration-5.

1. **`lisplens --help` exited non-zero.** `usage()` (stderr + `ExitCode::FAILURE`)
   was the only help path, so an agent's install-check (`lisplens --help`, which
   the skill recommends) looked like a failure. Added `--help`/`-h`/`help` →
   prints usage to **stdout**, exits **0**; bare/unknown invocation still →
   stderr + non-zero (that's genuine misuse). (`src/main.rs`.)
2. **`insert-after` on an inner node returns `BadOp`.** An agent tried
   `insert-after` anchored at a defun's arglist to add a docstring and got
   `BadOp`, then fell back to `replace`. Documented in SKILL.md: `insert-*`
   target a top-level/sibling form; to change something *inside* a form,
   `replace` the enclosing form. (The proper fix is primitive #2/#3 above.)

## Update — primitives surfaced to agents + step-count re-benchmark

Acting on recommendation #1: rebuilt the current source (which already carries
`rename`/`inline`/`rewrite`/`extract`/`check` per ADR-0032) and installed it over
`~/local/bin/lisplens` (old 7-command build backed up as `lisplens.bak-7cmd`).
Then updated the skill to teach the primitives ("reach for these first"), keeping
the read→anchor→edit loop as the fallback. Smoke-tested each:

- **`rename`** — `lisplens rename c-macro-cache c-cpp-macro-cache <file>` →
  `renamed 26 occurrence(s)`, siblings 14/12/10 untouched, 0 corruption, parses.
  One command for what was a `refs → line edit batch → refs` idiom.
- **`inline`** — hygienic: substitutes via `let`-bindings (`(let ((x a)) …)`) to
  preserve single-evaluation/order, and **requires the definition in the target
  file** (cross-file inline — iteration 4's def-in-`php.el`, calls-in-`php-mode.el`
  — is refused with "no definition found", the ADR-0032 cross-file-scope gap). So
  the skill flags cross-file inline as a loop-fallback case.
- **`check`** — parse/validate, exit 0/non-zero; replaces shelling out to Emacs.

**Re-benchmark (ADR-0032 #113)** — the symbol-rename trap, new skill (teaches
`rename`) vs the prior hand-assembled idiom:

| | iter-5 (manual idiom) | iter-6 (`rename` primitive) |
| --- | --- | --- |
| edit procedure | `refs` → `line read` → 26-op `line edit` → `refs` | **one `lisplens rename` call** |
| tool calls | 11 | 6 (the rest are optional verification the post-condition summary already gives) |
| wall time | 93.8s | 40.2s |
| tokens | 36.6k | 34.5k |
| correctness | 26/0, siblings 14·12·10, parses | identical |

The multi-step idiom collapsed to a single self-verifying command, at correctness
parity — the ergonomic win ADR-0032 predicted, now measured. The skill will need
one more pass once the primitive-equipped binary is the *published* release (the
crates.io/installed default), so downstream users get the same surface.

## Update 2 — the three shapes are now first-class primitives (PRs #17–#21)

The plan the report set out (surface the existing refactors, add the missing
docstring op, close the "edit inside a form" gap) was carried out as a short
sequence of PRs. Intent → what shipped:

| PR | Intent | Result |
| --- | --- | --- |
| #17 | Ship the skill in-repo; stop pointing users at ADRs; make `--help` usable | `skills/lisplens/` (vercel-labs/skills-installable); README/`--help` ADR links removed; `--help`/`-h` → stdout, exit 0 |
| #18 | The missing **docstring** primitive | `lisplens docstring <name> <file>` (text on stdin, escaped), function-like defs (ADR-0044) |
| #19 | Mirror it into MCP (ADR-0032 says CLI verbs mirror) | `docstring` MCP tool; stale tool-surface comment refreshed |
| #20 | Docstring **v2 — Elisp variables** | `defvar`/`defconst`/`defcustom`/… (docstring after the value); valueless/Scheme-value refused |
| #21 | Close the **insert-into-form** gap (the `insert-*` `BadOp`) | structural `insert-after`/`insert-before` on any node incl. inner ones; reindented, parse-checked |

Net: all three universal shapes — file-wide rename, add-docstring (function *and*
variable), and inserting a form inside another — now have a first-class,
self-verifying command or verb. `rename`/`inline`/`extract`/`check` already
existed (ADR-0032); this arc added `docstring` and structural `insert-*`.

**Iteration-7 — validation that the skill drives the new primitives.** Three
`cc-engine.el` tasks (symbol-rename trap, function docstring, `defvar`
docstring), with-skill vs no-skill baseline, on the primitive-equipped binary:

- **100% both configs**, all outputs re-verified (rename 26/0, siblings 14·12·10;
  docstrings placed and parsing; neighbours untouched).
- With-skill reached for the primitives unprompted: `lisplens rename` (one
  command for the trap, vs the baseline's hand-built `perl (?![-\w])` lookahead),
  and `lisplens docstring` for both the function and the `defvar` (v2). So the
  skill genuinely routes agents onto the atomic path.
- Tokens/time stay higher with-skill (primitives + a verification step) at
  correctness parity — consistent with the whole series' finding: *the value is
  the safe, one-shot, self-verifying path, not token economy.*

**One friction, now documented.** On the function-docstring task the with-skill
agent's docstring text contained a backtick + apostrophe (`` `c-macro-start' ``),
which its **shell** mangled in `printf '…' | lisplens docstring …` — truncating
the text before it reached lisplens (lisplens's own escaping was fine). It caught
this on read-back and recovered by feeding the text via a file on stdin. The
skill now tells agents to pass docstring/heredoc text through a quoted heredoc or
a file when it contains backticks or quotes, so the shell can't corrupt it.

## Caveats

- Iteration-5/6/7 are 1 run per (eval, config) — regression/validation *smoke
  tests*, not new statistics. They reuse iteration-3's fixtures and ground truth.
- The locally-installed binary was rebuilt from `master` after each merge, so it
  carries the new primitives; the remaining gap is the **published crates.io
  release** — until that ships, a fresh `cargo install lisplens` won't have
  `rename`/`docstring`/structural `insert-*`, and the skill's primitive guidance
  would outrun the binary. Cutting that release is the open follow-up.
