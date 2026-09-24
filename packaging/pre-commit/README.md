# pyscythe-pre-commit

[pre-commit](https://pre-commit.com) hooks for
[pyscythe](https://github.com/jacobsieradzki/pyscythe): dead code, import
cycles, duplication, complexity, architecture boundaries, and dependency drift
in Python projects, in one fast binary.

```yaml
repos:
  - repo: https://github.com/jacobsieradzki/pyscythe-pre-commit
    rev: v__VERSION__
    hooks:
      - id: pyscythe-dead-code
```

| Hook | Runs |
| --- | --- |
| `pyscythe-dead-code` | `pyscythe dead-code` |
| `pyscythe-cycles` | `pyscythe cycles` |
| `pyscythe-deps` | `pyscythe deps` |
| `pyscythe-boundaries` | `pyscythe boundaries` |
| `pyscythe-health` | `pyscythe health` |
| `pyscythe-dupes` | `pyscythe dupes` |

Every analysis reads the whole project rather than the files a commit touches,
so the hooks pass no filenames: one run per commit, over everything. On a large
codebase that belongs at push time instead.

```yaml
      - id: pyscythe-dead-code
        stages: [pre-push]
```

Arguments go straight through, so a codebase adopting pyscythe can record what
is there today and fail only on what is new.

```yaml
      - id: pyscythe-dead-code
        args: [--min-confidence, medium, --baseline, .pyscythe-baseline.json]
```

This repository holds nothing but the hook definitions. pre-commit installs the
hook repository itself into a virtualenv, and pyscythe ships as a binary wheel,
so the package here exists only to pull that wheel from PyPI; its tags track
pyscythe's releases. It is generated from `packaging/pre-commit` in the main
repository, which is where issues and pull requests belong.
