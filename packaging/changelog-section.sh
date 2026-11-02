#!/usr/bin/env bash
# Extrait la section « ## [X.Y.Z] » de CHANGELOG.md (corps de la page de version).
set -euo pipefail
version="${1:?version}"
awk -v v="$version" '
  /^## \[/ { if (found) exit; if (index($0, "[" v "]") > 0) { found = 1; next } }
  found { print }
' CHANGELOG.md
