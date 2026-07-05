# parinfer command — a native parinfer alternative, balance-generating

lisplens gains its own [parinfer](https://shaunlebron.github.io/parinfer/)-style
transform, `lisplens parinfer <mode>`, built on the existing toolset (the
faithful Emacs indenter, the Nameless model of ADR-0030, and the lispexp
reader/lexer). It is **not** an integration with `parinfer-rust` /
`parinfer-rust-emacs`, and it intentionally does **not** preserve API/interface
compatibility with `parinfer.js` — lisplens becomes its own alternative. The
motivating win: parinfer-rust-emacs ignores Nameless, so it reads indentation
from raw-character columns and mis-handles files edited under Nameless; lisplens
already models Nameless faithfully.

## Execution model

Stateless **whole-buffer transform** — no live editing loop, no dynamic module,
no per-keystroke `changes`/cursor state. The command reads a buffer, returns a
transformed buffer. Interactive editor integration (and the cursor-protection
rules a live loop needs) is deferred to a later, separate effort.

## Surface

A single subcommand with a mode argument, mirrored by one MCP tool.

- **CLI**: `lisplens parinfer <mode>` reads the buffer from **stdin** and writes
  the transformed text to **stdout**. `--json` emits the structured result
  instead. `--dialect` (the global override) selects the language, defaulting to
  **Emacs Lisp** since fileless stdin has no extension. `--nameless` enables the
  Nameless overlay (Emacs Lisp only); `--name NAME` / `--file PATH` supply the
  current-name hint (and `--file` also sources indentation config from
  file-locals / dir-locals / EditorConfig, ADR-0029). `--cursor-line` /
  `--cursor-x` pass an optional 0-based input cursor.
- **MCP**: the `parinfer` tool takes `{mode, text, dialect?, nameless?, name?,
  cursorLine?, cursorX?}` and returns the structured result directly.

**Structured result** (`--json` and the MCP return payload), shared by every
mode: `{text, success, error, cursorX, cursorLine}`. `error` is
`{name, message, line, x}` with 0-based `line`/`x`, or null. In plain CLI mode
the transformed text goes to stdout; on failure the (unchanged) input is still
echoed to stdout — a safe no-op for a stdin→stdout filter — with a stderr
diagnostic and a non-zero exit. In `--json` mode the exit is always 0 and
`success` carries the outcome.

**Cursor** is position-tracking only: an input cursor is reported at its
post-transform position. No cursor-protection semantics (those belong to the
future interactive layer).

## Modes

- **Paren mode** (`paren`) — parens are the source of truth. Require balanced
  input, then reindent to lisplens's faithful Emacs indentation (reusing the
  formatter and its Nameless *production* path — column generation). This is
  close to `format` behaviourally; its role here is to establish the command's
  contract, result type, error taxonomy, MCP wiring, and this ADR. (The classic
  parinfer paren-mode "clamp indentation to a valid range, preserve user style"
  behaviour is deliberately not built — it underuses lisplens's exact-column
  knowledge; a possible future refinement.)
- **Indent mode** — infer close-parens from indentation over a tolerant
  `lispexp::lex()` token scan, with Nameless-aware column *interpretation* (the
  headline win). Planned as issues #25 / #26, not built here.
- **Smart mode** — out of scope: it needs `changes`/cursor history, which the
  stateless model does not carry.

## Safety model

The edit pipeline (ADR-0005) enforces **error-parity**: an edit must not
introduce a parse error the input did not already have. The parinfer command
inverts this — it **generates balance**:

- **Success**: the output parses clean / balanced.
- **Failure** (an unresolvable lexical situation — e.g. unbalanced parens in
  paren mode; unterminated string / eol-backslash / quote-danger in the future
  indent mode): the input is returned **completely unchanged**, with
  `success = false` and a positioned diagnostic.

Broken output is never emitted, so this does not contradict ADR-0005; it reaches
the same "never emit a worse parse" guarantee by a different route (produce
balance, or refuse untouched) suited to a transform whose whole point is to fix
balance rather than preserve it.

## Status

accepted

## Consequences

- New `src/parinfer.rs` engine (`Mode`, `Request`, `Answer`, `Error`, `Cursor`,
  `run`, `answer_to_json`); thin CLI (`run_parinfer`) and MCP (`parinfer`)
  wrappers. Paren mode reuses `format::format` / `format_elisp_nameless`, so the
  faithful indenter and Nameless production come for free.
- The result shape and error taxonomy are fixed now and shared by the future
  indent-mode slices, which extend `Mode`, the tolerant scan, and the failure
  taxonomy without changing the contract.
- Nameless is threaded as an Emacs-Lisp-only overlay via the existing
  `FormatConfig.nameless` + `Nameless::for_file` path. In paren mode it affects
  column *generation*; indent mode will additionally use `Nameless::saving` for
  column *interpretation* (#26).
