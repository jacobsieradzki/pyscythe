# pyscythe

Codebase intelligence for Python. Dead code, circular imports, duplication, complexity, and architecture boundaries in one fast binary. The Python answer to [fallow](https://fallow.tools).

Built in Rust on the [ruff](https://github.com/astral-sh/ruff) and [ty](https://github.com/astral-sh/ty) crates, so name resolution is semantic rather than textual: a symbol is only "used" when a reference actually binds to it.

## Status

Walking skeleton. `pyscythe dead-code` reports module-level functions, classes, and variables that nothing refers to. See [TODO.md](TODO.md) for the roadmap.

## Usage

```bash
pyscythe dead-code path/to/project
pyscythe dead-code path/to/project --format json
```

Exit codes: `0` clean, `1` findings, `2` error.

## Developing

Rust is managed by [mise](https://mise.jdx.dev). Every warning is an error; clippy runs with `pedantic` and `nursery` on.

```bash
mise install
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all --check
```

## Layout

- `crates/pyscythe_core` — domain model and analyses. No parser, no filesystem. Analyses are written against the `CodebaseIndex` port and tested with an in-memory fake.
- `crates/pyscythe_ty` — the adapter that implements `CodebaseIndex` on the ty project database.
- `crates/pyscythe_cli` — the `pyscythe` binary, output formats, and acceptance tests that run the real binary over fixture projects.
