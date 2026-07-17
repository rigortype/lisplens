# lisplens — status snapshot

**Contract (keep this file honest).** This file is one ephemeral handoff and
nothing else — the single "resume here" block for the next session. **Replace,
never append**: a new handoff overwrites this file entirely; git history is the
archive, so deleted handoffs are never lost. Durable knowledge belongs in the
dev docs (`docs/dev/`, `CONTEXT.md`, `docs/adr/`, `CHANGELOG.md`); work items
belong in the GitHub issue tracker (`docs/agents/issue-tracker.md`, labels per
`docs/agents/triage-labels.md`) and are referenced here **by number only** —
never restate an issue's body here. CI enforces a 120-line cap on this file
(the "Snapshot size" step in `.github/workflows/ci.yml`); if the cap fights
you, you are putting durable content in the wrong file.

## Handoff — resume here: `^L`/fidelity arc done; backlog now lives in the issue tracker

**Where.** On `master`, clean, at the merge of PR #55. `[Unreleased]` in the
CHANGELOG holds one feature (`diff --deep --verbose`, PR #50) and five fixes
(PRs #51–#55) — enough material for the next release whenever convenient
(`lisplens-release-prep` skill).

**What just landed — the `^L`/fidelity arc (PRs #51–#55).** Starting from a
real report (`lisplens format` deleted the `^L` page-break separators of
phpstan.el), five same-family fixes: `format` deleting `^L`-only lines (#51);
whitespace-only lines *inside strings* being blanked, rewriting the string's
value (#52); `cl-flet`/`cl-labels`/`cl-flet*`/`cl-macrolet` binding bodies
indented as ordinary calls instead of local defuns (#53); `parinfer indent`
deleting `^L` lines (#54); and its movable-trail scan discarding a `^L` that
trails a close-paren (#55). The `^L` guarantee is pinned by 9 tests across
format + parinfer (incl. a cross-engine test); a full trim/whitespace audit of
`src/` found no remaining `^L` exposure (audit table in PR #55).

**Corpus audit (durable copy: `docs/dev/formatter.md` → Known fidelity
gaps).** A declare-aware three-way sweep of 1167 elpa + php-mode files says
real-world formatter fidelity is effectively complete: ~481 raw diverging
lines were almost all lisplens-*right* (maintained files stale w.r.t. their
own `(declare (indent N))`, or macros inside `eval-and-compile` that a naive
oracle misses). Only two genuine gaps survived — filed as #58 and #59.

**Backlog — in the tracker; this file only points.**

- #56 verify parinfer on real Emacs (ready-for-human; the gate before
  de-preview + README announcement)
- #57 skill-description triggering-optimization loop (ready-for-agent)
- #58 honor `lisp-indent-local-overrides` (ready-for-agent)
- #59 quote-at-EOL data-list indent (ready-for-agent)
- #60 `^L`-sharing-code column gap · #61 Racket infix dot · #62 Clojure
  `#_`-own-line residual · #63 ADR-0019/0018 design chunks · #64 `extract`
  opt-ins (all needs-triage)

**Quality gate.** 247 tests; `cargo fmt --check`, `clippy --all-targets`
(warnings deny), `cargo doc` (warnings deny), `cargo deny check licenses` +
THIRD-PARTY-LICENSES drift guard — all green in CI.

**Standing reminders.** Add a CHANGELOG `[Unreleased]` entry as each
user-facing change lands (release-prep is a seal, not a reconstruction). A
committed PostToolUse hook auto-runs `cargo fmt` after `.rs` edits. lispexp is
upstream — never edit it; record asks in `docs/lispexp-feedback/`.
