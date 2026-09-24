#!/usr/bin/env bash
# Render the contents of the jacobsieradzki/pyscythe-pre-commit mirror for a
# released version. pre-commit installs the hook repository itself, and this
# repository's own pyproject.toml is maturin's, so the hooks live in a mirror
# whose package does nothing but depend on the matching pyscythe wheel.
#
#   packaging/pre-commit.sh VERSION OUT_DIR
set -euo pipefail

version=${1:?usage: pre-commit.sh VERSION OUT_DIR}
out=${2:?usage: pre-commit.sh VERSION OUT_DIR}
src=$(cd "$(dirname "$0")" && pwd)

mkdir -p "$out"
cp "$src/pre-commit/.pre-commit-hooks.yaml" "$out/.pre-commit-hooks.yaml"
cp "$src/../LICENSE" "$out/LICENSE"
mkdir -p "$out/.github/workflows"
cp "$src/pre-commit/.github/workflows/tests.yml" "$out/.github/workflows/tests.yml"

# rustfmt is not the only thing that quietly defeats a textual patch; assert
# that every placeholder went, and that the version arrived.
for file in pyproject.toml README.md; do
  sed "s/__VERSION__/$version/g" "$src/pre-commit/$file" >"$out/$file"
  if grep -q __VERSION__ "$out/$file"; then
    echo "$file still has a placeholder after substitution" >&2
    exit 1
  fi
  if ! grep -q -F "$version" "$out/$file"; then
    echo "$file does not mention $version after substitution" >&2
    exit 1
  fi
done

ls -1 "$out"
