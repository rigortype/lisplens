# Native indent engines for Fennel, Janet, Hy, and LFE

## Context

After Clojure (ADR-0039), Phel (ADR-0041), and the induced ISLisp engine (ADR-0042),
the dialects still on the generic Emacs Lisp fallback were Fennel (`.fnl`), Janet
(`.janet`), Hy (`.hy`), and LFE (`.lfe`). The fallback imposes Emacs's
`lisp-indent-function` model on them, which fits poorly (16–20% of code-line
indentation on their own sources). Following the Phel precedent — *check for the
dialect's own formatter first; reverse-engineer from the corpus only if there is
none* — each was investigated:

- **Fennel** ships **`fnlfmt`** (`indentation.fnl`): a fixed set of special forms
  body-indent at `+2`, everything else aligns under arg 0. There is also a Fennel
  style guide. A real formatter → extract its table.
- **Janet** ships **`spork/fmt`**: a `*default-indent-2-forms*` list body-indents at
  `+2`, with `def`/`var`/`with-`/`if-`/`when-` prefix fuzzy-matching; everything else
  aligns under arg 0. A real formatter → extract its table.
- **Hy** has **no canonical formatter** (no `hyfmt`; the community uses Emacs
  `hy-mode` or hand-formatting). → induce from the corpus.
- **LFE** has **no canonical formatter** (a style guide only). → induce from the
  corpus.

Crucially, all four share **one shape**: a *special* head body-indents its children
at `open + 2`, every other head aligns under arg 0. That is exactly the Clojure
engine's `[:inner 0]` rule for the special set plus the default alignment, at the
standard body width **2** — so all four reuse the shared engine (ADR-0039) with only
a per-dialect table, no width override (unlike ISLisp's 4, ADR-0042). Inducing from
each corpus (the same learner as ADR-0042) confirmed body width 2 for all four, and
recovered special sets that — for Fennel and Janet — closely match their formatters'
declared lists, and for Hy and LFE recovered those languages' real special forms
(`defn`/`defclass`/`cond`/`for`/`with`/…; `defun`/`defmodule`/`case`/`receive`/…).

## Decision

Route `Fennel`/`Janet`/`Hy`/`Lfe` to `Engine::Clojure` and add them to
`has_native_engine`. Add four `*_rules_for` tables in `src/format/clojure.rs`, each
mapping its special set to `[:inner 0]` and everything else to the default:

- `fennel_rules_for` — `fnlfmt`'s set plus corpus-attested additions
  (`accumulate`/`case`/`case-try`/`match-try`, …).
- `janet_rules_for` — `spork/fmt`'s `*default-indent-2-forms*` verbatim, with the
  same `def`/`var`/`with-`/`if-`/`when-` prefix fuzzy-match.
- `hy_rules_for` / `lfe_rules_for` — induced from the corpus, pruned of obvious
  function false-positives.

Body width stays the shared default (2), so no dispatch change is needed. Collections
use the shared align-under-first-element rule.

## Status

Accepted — implemented in `src/format/clojure.rs` (`{fennel,janet,hy,lfe}_rules_for`,
dialect-aware `rules_for`) and `src/format/mod.rs` (`engine_for`,
`has_native_engine`). A golden test locks the shared shape for all four (special →
`open + 2`, calls align under arg 0). Corpus self-consistency (code-line indentation,
each formatted from de-indented and compared to the original), vs the old generic
Emacs Lisp fallback:

| dialect | files | native engine | fallback |
| --- | --- | --- | --- |
| Fennel | 91 | **91.7%** | 19.5% |
| Janet  | 210 | **80.2%** | 16.7% |
| Hy     | 76  | **67.3%** | 16.0% |
| LFE    | 68  | **74.4%** | 49.5% |

Fennel is highest because its corpus is `fnlfmt`-formatted and we extract `fnlfmt`'s
table. All four are a large improvement (+25 to +72 points) over the fallback.

## Consequences

- Every dialect lisplens routes by extension now has a native indent engine — the
  Emacs Lisp generic fallback is no longer anyone's *primary* formatter, only a
  safety net for an unrecognised extension.
- Two paths coexist cleanly under one engine: **extract** a table from the dialect's
  own formatter (Fennel/Janet, like Phel from `phel format`) or **induce** it from
  the corpus (Hy/LFE, like ISLisp from EISL). Same `[:inner 0]`-shaped output either
  way.
- Deferred refinements, honest about the residual: **Janet collections** — `spork/fmt`
  body-indents `[…]`/`@[…]`/`{…}`/`@{…}` at `+2`, whereas the shared engine aligns
  them under the first element; teaching the engine a per-dialect collection mode
  would lift Janet's ~80% further. Hy's lower number reflects a less consistently
  formatted corpus (no formatter to normalise it). Per-form nuances (e.g. LFE `if`
  vs a two-space body) remain in the corpus noise.
