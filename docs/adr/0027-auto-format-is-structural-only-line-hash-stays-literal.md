# Auto-format is Structural-only; Line-hash stays literal

Auto-formatting on edit (ADR-0025) applies to **Structural mode only**. Line-hash mode is deliberately **literal** — line-oriented, dialect-agnostic, a parinfer-style escape hatch for exact line control and local repair of broken code (ADR-0001). Imposing structural reindentation there would undermine its purpose, so a Line-hash edit writes content **verbatim** (lisplens only supplies terminators — ADR-0011).

ADR-0011's "edits are always formatted" is therefore the **Structural mode contract**. Anyone who wants a Line-hash-edited file reindented invokes the `format` surface explicitly.

## Status

accepted

## Considered Options

- **Both modes auto-format** — rejected: it loses Line-hash mode's verbatim, line-exact purpose.
- **Neither mode auto-formats** — rejected: contradicts ADR-0025 and ADR-0011.

## Consequences

- Clean role split: **Structural** = structure-aware + formatted; **Line-hash** = verbatim + unformatted.
