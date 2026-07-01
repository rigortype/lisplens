# Hash form: xxh3-64, 4 hex, verbatim span, two-tier

Both modes compute drift hashes with **xxh3-64 truncated to 4 hex digits** (matching hashline). The hash input is the **verbatim source bytes** — a line's raw bytes in Line-hash mode, a datum's span bytes in Structural mode (surrounding comments and trivia excluded). Hashing is **strict**: any byte change, including whitespace, counts as drift. Two tiers guard each edit — a **per-anchor** hash catches a localized change, and a **file-level** hash guards the whole snapshot.

## Status

accepted

## Considered Options

- **Normalized (whitespace-insensitive) hashing** — rejected. It would suppress false drift from formatting churn, but requires a per-dialect definition of "semantically identical," which is complex and fragile across the polyglot set. Strict verbatim keeps drift simple; the cost of a false drift is only a re-read.
- **Longer hashes** — unnecessary. The file-level hash is the global guard, so a short per-anchor hash suffices.

## Consequences

- Whitespace-only reformatting by another tool invalidates anchors and forces a re-read — accepted.
- The per-anchor / file-level split lets errors be reported locally ("node `foo` drifted") or globally.
