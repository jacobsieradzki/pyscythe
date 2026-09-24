# Roadmap

Ordered roughly by value. Each item lands with an acceptance test first.

## Dead code

- [x] Unused imports and phantom `__all__` entries: left to ruff (F401, F822), which every project already runs. Names in `__all__` are treated as exports and kept.
- [x] Unused methods and properties. Resolved references plus a textual attribute-name safety net for duck typing; overrides of inherited members (walked through ty, so stdlib and third-party bases count) are kept; `.setter`/`.deleter` halves are not candidates.
- [x] Unused class attributes and instance fields (`unused-attribute`): what a class body assigns and what its methods assign to `self`. A bare `name: int` declares a type and creates nothing. Kept: overrides of an inherited attribute, and the fields of a class whose attributes are its data (dataclass, enum, `NamedTuple`/`TypedDict`/`Protocol`, Pydantic/SQLModel, SQLAlchemy declarative, Django and DRF classes, GraphQL schema types, inner `Meta`/`Config`). Added 2026-09-16.
- [x] Unused files: modules nothing imports (including by dotted string) and nothing runs. Roots: `__main__` guard, package/test/conftest/manage/wsgi/asgi files, entry-point modules, files with plugin-kept symbols.
- [x] `pyscythe deps` (per nested `pyproject.toml` incl. Poetry, `setup.py`, `setup.cfg`, or `requirements*.txt`): unused declared dependencies, imported-but-undeclared distributions (owner read from site-packages `RECORD`/`top_level.txt`), and unresolved imports; PEP 508 names, optional-dependencies, PEP 735 groups with include-group; tool distributions and `[tool.pyscythe.deps] ignore` are exempt; imports reachable through a declared dependency's `Requires-Dist` are low confidence; typeshed-bundled stubs such as typing_extensions still count as dependencies.
- [x] Entry points from `[project.scripts]`, `[project.gui-scripts]`, `[project.entry-points]`, and from a setuptools project's `setup(entry_points=...)` (dict or INI string) and `[options.entry_points]` in `setup.cfg`.
- [x] Framework plugins (convention-based, by decorator name, base class name, and file layout): FastAPI, Pydantic, Typer/Click, pytest, Flask, Airflow, Django, Celery, SQLAlchemy/SQLModel, Alembic, Graphene/GraphQL, Textual, Ansible.
- [x] Base-class identity resolved through ty: plugins match qualified ancestors such as `sqlalchemy.orm.decl_api.DeclarativeBase` and only fall back to base-name text when a base cannot be resolved.
- [x] Decorator identity resolved through ty: each decorator carries the module that defines it, and plugins only accept decorators from their own packages (unresolved ones still match by name).
- [x] String references: a literal `"pkg.module.attr"` or `"pkg.module:attr"` anywhere in the project counts as a use of the symbol and a deferred import of the module.
- [x] Django `INSTALLED_APPS`, middleware, and context-processor strings are covered by dotted-string references.
- [x] `--show-kept` lists what plugins suppressed and why; JSON always carries `kept`.
- [x] `--exclude` on the command line; `--timings` prints phase durations.
- [x] `[tool.pyscythe]` config: `exclude`, `ignore-names`, `entry-points`, `include-notebooks`.
- [x] Inline suppression: `# pyscythe: ignore`, `# pyscythe: ignore[rule, ...]` on the definition line or the line above, `# pyscythe: ignore-file`.
- [x] Stale `# pyscythe: ignore` comments are reported as `unused-suppression`.
- [x] Baseline: `--write-baseline FILE` records current findings by rule, relative path, symbol, and owner (no line numbers); `--baseline FILE` hides them and exits 0 when nothing new.
- [x] `--min-confidence low|medium|high`.
- [x] `--since <ref>` keeps findings in files changed since a git ref, plus untracked files.
- [x] Performance: single-pass inverted reference index built in parallel (also fixed aliased-import misses).
- [x] Perf: the process forgets the salsa database at exit instead of dropping it (ty does the same), and the project is opened once. 64 files in ~180 ms wall.

## Other analyses

- [x] Circular imports: `pyscythe cycles`, Tarjan SCCs over runtime imports; `TYPE_CHECKING` and function-local imports are excluded.
- [x] Cycles: every simple cycle per component (capped at 25), `--include-deferred`.
- [x] `__init__.py` re-exports count as uses of the re-exported module and symbol (covered by an acceptance test).
- [x] `pyscythe dupes`: windowed token hashing with maximal-match extension; `--mode strict|mild|weak`, `--min-tokens`, `--min-lines`; duplication percentage in the summary.
- [x] Dupes: clone groups (three or more occurrences) are one finding listing every place; docstrings and import statements are never clone material.
- [x] `pyscythe health`: cyclomatic and cognitive complexity per function (`pyscythe_metrics`, parser-only), hotspots over 10/15, length-weighted 0-100 score with A-F grade.
- [x] Health thresholds in `[tool.pyscythe.health]`.
- [x] Health: per-file and per-package scores, radon-style maintainability index; trend is the JSON `score` diffed across runs.
- [x] `pyscythe boundaries`: `layers` (ranks, several prefixes per rank), `rules` with `from`/`deny`, `preset = "hexagonal"` with `root`, type-only imports allowed unless `check-type-only`.
- [x] `pyscythe boundaries --suggest` proposes layers from second-level package imports (tangles called out); rules take `allow` lists.
- [x] `--config FILE` reads `[tool.pyscythe]` from elsewhere, for a project whose checkout must stay as published or whose rules differ between CI and a desk. One file depending on another is one violation however many times it names it.
- [x] `pyscythe fix [--dry-run]`: removes dead definitions (whole lines, decorators included, gap preserved) and unused files; skips nested definitions and methods whose removal would empty a class; medium confidence and better by default.
- [x] Fix drops imports orphaned by a removal and takes `--only RULE,...`. Interactive confirmation is not planned: `--dry-run` plus git is the review step.
- [x] `pyscythe fix --format json` reports the plan instead of a diff, so the corpus snapshots what fix removes from every pinned project. `pyscythe-corpus repair` carries the plan out for real and checks every package the project ships still imports, then restores the checkout; it runs on Linux in CI, where the pinned wheels can actually be imported. Added 2026-09-16.

## Output and integration

- [x] `--format sarif|github|markdown` alongside `human` and `json`.
- [x] `--format pr-comment`: Markdown with a `<!-- pyscythe:<analysis> -->` marker.
- [x] Composite GitHub Action (`action.yml`) that installs from git and runs any analysis with annotations or SARIF; CI workflow for this repo.
- [x] Corpus: 36 public projects pinned by commit in `corpus/corpus.toml`, environments pinned in `corpus/locks`, every analysis snapshotted in `corpus/snapshots`; `mise run corpus` and the `corpus` workflow fail on any change in output. Added 2026-09-13. `boundaries` runs for a project that states its own architecture, with the layers in `corpus/configs/<name>.toml`; kedro's come from its import-linter contracts.

## Deferred by decision

- [x] Distribution: PyPI wheel via maturin (`release.yml` on `v*` tags, trusted publishing, GitHub Release), `cargo install --git`. First release v0.1.0 on 2026-09-13.
- [x] Homebrew, from a tap of its own (`jacobsieradzki/homebrew-tap`), fed the per-platform archives the release repacks out of the wheels rather than a second build of the same code. Added 2026-09-24.
- [x] pre-commit hooks, from a mirror repository (`jacobsieradzki/pyscythe-pre-commit`), because pre-commit installs the hook repository itself and this one's `pyproject.toml` is maturin's. Added 2026-09-24.
- [ ] conda-forge: the recipe is written and rendered by `packaging/conda.sh`; it waits on a tag whose tarball contains the LICENSE, then a pull request to `conda-forge/staged-recipes`.
- [ ] GitHub Marketplace: a checkbox on a GitHub Release. `action.yml` already has the name, description, and branding it asks for.
- [ ] crates.io: `cargo publish` refuses a git dependency, and `ty_project` and `ty_ide` are still not on crates.io, though the rest of the ruff and ty crates are, at 0.0.14 (checked 2026-09-24).
- [ ] A `curl | sh` installer: the release archives are what it would serve.
- [ ] Agent integration: MCP server, Claude Code skill, `actions[]` in JSON. (Not a priority as of 2026-09-05.)
- [ ] Runtime layer from coverage.py data.
