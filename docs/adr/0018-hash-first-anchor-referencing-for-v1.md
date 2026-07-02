# Hash-first anchor referencing for v1

For v1, an edit references its target by the **content hash the read emitted** — a `line-number + line-hash` in Line-hash mode, a `node-hash` in Structural mode — gated by the file-level hash (ADR-0017). The read is self-anchoring (ADR-0013) and AFK agents always read before editing, so passing back the hash the read just gave is the cheapest, least-ambiguous reference.

The S-expression **Structural address** (name / role / node path, with keywords like `:nth` / `:dispatch` / `in` — ADR-0016) is **deferred**. It is the stable, human-legible reference for addressing *without* a fresh read; it can be added later without changing hash referencing, so its concrete keyword spellings need not be finalized for v1.

## Status

accepted

## Consequences

- The CLI/patch and MCP edit surface can be designed entirely around hash references now; the S-expr address keywords stay deferred.
- An agent must read before editing (already the expected flow); there is no read-free, by-name edit in v1.
- **Collision handling.** A 4-hex hash can collide within a file. A Line-hash anchor is disambiguated by its line number; a Structural anchor is carried as `line + hash` (both shown in the Outline), with a small ordinal added by the read only on the rare same-line collision. The exact wire encoding is pinned with the batch format (next).
