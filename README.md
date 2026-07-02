# lisplens

Token-efficient, polyglot Lisp editing for AI agents — a CLI and MCP tool built
on the [lispexp](https://crates.io/crates/lispexp) reader.

Status: **pre-implementation / skeleton.** The design is recorded in
[`CONTEXT.md`](CONTEXT.md) (domain glossary) and [`docs/adr/`](docs/adr/)
(architecture decisions); a cross-dialect dependency-loading survey is in
[`docs/research/`](docs/research/).

## Try it

```sh
cargo run -- outline path/to/file.el
```

Prints a heuristic outline (start line, defining head, name) of a file's
top-level definitions — a first slice of the Lens (ADR-0013).

## License

Licensed under the Mozilla Public License 2.0 (MPL-2.0).
