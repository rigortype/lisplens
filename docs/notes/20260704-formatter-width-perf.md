# Formatter column-width: display-width vs byte, and why the ASCII fast path was reverted

2026-07-04. Context for `format::Cols::col` (see the comment there). Records a
perf measurement so we don't re-litigate the ASCII fast-path idea.

## Background

`Cols::col` computes an element's output column. It used to measure the line
content up to the position by **UTF-8 byte length**, which mis-indents any line
where a wide/multi-byte glyph precedes an alignment column — Emacs aligns by
**display width** (`current-column`), not bytes. Fix (ADR-0031): measure the
content slice by East Asian Width via `unicode-width`. `漢`/`Ａ` = 2, `λ`/`☆`
(ambiguous) = 1, matching Emacs's default; ASCII is unchanged (width == bytes).

That fix makes `col` scan the content slice with `unicode-width` on **every**
call (it is called several times per code line). So we tried two fast paths to
keep the old O(1) byte arithmetic on ASCII:

1. **per-line** — precompute a `Vec<bool>` of "is this line ASCII"; on an ASCII
   line use `(byte col) − old_indent` directly, run `unicode-width` only on
   non-ASCII lines.
2. **whole-file** — `source.is_ascii()` once; if the file is all ASCII, build no
   per-line table at all and always take the byte path.

## Measurement

In-process microbench (release, `perf_counter`, N = 400, medians), so process
startup + I/O are excluded and parse is decomposed out. Target:
`~/local/src/emacs/lisp/progmodes/cc-engine.el` — 620 KB, 16 733 lines, longest
line 101 B, **14 non-ASCII lines** (so `source.is_ascii()` is false: the
whole-file shortcut does *not* fire; it falls to the per-line path). Plus an
all-ASCII copy (`cc-ascii.el`, non-ASCII bytes → `x`) to exercise the shortcut.

`indent` = `format` − `parse`, in ms/iter:

| col() variant | cc-engine.el (14 non-ASCII lines) | cc-ascii.el (all ASCII) |
| --- | --- | --- |
| byte (pre-fix, wrong output) | 12.86 | 12.47 |
| **naive display-width** (every call) | 12.95 (+0.7 %) | 12.81 (+2.7 %) |
| per-line fast path | 12.70 | 12.61 |
| whole-file shortcut | 12.84 | 12.70 |

End-to-end (CLI, `format` subcommand, 40 runs incl. startup + parse + I/O): all
four variants sit within noise at ~28–34 ms — the col() difference is invisible
there.

## Findings

- **The width approach barely matters.** Even the naive version (unicode-width on
  every call, incl. pure ASCII) costs only **+0.1–0.35 ms** on a 620 KB file —
  ~1–3 % of the indent pass, ≪1 % of a full format. The fast paths recover
  byte-baseline speed but save only that sliver.
- **Why so small:** `unicode-width`'s `width()` already fast-handles ASCII, so
  the byte fast path removes little; and the per-line table build is cheap even
  at 16 733 lines.
- **The real cost is tree traversal, not width.** `indent` (~12.7 ms) is ~3× the
  parse (~4 ms). The dominant term is `container_at` re-descending the parse tree
  **from the root for every line** to find the innermost containing list —
  O(lines × depth). If the formatter ever needs to be faster, that (carry a
  container stack in one pass) is the target, not `col`.

## Decision

Keep `Cols::col` as the **plain display-width** version (no fast path). The
correctness fix stays; the fast-path state (a `Vec<bool>` field, an `is_ascii`
pre-scan, a branch in the hot loop) is not worth ≪1 %. The fast-path exploration
was squashed out of the shipped history, so this note + the `Cols::col` comment
are its only durable record.
