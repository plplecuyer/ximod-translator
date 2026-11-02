#!/usr/bin/env bash
# Prépare le dossier livré : les deux exécutables (l'interface et la ligne
# de commande ximod-cli, D-06 / D-91), les assets (locales, data, fonts,
# dictionaries, icons, images) — jamais de base vanilla (D-06) — et les
# documents. Usage : stage.sh <dossier release> <destination> [.exe]
set -euo pipefail
release="${1:?release dir}"
dest="${2:?dest}"
ext="${3:-}"
rm -rf "$dest"
mkdir -p "$dest/assets"
cp "$release/ximod-translator$ext" "$dest/"
cp "$release/ximod-cli$ext" "$dest/"
for d in locales data fonts dictionaries icons images; do
  if [ -d "assets/$d" ]; then
    cp -R "assets/$d" "$dest/assets/"
  fi
done
# Les dossiers de bases existent, vides : l'application y écrit les siens.
mkdir -p "$dest/assets/bases/jeux" "$dest/assets/bases/mods"
cp README.md "$dest/" 2>/dev/null || true
cp LICENSE "$dest/" 2>/dev/null || true
cp CHANGELOG.md "$dest/" 2>/dev/null || true
if [ -d Manuals ]; then cp -R Manuals "$dest/" ; fi
# Les scripts et exemples de la ligne de commande (R-D06-07).
if [ -d docs/cli ]; then mkdir -p "$dest/docs" && cp -R docs/cli "$dest/docs/" ; fi
echo "Staged in $dest:"
ls -la "$dest"
