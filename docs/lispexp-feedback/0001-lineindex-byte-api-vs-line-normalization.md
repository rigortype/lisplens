# Feedback: `LineIndex` mixes a byte-oriented API with a normalized `line_range`

Upstream feedback to [lispexp](https://crates.io/crates/lispexp) from lisplens, a **verbatim / round-trip** consumer (it hashes source bytes — ADR-0008 — and edits by splicing source spans). Severity: **low** (workaroundable; lisplens's own use happens to align), but the API shape invited a bug, so it is worth a doc note or an additive API.

## Confirmed facts (lispexp 0.2.1)

`LineIndex` (ADR-0024) is byte-oriented throughout:

- `offset_to_line_col(offset) -> (line, col)`, `line_col_to_offset(line, col) -> offset` — byte offsets; columns are byte offsets from the line start.
- `line_count()`, and `line_range(n) -> Option<Range<usize>>`.
- Documented line policy: breaks on `\n` and `\r\n` only; a lone `\r` is not a break.
- **`line_range(n)` returns line *content*, excluding the terminator** — and, for `\r\n`, excluding the `\r` too.

lisplens confirmed this satisfies its hashing need, and that `Datum.line` (reader) and `LineIndex` share the same `\n`/`\r\n` numbering, so structural and line-oriented views agree.

## The sharp edge

The API surface is uniformly **byte-oriented** (offsets, byte columns), which signals *losslessness* — yet its most natural "give me line N" accessor, `line_range`, silently returns **normalized content**. Two consequences bite a verbatim consumer:

1. **The ranges do not tile the input.** For `"ab\r\ncd"`, `line_range(1) == 0..2` (`"ab"`) and `line_range(2) == 4..6` (`"cd"`); bytes `2..4` (`"\r\n"`) belong to no line's range. Concatenating `source[line_range(n)]` over all `n` does **not** reconstruct the source — the terminators (and their exact form) are dropped.
2. **A line's terminator is unrecoverable from `line_range` alone.** There is no `line_range`-level way to get a line's verbatim bytes (content + terminator), the terminator's *kind* (LF / CRLF / none-at-EOF), or a full byte span. A consumer reaching for `line_range` to get "line N's bytes" — a very reasonable expectation of a byte index — gets terminator-stripped content instead.

The normalization is *correct* for display or a content hash; the problem is that a byte-offset API is where a consumer least expects silent normalization.

## lisplens's design decision

- **For hashing**, terminator-excluded content is exactly what lisplens wants (ADR-0008 line policy: LF/CRLF and final-newline changes should not drift a line's anchor), so lisplens uses `line_range` directly as the hash input — deliberately.
- **For reconstruction / edits**, lisplens will **not** rely on `line_range` being lossless or tiling. It derives verbatim line bytes (including terminators) from raw source spans (`line_start(n) .. line_start(n+1)`), and keeps a **file-level hash over the whole verbatim byte stream**, so any terminator-only change is still caught even though per-line hashes ignore terminators.

So lisplens is unaffected in practice — but only because it noticed. The next consumer may not.

## Proposed (additive, non-breaking)

Any one of these would remove the footgun without changing `line_range`'s convenient default:

1. **Expose verbatim line access** — e.g. `line_span_full(n) -> Range` (content + terminator, so full ranges tile the input) and/or `line_terminator(n) -> Terminator { Lf, CrLf, None }`.
2. **Guarantee/expose tiling** — a documented way to walk the source as `(content_range, terminator_range)` pairs.
3. **At minimum, document** `line_range` prominently as *normalized content* and *non-tiling*, and point verbatim consumers at `offset_to_line_col` + manual slicing.

`line_range` itself is a good default for the common display/hash case; keep it. The ask is only to make the verbatim path discoverable so a byte-oriented API doesn't quietly lose bytes.
