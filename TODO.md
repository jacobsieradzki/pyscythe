# Roadmap

Ordered roughly by value. Each item lands with an acceptance test first.

## Dead code

- [ ] Unused imports and re-exports (`from x import y` never used, `__all__` entries that do not exist).
- [x] Unused methods and properties. Resolved references plus a textual attribute-name safety net for duck typing; overrides of inherited members (walked through ty, so stdlib and third-party bases count) are kept; `.setter`/`.deleter` halves are not candidates. Class attributes and instance fields are still to do.
- [x] Unused files: modules nothing imports (including by dotted string) and nothing runs. Roots: `__main__` guard, package/test/conftest/manage/wsgi/asgi files, entry-point modules, files with plugin-kept symbols.
- [ ] Unused dependencies: distributions in `pyproject.toml` that no import resolves into.
- [x] Entry points from `[project.scripts]`, `[project.gui-scripts]`, `[project.entry-points]`. Still to do: `setup.py` / `setup.cfg` entry points.
- [x] Framework plugins (convention-based, by decorator name, base class name, and file layout): FastAPI, Pydantic, Typer/Click, pytest, Flask, Airflow, Django, Celery, SQLAlchemy/SQLModel, Alembic.
- [x] Base-class identity resolved through ty: plugins match qualified ancestors such as `sqlalchemy.orm.decl_api.DeclarativeBase` and only fall back to base-name text when a base cannot be resolved.
- [ ] Decorator identity resolved through ty, so `from fastapi import APIRouter as R` and re-exported decorators are recognised.
- [x] String references: a literal `"pkg.module.attr"` or `"pkg.module:attr"` anywhere in the project counts as a use of the symbol and a deferred import of the module.
- [ ] Django: `INSTALLED_APPS`, middleware, and context-processor strings once string references land.
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
- [ ] Dupes: report clone groups (three or more occurrences) as one finding; ignore docstrings and import blocks by default.
- [x] `pyscythe health`: cyclomatic and cognitive complexity per function (`pyscythe_metrics`, parser-only), hotspots over 10/15, length-weighted 0-100 score with A-F grade.
- [x] Health thresholds in `[tool.pyscythe.health]`.
- [ ] Health: per-file and per-package scores; maintainability index; trend against a baseline.
- [x] `pyscythe boundaries`: `layers` (ranks, several prefixes per rank), `rules` with `from`/`deny`, `preset = "hexagonal"` with `root`, type-only imports allowed unless `check-type-only`.
- [ ] Boundaries: `pyscythe boundaries --suggest` to propose layers from the import graph; per-rule allow lists.
- [x] `pyscythe fix [--dry-run]`: removes dead definitions (whole lines, decorators included, gap preserved) and unused files; skips nested definitions and methods whose removal would empty a class; medium confidence and better by default.
- [ ] Fix: also drop imports that become unused after a removal; `--only RULE`; interactive confirmation.

## Output and integration

- [x] `--format sarif|github|markdown` alongside `human` and `json`.
- [x] `--format pr-comment`: Markdown with a `<!-- pyscythe:<analysis> -->` marker.
- [ ] GitHub Action.

## Deferred by decision

- [ ] Distribution: PyPI wheel via maturin, `cargo install`, Homebrew tap. (Held off on 2026-09-05.)
- [ ] Agent integration: MCP server, Claude Code skill, `actions[]` in JSON. (Not a priority as of 2026-09-05.)
- [ ] Runtime layer from coverage.py data.
