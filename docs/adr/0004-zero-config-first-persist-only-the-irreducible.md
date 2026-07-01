# Zero-config first; persist only what cannot be inferred

lisplens aims to work with **zero configuration** wherever possible. Dialect and structure are inferred from file extension and content signals (lang line, shebang, and project manifests such as `.asd`, `deps.edn`, `project.clj`, `.dir-locals.el`) rather than requiring the agent to declare them. A per-project configuration file — the **Project profile** — is an escape hatch, not a prerequisite: it holds only the irreducible minimum that genuinely cannot be inferred. Both the stateless CLI and the stateful MCP server read the same profile when it is present; the MCP server may additionally cache resolution in memory.

## Status

accepted

## Considered Options

- **Always-explicit** (the agent declares dialect / implementation on every call) — rejected: token cost and friction, contrary to the product's token-efficiency goal.
- **Persisted profile as the primary source of truth** — rejected: pushes a configuration burden onto every project when most files resolve fine from extension plus content signals.

## Open questions (needs investigation — 実態調査)

- How do the target Lisp implementations resolve **library / dependency loading**, especially when **no package manager** is used? Load paths, `require` / `load` search behavior, and implementation selection may be impossible to infer and are the leading candidates for what the Project profile must persist. A real-world survey across the dialects is required before the profile schema is fixed.

## Consequences

- The default path spends zero tokens on configuration.
- The Project profile schema is intentionally deferred and kept minimal until the library-loading investigation lands.
