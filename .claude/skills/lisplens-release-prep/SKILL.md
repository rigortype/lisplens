---
name: lisplens-release-prep
description: Prepare a lisplens release through a review PR — bump the crate version, seal the changelog, reconcile the README, run verification, open a release PR for a human to approve, then merge and tag so GitHub Actions publishes to crates.io and attaches pre-built binaries. Use when the user asks to prepare the next version, cut a release, refresh release metadata, or make versioned files consistent before tagging.
metadata:
  internal: true
---

# lisplens Release Prep

Follow this workflow to release a new `lisplens` version. It ships two ways from
one tag: the crate on [crates.io](https://crates.io/crates/lisplens) (`cargo
install lisplens`) and per-platform pre-built binaries on the GitHub Release.

**The flow is PR-gated.** You prepare the release on a branch (version bump,
changelog seal, README reconcile, verify), open a **release PR** so a human can
review the `CHANGELOG.md` and `README.md` diffs, and on their Go you merge and push
a `vX.Y.Z` tag. The tag triggers [`release.yml`](../../../.github/workflows/release.yml),
which runs `cargo publish`, creates the GitHub Release from `CHANGELOG.md`, and
builds + uploads the binaries. You never run `cargo publish` or handle the crates.io
token by hand (a manual fallback is at the end). **Publishing is irreversible**
(crates.io versions can only be yanked, never replaced), so the human Go sign-off on
the PR is the gate — do not merge-to-publish without it.

At a glance: prepare on a branch → **PR (human reviews CHANGELOG + README)** → merge
→ tag → Actions publishes → record the release in `CURRENT_WORKS.md`.

## One-time setup (skip if already done)

- A crates.io API token is stored as the repository secret
  `CARGO_REGISTRY_TOKEN` (GitHub → Settings → Secrets and variables → Actions),
  created at <https://crates.io/settings/tokens> with the `publish-new` and
  `publish-update` scopes. The first-ever publish claims the crate name; after
  that the token can be narrowed to `publish-update`.
- The binary job uses the built-in `GITHUB_TOKEN`; no extra secret needed.

## Update release metadata

Decide the next semantic version first, then update all versioned files together.

- `Cargo.toml` — the `version` field.
- `Cargo.lock` — bump lisplens's own entry (run `cargo build`, or
  `cargo update -p lisplens --precise <x.y.z>`, and commit the change). Unlike a
  library, this binary crate **tracks `Cargo.lock`**, so it must stay in sync.
- `THIRD-PARTY-LICENSES.md` — **regenerate it in the same commit**, even when no
  dependency changed: `cargo about generate about.hbs -o THIRD-PARTY-LICENSES.md`.
  `cargo about` includes lisplens *itself* in the file (its own MPL-2.0 entry
  carries the version number), so a version bump alone makes the committed file
  stale — and CI's "Third-party licenses up to date" drift guard will fail the
  release PR if you skip this. (Needs `cargo about` installed:
  `cargo install cargo-about --features cli`.)
- `CHANGELOG.md` — seal `[Unreleased]` into the new version section (below).

### Seal the `[Unreleased]` entries — the load-bearing step

The highest-value, most-skipped part of a release; `cargo test` cannot check it.
The changelog is for humans — make it read like release notes, not commit
messages. It is also the review surface the release PR exists for.

1. **If `[Unreleased]` is empty or thin, reconstruct it first.** Entries are meant
   to accumulate there as work lands, but that discipline slips. Run
   `git log <last-tag>..HEAD --oneline` (e.g. `git log v0.1.1..HEAD`), and derive
   the user-facing changes from the commits, PRs, and the `## Now` bullets in
   `docs/CURRENT_WORKS.md` (which already summarise each change in release voice).
2. Read the whole `[Unreleased]` block. Classify each top-level bullet:
   release-style (leave) or commit-style (rewrite).
3. Rewrite every commit-style bullet — one self-contained sentence per bullet;
   move "why / how / measured numbers" into a child item (`  - …`); delete
   internal-only detail (private refactors, test additions) outright. Ask of each
   entry: "would a user of the tool care if they weren't reading the source?"
4. Consolidate several commits' entries into one user-recognisable change; split
   merge artefacts.
5. Re-read the sealed section as a user would.

### Release mechanics

- Add a `## [x.y.z] - YYYY-MM-DD` section immediately below `## [Unreleased]`,
  optionally opening with a 2–4 sentence prose summary of the release's themes.
- Use Keep a Changelog headings verbatim: `Added`, `Changed`, `Deprecated`,
  `Removed`, `Fixed`, `Security`. Group like changes; no `####` inside a version
  block.
- **Do not hard-wrap entries.** Each bullet and the summary paragraph is a single
  physical line, however long — `release.yml` extracts the section verbatim as
  the GitHub Release body, and wrapping degrades it there.
- Update the bottom-of-file links: point `[Unreleased]` at
  `compare/vx.y.z...HEAD` and add
  `[x.y.z]: https://github.com/rigortype/lisplens/releases/tag/vx.y.z`.

## Reconcile the README

`README.md` is the crate's crates.io page and first impression — it drifts as
features land. Before opening the PR, reconcile it against the **sealed changelog**
and the **actual binary**, and fold any fixes into the release branch so they are
part of the reviewed diff:

- **Commands** — the `## CLI` block and the `### Refactoring` list match the real
  subcommands. Check against the usage string / `--help` (grep `src/main.rs`), not
  memory; every subcommand the binary exposes should appear, and none that it does
  not.
- **Languages** — the native-engine vs fallback tables match the code:
  `has_native_engine` and `engine_for` in `src/format/mod.rs` for the tiers, and
  the extension→dialect map in `src/lib.rs` (`dialect_for_path`) for the columns.
- **Features & Status** — every user-facing change in this release's changelog that
  a reader would look for is reflected, and the `## Status` section states the
  current reality (no stale "future work" for things now shipped).
- **Config** — the formatting-config section still names what is read (file-local
  `-*- … -*-` / `Local Variables:`, `.dir-locals.el`, `.editorconfig`, Nameless).

If nothing changed, say so and move on — but actually look; this check exists
because README drift is silent.

## Verify the release

Run before opening the PR (this is what `ci.yml` enforces, plus the package check):

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo doc --no-deps
cargo publish --dry-run --allow-dirty   # --allow-dirty only if not yet committed
git diff --check
```

`cargo publish --dry-run` packages the crate as crates.io will; confirm a small
file count (agent/CI files are dropped via `Cargo.toml`'s `exclude`). Commit any
formatting/non-version cleanup separately — do not fold it into the version bump.

## Prepare on a branch and commit

Work on a release branch off up-to-date `master` (never bump on `master` directly —
the whole point is that the change lands via the reviewed PR):

```sh
git checkout master && git pull --ff-only
git checkout -b release/vx.y.z
```

One release-prep commit with the `Cargo.toml` bump, the `Cargo.lock` bump, the
`CHANGELOG.md` seal, and any `README.md` reconcile edits:

```text
Bump up version to x.y.z
```

## Open the release PR — the review gate

```sh
git push -u origin release/vx.y.z
gh pr create --base master --title "Release vx.y.z" --body "$(cat <<'EOF'
Release vx.y.z. Publishing is triggered by the `vx.y.z` tag after this merges.

**Review focus:** the `CHANGELOG.md` [x.y.z] section (it becomes the GitHub Release
body verbatim) and the `README.md` diff (it is the crates.io page). Approve to
publish.
EOF
)"
```

Then **stop and hand off to the human**: they review the rendered `CHANGELOG.md` and
`README.md` diffs on the PR and give the Go. Do not merge on your own initiative —
this approval is the irreversible-publish gate. Make sure the PR's `ci.yml` check is
green before asking for the Go.

## Merge, then tag to publish

Only after the PR is **approved and its CI is green**:

```sh
gh pr merge --merge                 # or the repo's preferred merge style
git checkout master && git pull --ff-only
grep '^version' Cargo.toml          # sanity: equals x.y.z (release.yml re-checks)
git tag vx.y.z                      # tag master's HEAD (Cargo.toml == x.y.z there)
git push origin vx.y.z              # runs release.yml -> crate + binaries + Release
gh run watch                        # watch the publish
```

The tag push triggers [`release.yml`](../../../.github/workflows/release.yml): it
checks the tag matches `Cargo.toml`, runs `cargo publish`, creates the GitHub
Release from this version's `CHANGELOG.md` section, then builds and uploads
binaries for x86_64/aarch64 Linux, x86_64/aarch64 macOS, and x86_64 Windows.

## After publish — record the release

Once the publish is verified (crate on crates.io, Release with binaries attached),
add a **`Released x.y.z`** bullet to the `## Now` section of
[`docs/CURRENT_WORKS.md`](../../../docs/CURRENT_WORKS.md) — one paragraph: the
release's theme, the tag + release commit, and confirmation that crates.io + the
5-platform binaries are live — and refresh the top **Handoff** block to the
post-release state. This step is *after* publish on purpose: the bullet asserts
published facts. A docs-only commit straight to `master` is fine here:

```text
CURRENT_WORKS: record the x.y.z release
```

Verify the outcome:

```sh
cargo search lisplens | head -1                              # newest = x.y.z
gh release view vx.y.z --json assets --jq '.assets[].name'  # 5 binaries attached
```

## Manual fallback (if Actions is unavailable)

Publish from a clean `master` at the release commit (after the PR has merged):

```sh
cargo login                         # paste a crates.io token, once
cargo publish
git tag vx.y.z && git push origin vx.y.z
gh release create vx.y.z --title vx.y.z \
  --notes "$(awk -v v=x.y.z '$0 ~ "^## \\["v"\\]"{p=1;next} p&&/^## \\[/{exit} p' CHANGELOG.md)"
# then build binaries per target and `gh release upload vx.y.z <files>`
```

## Quick checklist

- `[Unreleased]` reconstructed if it had drifted empty; every bullet classified and,
  if commit-style, rewritten — no bullet has two sentences, an internal-only detail,
  or a merge artefact. (Confirm by eye — CI cannot.)
- `README.md` reconciled against the sealed changelog and the real CLI: commands,
  language tiers, features/Status, and config sources all current.
- `Cargo.toml` `version` and the `lisplens` entry in `Cargo.lock` both equal `x.y.z`.
- `THIRD-PARTY-LICENSES.md` regenerated with `cargo about` (it carries lisplens's own
  version, so a bump alone makes it stale and fails CI's drift guard).
- `[Unreleased]` / `[x.y.z]` links at the bottom of `CHANGELOG.md` resolve.
- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo doc`, and
  `cargo publish --dry-run` all pass; the commit message is `Bump up version to x.y.z`.
- The change lands via a **release PR** approved by a human; you did not bump or
  merge on `master` without that Go.
- The `vx.y.z` tag matches `Cargo.toml` and is pushed only after merge + green CI.
- After publish: the crate version is on crates.io, the Release has binaries, and
  `docs/CURRENT_WORKS.md` records the release.
