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

## Handoff — resume here: 0.6.0 released; backlog lives in the issue tracker

**Where.** On `master`, clean, at the merge of release PR #66 (`2662b94`) plus
this record commit. Tag **`v0.6.0`**; crates.io shows 0.6.0 and the GitHub
Release carries all 5 platform binaries — both verified. CHANGELOG
`[Unreleased]` is empty.

**Released 0.6.0 (2026-07-18) — the fidelity-hardening release.** The
`^L`/fidelity arc (PRs #51–#55: page-break preservation across `format` +
`parinfer`, string-interior whitespace preservation, `cl-flet`/`cl-labels`
binding bodies as local defuns) plus `diff --deep --verbose` (PR #50). Backed
by a declare-aware sweep of 1167 elpa + php-mode files: real-world formatter
fidelity is effectively complete; the two surviving niche gaps are filed as
#58/#59 (durable copy of the audit: `docs/dev/formatter.md` → Known fidelity
gaps). 247 tests.

**Release ops note.** The first publish attempt failed 403 at the crates.io
upload (expired `CARGO_REGISTRY_TOKEN`); rotating the repo secret and
re-running the same tag-triggered run completed cleanly — no re-tag needed.
The durable version of this lesson now lives in the release-prep skill.

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
