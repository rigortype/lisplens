# Both modes share one content-hash drift mechanism

Structural mode anchors on a **content hash of the addressed node**, exactly as Line-hash mode anchors on a line's content hash. The two modes therefore share a single drift-detection mechanism and differ only in how the edit target is **named** (a line, versus a definition name / role / node path). The Lens emits a short hash per addressable node, so an agent can edit by name-plus-hash safely.

## Status

accepted

## Consequences

- Closes the open question previously noted on the `Anchor` glossary term.
- Drift stays a single concept across the product, even though addressing and read output remain mode-specific (ADR-0001).
