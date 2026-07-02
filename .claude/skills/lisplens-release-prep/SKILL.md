---
name: lisplens-release-prep
description: Prepare a lisplens release by bumping the crate version, sealing the changelog, running release verification, and tagging so GitHub Actions publishes to crates.io and attaches pre-built binaries. Use when the user asks to prepare the next version, cut a release, refresh release metadata, or make versioned files consistent before tagging.
metadata:
  internal: true
---

# lisplens Release Prep

Follow this workflow to release a new `lisplens` version. It ships two ways from
one tag: the crate on [crates.io](https://crates.io/crates/lisplens) (`cargo
install lisplens`) and per-platform pre-built binaries on the GitHub Release.

**Publishing is automated.** You prepare the release locally (version bump,
changelog, verify, commit) and push a `vX.Y.Z` tag; the
[`release.yml`](../../../.github/workflows/release.yml) workflow then runs
`cargo publish`, creates the GitHub Release from `CHANGELOG.md`, and builds +
uploads the binaries. You never run `cargo publish` or handle the crates.io token
by hand (a manual fallback is at the end).

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
- `CHANGELOG.md` — seal `[Unreleased]` into the new version section (below).

### Seal the `[Unreleased]` entries — the load-bearing step

The highest-value, most-skipped part of a release; `cargo test` cannot check it.
The changelog is for humans — make it read like release notes, not commit
messages.

1. Read the whole `[Unreleased]` block. Classify each top-level bullet:
   release-style (leave) or commit-style (rewrite).
2. Rewrite every commit-style bullet — one self-contained sentence per bullet;
   move "why / how / measured numbers" into a child item (`  - …`); delete
   internal-only detail (private refactors, test additions) outright. Ask of each
   entry: "would a user of the tool care if they weren't reading the source?"
3. Consolidate several commits' entries into one user-recognisable change; split
   merge artefacts.
4. Re-read the sealed section as a user would.

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

## Verify the release

Run before committing (this is what `ci.yml` enforces, plus the package check):

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo doc --no-deps
cargo publish --dry-run
git diff --check
```

`cargo publish --dry-run` packages the crate as crates.io will; confirm a small
file count (agent/CI files are dropped via `Cargo.toml`'s `exclude`). Commit any
formatting/non-version cleanup separately — do not fold it into the version bump.

## Commit

A single release-prep commit with the `Cargo.toml` bump, the `Cargo.lock` bump,
and the `CHANGELOG.md` update:

```text
Bump up version to x.y.z
```

## Push, then tag to publish

```sh
git push origin master              # runs ci.yml
gh run watch                        # wait for the CI gate to go green
git tag vx.y.z                      # tag the release commit
git push origin vx.y.z              # runs release.yml -> crate + binaries + Release
gh run watch                        # watch the publish
```

The tag push triggers [`release.yml`](../../../.github/workflows/release.yml): it
checks the tag matches `Cargo.toml`, runs `cargo publish`, creates the GitHub
Release from this version's `CHANGELOG.md` section, then builds and uploads
binaries for x86_64/aarch64 Linux, x86_64/aarch64 macOS, and x86_64 Windows. Do
not tag until `ci.yml` is green.

## Manual fallback (if Actions is unavailable)

Publish from a clean `master` at the release commit:

```sh
cargo login                         # paste a crates.io token, once
cargo publish
git tag vx.y.z && git push origin vx.y.z
gh release create vx.y.z --title vx.y.z \
  --notes "$(awk -v v=x.y.z '$0 ~ "^## \\["v"\\]"{p=1;next} p&&/^## \\[/{exit} p' CHANGELOG.md)"
# then build binaries per target and `gh release upload vx.y.z <files>`
```

## Quick checklist

- Working tree starts clean or every pending change is understood.
- `Cargo.toml` `version` and the `lisplens` entry in `Cargo.lock` both equal the
  new `x.y.z`.
- Every former `[Unreleased]` bullet was classified and, if commit-style,
  rewritten; no bullet has two sentences, an internal-only detail, or a merge
  artefact. (Confirm by eye — CI cannot.)
- `[Unreleased]` / `[x.y.z]` links at the bottom of `CHANGELOG.md` resolve.
- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo doc`, and
  `cargo publish --dry-run` all pass.
- The final commit message is `Bump up version to x.y.z`.
- `ci.yml` is green before tagging; the `vx.y.z` tag matches `Cargo.toml`.
- After publish: the crate version is on crates.io, the `vx.y.z` tag is on
  `origin`, and the GitHub Release exists with binaries attached.
