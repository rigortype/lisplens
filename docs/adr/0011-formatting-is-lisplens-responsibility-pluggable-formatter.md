# Formatting is lisplens's responsibility, via a pluggable formatter

Agents supply **content, never whitespace**. lisplens guarantees that code is properly formatted and indented for its dialect both **before and after** an edit, so agents never count spaces or manage indentation — a direct token-efficiency and reliability win.

Formatting sits behind a **pluggable interface** with two backends:

- **(A) Native, spec-driven Rust indenter** — the strategic default: deterministic, dependency-free, cross-compilable, and driven by **Indent specs** (form-spec-level metadata), so it stays within the semantic ceiling (ADR-0003).
- **(B) External formatter / Emacs batch subprocess** — a fidelity escape hatch and interim path, opt-in because it breaks determinism and adds environment dependencies.

The external backend lives in **lisplens** (a CLI/MCP tool, which may spawn subprocesses), keeping **lispexp** (a library) pure and dependency-minimal (lispexp ADR-0013). Running both backends lets us compare which reaches acceptable fidelity, mirroring ADR-0001's learn-first stance.

## Status

accepted

## Requires (lispexp)

- Exposure of indent metadata / Indent specs alongside the existing form-spec harvesting, to drive the native indenter.

## Considered Options

- **Delegate formatting to the agent (best-effort splice only)** — rejected: forces agents to manage whitespace, defeating token efficiency.
- **Native-only** — rejected for now: deterministic and pure, but a large faithful-implementation cost with a weak early long tail.
- **External / Emacs-only** — rejected as the default: faithful, but slow to start, environment-dependent, and nondeterministic.

## Consequences

- A custom macro's Indent spec is a Project profile persistence candidate (ADR-0004).
- The agent-facing contract is stable regardless of backend: give content, receive properly formatted code.
