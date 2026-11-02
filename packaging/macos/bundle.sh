#!/usr/bin/env bash
# Construit « XIMOD Translator.app » depuis le dossier livré (non signé : voir
# le README pour l'ouverture par clic droit ▸ Ouvrir). Les assets vont dans
# Contents/Resources/assets, chemin que l'application connaît.
# Usage : bundle.sh <stage> <dossier de sortie> <version>
set -euo pipefail
stage="${1:?stage}"
out="${2:?out}"
version="${3:?version}"
app="$out/XIMOD Translator.app"
rm -rf "$out"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$stage/ximod-translator" "$app/Contents/MacOS/"
chmod +x "$app/Contents/MacOS/ximod-translator"
# La ligne de commande (D-06) : à côté, appelée par son chemin
# (« XIMOD Translator.app/Contents/MacOS/ximod-cli »).
cp "$stage/ximod-cli" "$app/Contents/MacOS/"
chmod +x "$app/Contents/MacOS/ximod-cli"
cp -R "$stage/assets" "$app/Contents/Resources/assets"
if [ -f "$stage/assets/icons/ximod-translator.icns" ]; then
  cp "$stage/assets/icons/ximod-translator.icns" "$app/Contents/Resources/ximod-translator.icns"
fi
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
for f in README.md LICENSE CHANGELOG.md; do
  [ -f "$stage/$f" ] && cp "$stage/$f" "$out/"
done
if [ -d "$stage/docs" ]; then cp -R "$stage/docs" "$out/" ; fi
ln -s /Applications "$out/Applications"
echo "Bundle: $app"
