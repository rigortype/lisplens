# Separate structural and line-hash edit modes

lisplens exposes two independent editing surfaces: a **Structural mode** (addressing code by definition name, role, or node path via lispexp) and a **Line-hash mode** (hashline-style line number + content-hash anchoring). We deliberately keep them separate rather than unifying them under a single hash-anchored substrate, because there is no decisive reason to prefer one unified model yet. Running both in parallel lets us accumulate real-world best practices before committing to a unification.

## Status

accepted

## Considered Options

- **Unified anchor substrate** — make hashed spans the universal substrate and compile structural addresses down to hash-checked spans, giving one read output and one drift mechanism. Deferred: attractive, but premature without usage data on which mode agents actually reach for.

## Consequences

- Two edit code paths and two read output shapes to maintain in the near term.
- Revisit unification once best practices from real usage emerge.
