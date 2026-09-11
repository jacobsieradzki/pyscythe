# pyscythe

Codebase intelligence for Python. Dead code, circular imports, duplication, complexity, and architecture boundaries in one fast binary. The Python answer to [fallow](https://fallow.tools).

Built in Rust on the [ruff](https://github.com/astral-sh/ruff) and [ty](https://github.com/astral-sh/ty) crates, so name resolution is semantic rather than textual: a symbol is only "used" when a reference actually binds to it.

## Status

Early. `pyscythe dead-code` reports functions, classes, methods, properties, variables, and whole files that nothing refers to, resolving references semantically through ty (aliased imports, attribute access, re-exports). Dotted strings such as `"pkg.settings.DEBUG"` count as references. Methods that override an inherited member (walked through ty, so library bases count) are kept, and any attribute name accessed anywhere keeps same-named methods as a duck-typing safety net. `pyscythe cycles` reports import cycles that would bite at load time. `pyscythe health` measures cyclomatic and cognitive complexity per function, lists hotspots, and grades the codebase 0 to 100. `pyscythe dupes` finds copied code, including renamed copies, and reports the duplicated percentage. `pyscythe boundaries` enforces layering rules from `pyproject.toml`. `pyscythe fix --dry-run` shows the diff that would delete the dead code, and `pyscythe fix` applies it. `pyscythe deps` compares what `pyproject.toml` declares with what the code imports, using the installed environment's metadata when there is one. Built-in plugins keep symbols that frameworks reach by convention: pyproject entry points, pytest, FastAPI, Flask, Click/Typer, Celery, Airflow, Django, Alembic, SQLAlchemy/SQLModel, Pydantic. See [TODO.md](TODO.md) for the roadmap.

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

Rust is managed by [mise](https://mise.jdx.dev). Every warning is an error; clippy runs with `pedantic` and `nursery` on.

```bash
mise install
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all --check
```

## Layout

- `crates/pyscythe_core` — domain model and analyses. No parser, no filesystem. Analyses are written against the `CodebaseIndex` port and tested with an in-memory fake.
- `crates/pyscythe_ty` — the adapter that implements `CodebaseIndex` on the ty project database, including the parallel reference index.
- `crates/pyscythe_pyproject` — reads `pyproject.toml` into the manifest and config.
- `crates/pyscythe_metrics` — per-function complexity from the ruff AST alone, unit-tested on snippets.
- `crates/pyscythe_cli` — the `pyscythe` binary, output formats, and acceptance tests that run the real binary over fixture projects.
