# A `docstring` refactoring primitive: set/replace a definition's docstring

## Context

The agent-skill benchmark (`docs/notes/20260704-skill-benchmark-agent-edit-procedures.md`,
`20260705-lisplens-skill-report.md`) surfaced three universal Lisp-edit shapes:
file-wide symbol rename, scoped rename, and **adding a docstring**. The first two
map onto existing primitives (`rename`, and the `struct edit` `rename` verb,
ADR-0032). Adding a docstring had **no primitive** — agents did it by `replace`-ing
the whole enclosing form (a bigger, more error-prone payload than the edit
warrants), and one run hit `BadOp` trying to `insert-after` an inner arglist node
before falling back to `replace`.

"Give this definition a docstring" is mechanical, safe, and common enough
(especially in Emacs Lisp / Common Lisp, where nearly every public defun and
defvar carries one) to deserve a first-class, self-verifying operation in the
ADR-0032 family.

## Decision

Add `lisplens docstring <name> <file>`, with the docstring **text on stdin**
(raw — lisplens escapes it into a string literal, so the caller never hand-quotes
and can't unbalance the parens). Like the other ADR-0032 procedures it resolves
the target structurally, applies one edit through the safety pipeline (splice →
native reindent → validate-then-write → atomic), and prints a post-condition
(`set docstring on <name>` / `replaced docstring on <name>` + the new file hash).

**v1 scope — function-like definitions**, where the docstring slot is
unambiguous: the form right after the argument list. Covers `defun`, `defsubst`,
`defmacro`, their `cl-` variants, and Scheme's `(define (name params…) …)`. The
slot rule mirrors `finish_body` (ADR-0032): the element after the arglist is an
existing docstring **only** if it is a string *and* the body has more forms after
it; a lone string body is a return value, so a docstring is inserted before it,
not a replacement of it.

- **Insert** when there is no docstring: an empty-range edit right after the
  arglist span; native-engine dialects (ADR-0031) then reindent the touched form
  so the new line lands at the right column, others stay verbatim (ADR-0027).
- **Replace** when one exists: replace the existing docstring datum's span.

**v2 — Elisp variable definitions** (`defvar`, `defvar-local`, `defconst`,
`defconstant`, `defcustom`, `defparameter`). The docstring follows the *value*
form (`(defvar x VALUE "doc" …)`), so the slot is the element after the value and
an existing string there is the docstring (no lone-string ambiguity — the value
is separate). `defcustom`'s trailing `:keyword` args are preserved (the docstring
is inserted before them).

Refusals (no partial write, ADR-0005 spirit): `name` not found; defined more than
once (ambiguous); a variable declared with **no value** (`(defvar x)`) — no slot
to attach after (`NoValue`); a definition with no docstring convention such as a
Scheme `(define name value)` (`NoDocstringSlot`); empty stdin.

## Consequences

- Closes the last of the three benchmark shapes; the skill can teach `docstring`
  alongside `rename`/`inline` instead of the `replace`-the-whole-form workaround,
  and the `insert-*`-into-a-form `BadOp` friction stops mattering for this case.
- Function-like and Elisp variable definitions are both covered. Scheme/Clojure
  *value* definitions (`(define name value)`, `def`) have no docstring slot and
  are refused; a metadata/attribute-map convention for them could be a later
  extension, but is out of scope here.
- No new edit machinery: it is a span→edit composition over `edit`/`format`/
  `write` like every other member, and reuses the definition-classification shape
  from `refactor.rs`.

## Status

accepted
