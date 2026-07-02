# Strict file-level drift gating by default

An edit is accepted only if the file's current **file-level hash** matches the one the read it was based on carried. That is, drift is gated **strictly at the whole-file level** by default: if anything in the file changed since the read, the edit is refused and the agent must re-read — exactly hashline's "changed → re-read" contract.

The two-tier scheme (ADR-0008) keeps a per-anchor hash as well, but under strict gating the per-anchor hash is used to **identify and report** the target site (e.g. "line 12 / node `foo` — hash `a3f2`"), not to widen acceptance. A **relaxed** mode — accept an edit when the file drifted but the specific anchor's content is provably intact — is a deliberate **future opt-in**, not the default.

## Status

accepted

## Considered Options

- **Relaxed per-anchor acceptance by default** — rejected for now. It is attractive (an untouched region should be editable even if elsewhere changed), but a 4-hex per-anchor hash is only 16 bits: a ~1/65536 per-site collision chance is too weak to *gate* a write without the file-level guard behind it. Under strict gating the file-level hash (64-bit) is the real guarantee, so the short per-anchor hash never has to carry safety alone.

## Consequences

- Simple, predictable contract: any concurrent change to the file forces a re-read before editing — matching Line-hash mode's hashline lineage and keeping Batch application (ADR-0006) easy to reason about.
- Resolves the ambiguity left by ADR-0007/ADR-0008 over whether the two tiers gate strictly or relaxed, before Batch application is built.
- If real usage shows strict gating is too coarse (e.g. large files edited by several agents at once), a relaxed opt-in can be added without changing the default or the on-the-wire hashes.
