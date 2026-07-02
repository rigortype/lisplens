# Hash form: xxh3-64, 4 hex, verbatim span, two-tier

Both modes compute drift hashes with **xxh3-64 truncated to 4 hex digits** (matching hashline). The hash input is the **verbatim source bytes** — a line's content bytes in Line-hash mode (excluding its terminator; see below), a datum's span bytes in Structural mode (surrounding comments and trivia excluded). Hashing is **strict**: any byte change, including whitespace, counts as drift. Two tiers guard each edit — a **per-anchor** hash catches a localized change, and a **file-level** hash guards the whole snapshot.

## Line-ending policy

A line's per-anchor hash is computed over the line's content **excluding its line terminator**. LF vs CRLF, and a present-or-absent final newline, therefore do not spuriously drift a line whose visible content is unchanged. The line terminator is line-framing metadata, not content, and normalizing it (e.g. an editor's LF↔CRLF conversion) should not invalidate every line's anchor. The **file-level** hash still covers the whole verbatim byte stream — terminators included — so a line-ending change is caught there, under strict file-level gating (ADR-0017). Structural node hashes remain fully verbatim span bytes; this exception is specific to line anchors.

## Status

accepted

## Considered Options

- **Normalized (whitespace-insensitive) hashing** — rejected. It would suppress false drift from formatting churn, but requires a per-dialect definition of "semantically identical," which is complex and fragile across the polyglot set. Strict verbatim keeps drift simple; the cost of a false drift is only a re-read.
- **Longer hashes** — unnecessary. The file-level hash is the global guard, so a short per-anchor hash suffices.

## Consequences

- Whitespace-only reformatting by another tool invalidates anchors and forces a re-read — accepted.
- The per-anchor / file-level split lets errors be reported locally ("node `foo` drifted") or globally.
