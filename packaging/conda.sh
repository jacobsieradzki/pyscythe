#!/usr/bin/env bash
# Render the conda-forge recipe for a released version, checksumming the tag
# tarball GitHub serves. The result goes in recipes/pyscythe/meta.yaml of a
# conda-forge/staged-recipes fork, once; after that the feedstock bumps itself.
#
#   packaging/conda.sh VERSION OUT_DIR
set -euo pipefail

version=${1:?usage: conda.sh VERSION OUT_DIR}
out=${2:?usage: conda.sh VERSION OUT_DIR}
src=$(cd "$(dirname "$0")" && pwd)

tarball="https://github.com/jacobsieradzki/pyscythe/archive/refs/tags/v$version.tar.gz"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fsSL -o "$work/source.tar.gz" "$tarball"

if command -v sha256sum >/dev/null; then
  sha256=$(sha256sum "$work/source.tar.gz" | cut -d' ' -f1)
else
  sha256=$(shasum -a 256 "$work/source.tar.gz" | cut -d' ' -f1)
fi

# conda-forge will not take a recipe whose license_file is missing from the
# source, and the LICENSE only arrived after 0.3.0.
if ! tar -tzf "$work/source.tar.gz" | grep -q "^pyscythe-$version/LICENSE$"; then
  echo "the v$version tarball has no LICENSE; conda-forge needs one" >&2
  exit 1
fi

mkdir -p "$out"
sed -e "s/__VERSION__/$version/g" -e "s/__SHA256__/$sha256/g" "$src/conda/meta.yaml" >"$out/meta.yaml"
for placeholder in __VERSION__ __SHA256__; do
  if grep -q "$placeholder" "$out/meta.yaml"; then
    echo "meta.yaml still has $placeholder after substitution" >&2
    exit 1
  fi
done
grep -q -F "$sha256" "$out/meta.yaml"

echo "$out/meta.yaml"
