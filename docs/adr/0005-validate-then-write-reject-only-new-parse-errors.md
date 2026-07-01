# Validate-then-write: reject only newly-introduced parse errors

Every edit — in both Structural mode and Line-hash mode — is applied in memory, then the result is re-parsed with lispexp before anything is written. lisplens takes the pre-edit parse errors as a **baseline** and rejects the edit only if it **introduces new errors** (e.g. paren imbalance, unterminated string). Edits that repair an already-broken file are therefore never blocked, while edits that break a working file are. On success the file is written **atomically** (temp file + rename). A **drift check** (content hash) runs before applying; a stale anchor is rejected and the agent must re-read.

Degradations at the Form-annotator level — parens still balance, but a touched definition no longer matches a known form spec — are surfaced as **warnings on an otherwise-successful write**, not hard rejects, consistent with the best-effort, form-annotator-level semantic ceiling (ADR-0003).

## Status

accepted

## Consequences

- The safety contract is simply: **never make a file's syntax worse.**
- Validation needs may drive feature additions on the lispexp side (e.g. a cheaper error-set diff, or incremental re-parse).
