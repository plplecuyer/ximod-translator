#!/usr/bin/env bash
# Prépare le dossier livré : l'interface et la ligne de commande ximod-cli
# (D-06 / D-91) — sous Linux et macOS, ximod-cli est un lien vers
# ximod-translator, qui choisit son mode d'après son nom (D-104) —, les
# assets (locales, data, fonts, dictionaries, icons, images) — jamais de
# base vanilla (D-06) — et les documents.
# Polices (D-104, R-D104-01) : assets/fonts ne contient que les polices
# citées par les données (un test le garde) et leurs licences OFL ; les
# polices de jeu que l'utilisateur dépose dans fonts/books ne sont jamais
# livrées (droits d'auteur), seul le README du dossier l'est.
# Usage : stage.sh <dossier release> <destination> [.exe]
# XIMOD_STAGE_SKIP : dossiers d'assets à ne pas copier (tests).
set -euo pipefail
release="${1:?release dir}"
dest="${2:?dest}"
ext="${3:-}"
skip=" ${XIMOD_STAGE_SKIP:-} "
rm -rf "$dest"
mkdir -p "$dest/assets"
cp "$release/ximod-translator$ext" "$dest/"
if [ -n "$ext" ]; then
  # Windows : une application à fenêtre ne s'attache pas à la console qui
  # l'a lancée ; les deux exécutables restent.
  cp "$release/ximod-cli$ext" "$dest/"
else
  ln -s ximod-translator "$dest/ximod-cli"
fi
for d in locales data dictionaries icons images; do
  case "$skip" in *" $d "*) continue ;; esac
  if [ -d "assets/$d" ]; then
    cp -R "assets/$d" "$dest/assets/"
  fi
done
if [ -d assets/fonts ]; then
  tar -C assets -cf - \
    --exclude='fonts/books/*.ttf' --exclude='fonts/books/*.TTF' \
    --exclude='fonts/books/*.otf' --exclude='fonts/books/*.OTF' \
    --exclude='fonts/.gitkeep' fonts | tar -C "$dest/assets" -xf -
fi
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
