# Roadmap

Ordered roughly by value. Each item lands with an acceptance test first.

## Dead code

- [ ] Unused imports and re-exports (`from x import y` never used, `__all__` entries that do not exist).
- [ ] Unused methods, properties, and class attributes (needs attribute-level references; ty resolves these).
- [ ] Unused files: modules no other module imports and that are not entry points.
- [ ] Unused dependencies: distributions in `pyproject.toml` that no import resolves into.
- [ ] Entry points: `[project.scripts]`, `[project.entry-points]`, `__main__.py`, `if __name__ == "__main__"`.
- [ ] Framework plugins that mark roots and framework-consumed symbols: FastAPI, Pydantic, Typer/Click, pytest (fixtures via ty), Flask, Airflow, setuptools entry points, Django (migrations, admin, management commands, settings strings, signals), Celery, SQLAlchemy, Alembic.
- [ ] `[tool.pyscythe]` config in `pyproject.toml`: ignore globs, ignore names, extra entry points.
- [ ] Inline suppression comment (`# pyscythe: ignore[unused-function]`).
- [ ] Baseline file and `--since <ref>` for PR gating.
- [ ] Performance: replace per-symbol `find_references` with a single pass that resolves every reference expression to its definition and builds an inverted index.

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
