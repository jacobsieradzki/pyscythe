# pyscythe

Codebase intelligence for Python. Dead code, circular imports, duplication, complexity, and architecture boundaries in one fast binary. The Python answer to [fallow](https://fallow.tools).

Built in Rust on the [ruff](https://github.com/astral-sh/ruff) and [ty](https://github.com/astral-sh/ty) crates, so name resolution is semantic rather than textual: a symbol is only "used" when a reference actually binds to it.

## Status

Early. `pyscythe dead-code` reports functions, classes, methods, properties, variables, and whole files that nothing refers to, resolving references semantically through ty (aliased imports, attribute access, re-exports). Dotted strings such as `"pkg.settings.DEBUG"` count as references. Methods that override an inherited member (walked through ty, so library bases count) are kept, and any attribute name accessed anywhere keeps same-named methods as a duck-typing safety net. `pyscythe cycles` reports import cycles that would bite at load time. `pyscythe health` measures cyclomatic and cognitive complexity per function, lists hotspots, and grades the codebase 0 to 100. `pyscythe dupes` finds copied code, including renamed copies, and reports the duplicated percentage. `pyscythe boundaries` enforces layering rules from `pyproject.toml`. `pyscythe fix --dry-run` shows the diff that would delete the dead code, and `pyscythe fix` applies it. `pyscythe deps` compares what `pyproject.toml` (PEP 621 or Poetry tables, or `setup.py`, `setup.cfg`, or pip `requirements*.txt` files) declares with what the code imports, per nested project in a monorepo, using the installed environment's metadata when there is one; an import that only arrives through a declared dependency (`starlette` via `fastapi`) is reported at low confidence. pytest's `python_files`, `python_classes`, and `python_functions` are honoured from `pytest.ini`, `pyproject.toml`, `tox.ini`, or `setup.cfg`. Built-in plugins keep symbols that frameworks reach by convention: pyproject entry points, pytest, FastAPI, Flask, Click/Typer, Celery, Airflow, Django, Graphene/GraphQL, Alembic, SQLAlchemy/SQLModel, Pydantic, Home Assistant. A project whose package is named after the distribution (`django`, `langchain_classic`) is a library, and its unimported modules are reported at low confidence since they are its users' entry points. See [TODO.md](TODO.md) for the roadmap.

## Install

The binary ships as a wheel on PyPI, so any Python tool runner works, and no Python is needed at runtime:

```bash
uvx pyscythe dead-code .            # run without installing
uv tool install pyscythe            # or: pipx install pyscythe
cargo install --locked --git https://github.com/jacobsieradzki/pyscythe pyscythe   # from source
```

Wheels are built for Linux (x86_64, aarch64), macOS (Intel, Apple silicon), and Windows (x86_64). crates.io is not an option while the ruff and ty crates are git dependencies.

## Usage

```bash
pyscythe dead-code path/to/project
pyscythe dead-code path/to/project --format json
pyscythe dead-code path/to/project --no-plugins   # report every unreferenced symbol
pyscythe dead-code path/to/project --show-kept    # list what plugins suppressed, and why
pyscythe dead-code path/to/project --exclude scripts --exclude "**/legacy_*.py"
pyscythe cycles path/to/project
```

Confidence: `high` for private module-level names, `medium` for public ones and private methods, `low` for public methods, where overriding or reflection could hide a use.

Configuration lives in `pyproject.toml`:

```toml
[tool.pyscythe]
exclude = ["scripts", "**/legacy_*.py"]   # left out of reports; their references still count
ignore-names = ["deprecated_*"]           # symbols never reported
entry-points = ["pkg.worker:run"]         # extra roots beyond [project.scripts]
include-notebooks = false

public-modules = ["mylib"]         # a library: public names in these modules are API, not dead

[tool.pyscythe.deps]
ignore = ["some-pytest-plugin"]   # declared on purpose, never imported

[tool.pyscythe.health]
max-cyclomatic = 10
max-cognitive = 15
max-lines = 50
max-parameters = 6

[tool.pyscythe.boundaries]
layers = ["app.api", ["app.services", "app.workers"], "app.domain"]  # top to bottom
# or: preset = "hexagonal" with root = "app"

[[tool.pyscythe.boundaries.rules]]
from = "app.domain"
deny = ["app.infra"]
```

Exit codes: `0` clean, `1` findings, `2` error.

### In CI

As a GitHub Action, which installs the PyPI wheel with uv (the action itself lives in this repository, so the runner needs access to it while the repository is private):

```yaml
- uses: jacobsieradzki/pyscythe@main
  with:
    command: dead-code
    version: 0.1.0                      # omit for the latest release
    since: origin/${{ github.base_ref }}
    args: --min-confidence medium
```

```bash
pyscythe dead-code --format github          # GitHub Actions annotations
pyscythe dead-code --format sarif > out.sarif
pyscythe dead-code --format pr-comment      # a table with a marker a bot can find and update
pyscythe dead-code --min-confidence medium  # skip low-confidence method findings
pyscythe dead-code --since origin/main      # only files this branch changed
```

Adopting on an existing codebase: record what is there today, then fail only on new findings.

```bash
pyscythe dead-code --write-baseline .pyscythe-baseline.json
pyscythe dead-code --baseline .pyscythe-baseline.json
```

Silence a single finding where it happens:

```python
def kept_for_a_reason() -> None:  # pyscythe: ignore
    ...

# pyscythe: ignore[unused-method]
def another() -> None:
    ...
```

`# pyscythe: ignore-file` anywhere in a file silences the whole file. A comment that silences nothing is itself reported, so suppressions do not rot.

## Developing

Rust is managed by [mise](https://mise.jdx.dev), which also installs `cargo-shear` and `typos`. Every warning is an error; clippy runs with `pedantic` and `nursery` on, rustdoc denies broken links, `cargo shear` fails on unused dependencies, and `typos` checks spelling. `mise run check` runs the whole gate, the same one CI runs.

```bash
mise install
mise run check
```

For Rust, the tools that map onto oxlint, oxfmt, and fallow are clippy, rustfmt, and rustc's own `dead_code` and `unreachable_pub` lints plus `cargo shear`; all are wired in here.

### The corpus

`corpus/corpus.toml` pins 35 public projects (Django, Home Assistant, pandas, pydantic, ansible, saleor, and others chosen for the shapes they add) at a commit each, with the environment for each pinned in `corpus/locks`. `mise run corpus` clones them into `corpus/cache`, builds their environments with uv, runs every analysis, and compares the reports with `corpus/snapshots`: one sorted line per finding, paths relative to the project. The `corpus` workflow does the same on every push and pull request, so a change in output over real code is always a visible diff in the pull request.

```bash
mise run corpus                              # check every project
mise run corpus -- --project django          # one project
mise run corpus -- --shard 2/6               # one slice, as the CI matrix runs them
mise run corpus:update                       # accept a deliberate change
cargo run -p pyscythe_corpus -- lock --project django   # re-resolve an environment
```

When a rule changes, the snapshot diff is the review: read it, make sure every line that moved is one you meant to move, then update. To add a project, add its entry to the manifest, run `lock` for it, then `update`.

### Releasing

Bump `version` in the workspace `Cargo.toml`, commit, then tag and push:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

The `release` workflow builds the wheels and sdist with maturin, publishes them to PyPI through trusted publishing (the `pypi` environment, no token stored anywhere), and attaches the same files to a GitHub Release. `uvx maturin build --release` builds a wheel locally into `target/wheels`.

## Layout

- `crates/pyscythe_core` — domain model and analyses. No parser, no filesystem. Analyses are written against the `CodebaseIndex` port and tested with an in-memory fake.
- `crates/pyscythe_ty` — the adapter that implements `CodebaseIndex` on the ty project database, including the parallel reference index.
- `crates/pyscythe_pyproject` — reads `pyproject.toml` into the manifest and config.
- `crates/pyscythe_metrics` — per-function complexity from the ruff AST alone, unit-tested on snippets.
- `crates/pyscythe_cli` — the `pyscythe` binary, output formats, and acceptance tests that run the real binary over fixture projects.
