#!/usr/bin/env bash
# Repack the maturin wheels into plain archives: the bare binary, the license,
# and the readme, one archive per platform, with the SHA256SUMS the Homebrew
# formula reads. Homebrew and a direct download want a binary, not a wheel, and
# the wheels are already built, so this never compiles anything.
#
#   packaging/binaries.sh WHEEL_DIR OUT_DIR
set -euo pipefail

wheels=${1:?usage: binaries.sh WHEEL_DIR OUT_DIR}
out=${2:?usage: binaries.sh WHEEL_DIR OUT_DIR}
root=$(cd "$(dirname "$0")/.." && pwd)

# macOS tar otherwise writes an AppleDouble file beside anything with an
# extended attribute, and a downloaded binary has one.
export COPYFILE_DISABLE=1

target_for() {
  case $1 in
  macosx_*_arm64) echo aarch64-apple-darwin ;;
  macosx_*_x86_64) echo x86_64-apple-darwin ;;
  manylinux_*_x86_64) echo x86_64-unknown-linux-gnu ;;
  manylinux_*_aarch64) echo aarch64-unknown-linux-gnu ;;
  win_amd64) echo x86_64-pc-windows-msvc ;;
  *)
    echo "unknown platform tag: $1" >&2
    return 1
    ;;
  esac
}

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}

mkdir -p "$out"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

found=0
for wheel in "$wheels"/*.whl; do
  [ -e "$wheel" ] || break
  name=$(basename "$wheel" .whl) # pyscythe-VERSION-py3-none-PLATFORM
  version=${name#pyscythe-}
  version=${version%%-*}
  platform=${name##*-}
  target=$(target_for "$platform") || exit 1

  stage="$work/pyscythe-$version-$target"
  mkdir -p "$stage"
  unzip -q -o "$wheel" -d "$work/wheel"
  cp "$work/wheel/pyscythe-$version.data/scripts"/pyscythe* "$stage/"
  chmod +x "$stage"/pyscythe*
  cp "$root/LICENSE" "$root/README.md" "$stage/"
  rm -rf "$work/wheel"

  if [ "$platform" = win_amd64 ]; then
    (cd "$work" && zip -qr "$out/pyscythe-$version-$target.zip" "pyscythe-$version-$target")
  else
    tar -czf "$out/pyscythe-$version-$target.tar.gz" -C "$work" "pyscythe-$version-$target"
  fi
  found=$((found + 1))
done

if [ "$found" -eq 0 ]; then
  echo "no wheels in $wheels" >&2
  exit 1
fi

(
  cd "$out"
  shopt -s nullglob
  archives=(*.tar.gz *.zip)
  sha256 "${archives[@]}" >SHA256SUMS
)
cat "$out/SHA256SUMS"
