# `rewrite` — structural pattern→template rewriting

`lisplens rewrite <file>` (and the MCP `rewrite` tool) applies an s-expr
**pattern → template** rewrite everywhere it matches in a file — a *structural
sed*. Design and rationale: **ADR-0033**; vocabulary in `CONTEXT.md`.

> **It is not behaviour-preserving.** Unlike `rename`/`inline`, a rewrite can
> change meaning (removing a guard drops the guard; folding a call may duplicate
> an operand). lisplens guarantees only that the result **still parses** and that
> matching is **exactly structural** (never a substring, and it can't confuse
> `foo` with `foo-bar`). *You* assert the rewrite is valid; the metavariable
> classes below are your tool for saying when it is.

## Invocation

The spec is read from **stdin**:

```
[@ <file-hash>]
pattern <<TAG
<pattern s-expr>
TAG
template <<TAG
<template s-expr>
TAG
```

```sh
printf 'pattern <<P\n(when $flag $body)\nP\ntemplate <<T\n$body\nT\n' \
  | lisplens rewrite foo.el
# → rewrote 3 site(s)  <new-file-hash>
```

- The heredoc `TAG` is yours (like the patch DSL) so a marker word in the body
  can't terminate the block.
- `@ <file-hash>` is **optional**: with it you get strict drift rejection (the
  file must match the hash); without it, rewrite reads-and-edits in one shot and
  re-running the same spec is idempotent.
- **Zero matches is success** (exit 0, `rewrote 0 site(s)`).
- Both pattern and template are parsed in the **file's dialect**.

## Pattern language

Everything in a pattern is a **literal** matched structurally, except
metavariables:

| syntax | meaning |
| --- | --- |
| `$name` | capture the single form here |
| `$_` | wildcard — match one form, capture nothing |
| `$name...` (≡ `$name ...`) | sequence — capture a contiguous run of sibling forms (one per list, must be last) |
| `$name:class` | capture, but only if the form is of `class` |
| `$$foo` | a literal `$foo` (escape, for code that really uses `$`-symbols) |

A repeated metavariable is **non-linear**: `(eq $x $x)` matches only when both
arguments are structurally equal. In the template, a metavariable expands to the
**verbatim source text** it captured (comments and formatting inside it are
preserved; the touched form is then reindented).

### Metavariable classes

A syntactic filter — the match fails if the form is not of the class. Use it to
gate a rewrite that would duplicate or move an operand.

| class | matches |
| --- | --- |
| `any` (default) | any form |
| `atom` | a symbol, keyword, number, string, char, or boolean |
| `lit` | a number/string/char/boolean, or a plain-quoted datum `'x` (no variable) |
| `sym` | a bare symbol |
| `list` | a compound form / call `(…)` |

## Cookbook

Each is a stdin spec plus a before → after. All are verified.

**Remove a guard** (behaviour-changing — drops the condition):
```
pattern <<P
(when $flag $body)
P
template <<T
$body
T
```
`(when ready (do-it))` → `(do-it)`

**Unwrap a `progn`** (sequence metavariable):
```
pattern <<P
(progn $body...)
P
template <<T
$body...
T
```
`(progn (a) (b) (c))` → `(a) (b) (c)`

**`if` with a `nil` else → `when`** (behaviour-preserving):
```
pattern <<P
(if $c $a nil)
P
template <<T
(when $c $a)
T
```
`(if c a nil)` → `(when c a)`

**`if (not …)` → `unless`**:
```
pattern <<P
(if (not $c) $a)
P
template <<T
(unless $c $a)
T
```
`(if (not ready) (wait))` → `(unless ready (wait))`

**Drop a redundant `identity`**:
```
pattern <<P
(identity $x)
P
template <<T
$x
T
```
`(f (identity a) (identity (g b)))` → `(f a (g b))`

**Fold a call, safely** — the `:atom` class stops it from duplicating a
side-effecting argument:
```
pattern <<P
(double $n:atom)
P
template <<T
(* 2 $n)
T
```
`(list (double 100) (double (getnum)) (double x))` →
`(list (* 2 100) (double (getnum)) (* 2 x))` — the `(getnum)` call is left alone.

**Reorder / rename a call**, keeping trailing arguments with a sequence:
```
pattern <<P
(make $a $rest...)
P
template <<T
(make/2 $a $rest...)
T
```
`(make x :k1 1 :k2 2)` → `(make/2 x :k1 1 :k2 2)`

**Wrap an argument**:
```
pattern <<P
(emit $x)
P
template <<T
(emit (escape $x))
T
```
`(emit user-input)` → `(emit (escape user-input))`

**Delete matched forms** — an empty template removes them:
```
pattern <<P
(debug-log $_)
P
template <<T
T
```
`(progn (debug-log x) (real))` → `(progn  (real))`

## What to watch for

- **Verify the count.** `rewrote N site(s)` and (with `--json`-style tooling) the
  diff are your check that the match hit what you meant.
- **Quoted data matches too.** The whole tree is searched, so a pattern like
  `(when $f $b)` also matches inside `'(when x y)` data. Make the pattern specific,
  or review the sites.
- **Duplication is on you.** A metavariable used twice in the template duplicates
  its text (and its side effects) — guard with `:atom`/`:lit`.
- **Comments outside a capture are lost;** comments *inside* a captured form
  survive.
- **One outermost pass.** Nested matches (`(progn (progn x))`) are rewritten one
  layer per run — run again to go deeper.
- **Single file, single form.** A pattern matches one sub-form; project-wide scope
  and adjacent-form patterns are future work.
- **No normalization.** `'x` ≠ `(quote x)`, `1` ≠ `1.0`, and (for now) CL `FOO` ≠
  `foo` — patterns match as written.
