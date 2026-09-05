# Roadmap

Ordered roughly by value. Each item lands with an acceptance test first.

## Dead code

- [ ] Unused imports and re-exports (`from x import y` never used, `__all__` entries that do not exist).
- [ ] Unused methods, properties, and class attributes (needs attribute-level references; ty resolves these).
- [ ] Unused files: modules no other module imports and that are not entry points.
- [ ] Unused dependencies: distributions in `pyproject.toml` that no import resolves into.
- [x] Entry points from `[project.scripts]`, `[project.gui-scripts]`, `[project.entry-points]`. Still to do: `setup.py` / `setup.cfg` entry points.
- [x] Framework plugins (convention-based, by decorator name, base class name, and file layout): FastAPI, Pydantic, Typer/Click, pytest, Flask, Airflow, Django, Celery, SQLAlchemy/SQLModel, Alembic.
- [ ] Resolve decorator and base-class identity through ty instead of matching text, so `from fastapi import APIRouter as R` and re-exported bases are recognised.
- [ ] String references: a literal `"pkg.module.attr"` or `"pkg.module:attr"` anywhere in the project counts as a use (Django settings, Celery `include`, `importlib`, Airflow).
- [ ] Django: `INSTALLED_APPS`, middleware, and context-processor strings once string references land.
- [ ] `--show-kept` to list what plugins suppressed and why.
- [ ] Config toggle to include notebooks (excluded by default).
- [ ] `[tool.pyscythe]` config in `pyproject.toml`: ignore globs, ignore names, extra entry points.
- [ ] Inline suppression comment (`# pyscythe: ignore[unused-function]`).
- [ ] Baseline file and `--since <ref>` for PR gating.
- [x] Performance: single-pass inverted reference index built in parallel (also fixed aliased-import misses).
- [ ] Project discovery shells out to `uv` for workspace metadata, which costs roughly half a second on a small project. Consider a `--no-uv` flag or caching.

## Other analyses

- [ ] Circular imports (module graph SCCs, with the import lines that close each cycle).
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
