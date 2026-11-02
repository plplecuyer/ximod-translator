#!/usr/bin/env bash
# L'étiquette Git (v1.2.3) doit correspondre à la version de Cargo.toml
# (R-S02-05). Usage : check-version.sh vX.Y.Z
set -euo pipefail
tag="${1:?tag}"
tag_version="${tag#v}"
cargo_version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
if [ "$tag_version" != "$cargo_version" ]; then
  echo "L'étiquette $tag ne correspond pas à la version de Cargo.toml ($cargo_version)." >&2
  exit 1
fi
echo "Version $cargo_version"
