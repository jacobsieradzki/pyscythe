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
- [ ] Inline suppression comment (`# pyscythe: ignore[unused-function]`).
- [ ] Baseline file and `--since <ref>` for PR gating.
- [x] Performance: single-pass inverted reference index built in parallel (also fixed aliased-import misses).
- [x] Perf: the process forgets the salsa database at exit instead of dropping it (ty does the same), and the project is opened once. 64 files in ~180 ms wall.

## Other analyses

- [x] Circular imports: `pyscythe cycles`, Tarjan SCCs over runtime imports; `TYPE_CHECKING` and function-local imports are excluded.
- [ ] Cycles: report every distinct simple cycle in a component, not just one; offer `--include-deferred`.
- [ ] Unused files: treat `__init__.py` re-exports as uses of the re-exported file.
- [ ] Duplication (token-hash / suffix-array detector over function bodies).
- [ ] Complexity hotspots and a 0-100 health score.
- [ ] Architecture boundaries with layered / hexagonal presets.
- [ ] `fix --dry-run` for safe deletions.

## Output and integration

- [ ] SARIF, GitHub annotations, markdown, and PR-comment formats.
- [ ] GitHub Action.

## Deferred by decision

- [ ] Distribution: PyPI wheel via maturin, `cargo install`, Homebrew tap. (Held off on 2026-09-05.)
- [ ] Agent integration: MCP server, Claude Code skill, `actions[]` in JSON. (Not a priority as of 2026-09-05.)
- [ ] Runtime layer from coverage.py data.
