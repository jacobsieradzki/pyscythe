# pyscythe

Codebase intelligence for Python, in Rust, on the ruff and ty crates. See README.md and TODO.md.

## How we work here

- **GOOS.** Walking skeleton first, then outside-in. Every feature starts with a failing acceptance test in `crates/pyscythe_cli/tests` that runs the real binary over a fixture project, then unit tests in `pyscythe_core` against the `FakeIndex`. Listen to the tests: if something is hard to test, the design is wrong.
- **Ports and adapters.** `pyscythe_core` never imports ruff or ty. Anything that touches a parser, the filesystem, or the ty database lives behind the `CodebaseIndex` trait in `pyscythe_ty`. Tell, don't ask: analyses ask the index questions, they do not reach into its internals.
- **Strong types.** Newtypes for ids, offsets, lines, columns, names, and module paths. Enums instead of strings or booleans. No `String` where a domain type exists.
- **Strictest lints.** `-D warnings` is set in `.cargo/config.toml`; clippy runs `pedantic` and `nursery`; `unwrap`, `expect`, `panic`, and indexing are lint errors outside tests. Fix the code, do not add `#[allow]`. If a lint is genuinely wrong for the whole project, disable it once in `Cargo.toml` with a comment saying why.

## Commands

Rust comes from mise (`mise install`). Run cargo via `mise exec -- cargo ...` if the shell has not activated mise.

```bash
mise run check      # fmt, clippy, tests, rustdoc, cargo shear, typos: the CI gate
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```

Patch scripts that edit Rust by exact-string replacement must assert every replacement landed; rustfmt reflows text and silently defeats them otherwise. Never chain a commit after a piped cargo command: the pipeline's exit status hides failures.

## Dependencies

The astral crates are git dependencies pinned to one ruff commit in the workspace `Cargo.toml`. Bump them all together.
