# Packaging

Everything pyscythe is published through, and the scripts that render it. PyPI
is the source of truth: every other channel serves the same binaries, built
once by `release.yml`.

| Channel | Where it lives | Automated |
| --- | --- | --- |
| PyPI wheels and sdist | `release.yml`, trusted publishing | yes, on a `v*` tag |
| GitHub Release archives | `binaries.sh`, repacked from the wheels | yes, on a `v*` tag |
| Homebrew | [`jacobsieradzki/homebrew-tap`](https://github.com/jacobsieradzki/homebrew-tap) | yes, if `DOWNSTREAM_TOKEN` is set |
| pre-commit | [`jacobsieradzki/pyscythe-pre-commit`](https://github.com/jacobsieradzki/pyscythe-pre-commit) | yes, if `DOWNSTREAM_TOKEN` is set |
| GitHub Marketplace | `action.yml` in this repository | no, a checkbox per release |
| conda-forge | `conda/meta.yaml`, submitted once to staged-recipes | no, then the feedstock bumps itself |
| crates.io | blocked | no |

## The scripts

Each takes a version and writes somewhere, so the release workflow and a
person at a desk run the same code.

```bash
packaging/binaries.sh WHEEL_DIR OUT_DIR       # wheels -> per-platform archives + SHA256SUMS
packaging/homebrew.sh VERSION ARCHIVE_DIR     # -> Formula/pyscythe.rb on stdout
packaging/pre-commit.sh VERSION OUT_DIR       # -> the mirror repository's contents
packaging/conda.sh VERSION OUT_DIR            # -> meta.yaml, checksumming the tag tarball
```

`binaries.sh` repacks the maturin wheels rather than building anything: a wheel
is a zip with the binary in `pyscythe-VERSION.data/scripts`, and Homebrew wants
a tarball with a binary in it.

## One-time steps a person has to do

- **`DOWNSTREAM_TOKEN`.** A fine-grained personal access token with contents
  write on `homebrew-tap` and `pyscythe-pre-commit`, saved as a repository
  secret here. Without it the release still succeeds and the downstream job
  logs a warning instead of pushing.
- **Marketplace.** On a GitHub Release, tick "Publish this Action to the
  GitHub Marketplace". It needs `action.yml` at the repository root with a name
  unique across GitHub, a description, and branding, all of which are there.
- **conda-forge.** Run `packaging/conda.sh`, open a pull request adding
  `recipes/pyscythe/meta.yaml` to `conda-forge/staged-recipes`, and answer the
  review. The recipe builds from the tag tarball with cargo, so it needs
  network during the build for the ruff git dependency; that is the part a
  reviewer is most likely to push back on.

## Why pre-commit needs a second repository

pre-commit installs the hook repository itself into a virtualenv with pip. For
a tool written in Rust the trick is a package that only depends on the wheel
already on PyPI, which is what `typos` does with a `setup.py` at its root. This
repository's root `pyproject.toml` is already taken: it is maturin's, and
installing it means a full Rust build. So the hooks live in a mirror, the way
ruff, black, and mypy all do it, and `pre-commit.sh` renders it.

## Why crates.io is still blocked

`cargo publish` refuses a git dependency, and `pyscythe_ty` needs `ty_project`
and `ty_ide`. Astral publishes the rest of the workspace to crates.io at
0.0.14 — `ruff_db`, `ruff_python_ast`, `ty_python_semantic`,
`ty_module_resolver` and the others are all there — but not those two, checked
2026-09-24. The crates here already carry the description, license, and
repository that publishing requires, so the day they appear it is
`cargo publish` for `pyscythe_core`, `pyscythe_metrics`, `pyscythe_pyproject`,
`pyscythe_ty`, then `pyscythe`, in that order.
