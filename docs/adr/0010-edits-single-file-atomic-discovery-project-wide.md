# Edits are single-file and atomic; discovery is project-wide

An edit Batch targets exactly **one file**; the drift hash, the validate-then-write check, and the atomic write are all per-file. lisplens does **not** offer cross-file atomic edits.

Read and discovery, by contrast, may span the project. A **Project search** returns the locations of definitions or symbol occurrences (bodies omitted), and an optional project outline aggregates per-file Outlines. Cross-file changes are the agent's job to orchestrate as a sequence of single-file, drift-checked Batches.

In particular, **cross-file rename is not a dedicated operation** — it decomposes into a Project search plus per-file occurrence edits, staying within the occurrence-based, semantic-ceiling limits of Rename (ADR-0003).

The project root is inferred zero-config from the working directory / VCS root (ADR-0004); the file set is discovered by recognized extensions and may be overridden by the Project profile.

## Status

accepted

## Considered Options

- **Fully single-file (no project features)** — rejected: "find the target first" is a core agent flow; forcing an external tool wastes the structural knowledge lisplens already has.
- **Full project scope with cross-file atomic edits / rename** — rejected: cross-file atomicity is hard to roll back safely, and cross-file occurrence rename exceeds the safe, form-annotator-level ceiling.

## Consequences

- Safety machinery stays simple (per-file).
- Multi-file refactors require agent orchestration; the tool provides cheap discovery but not one-shot cross-file mutation.
