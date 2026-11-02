#!/usr/bin/env bash
# Reconstruire une traduction publiée depuis son plugin et son dictionnaire
# du mod, sans interface (XIMOD Translator, ximod-cli, D-06 / T-D06-01) :
# apply (la pile, pour le compte), check, export DSD et override, manifest,
# package. Rien n'est écrit si le plugin manque (code 2).
#
# Usage : rebuild.sh <plugin> <dossier de sortie> [nom] [version] [auteur]
# Variables : XIMOD_CLI (le chemin de ximod-cli, défaut : ximod-cli),
#             XIMOD_ARGS (options communes, par ex. « --game SkyrimSE --to fra »).
#             XIMOD_STRICT=1 : les réserves de check (erreurs trouvées)
#             comptent dans le code de sortie.
# Code de sortie : 0 les sorties sont reconstruites, 1 avec réserves
# (export, ou check avec XIMOD_STRICT=1), 2 non fait — arrêt immédiat.
set -u
cli="${XIMOD_CLI:-ximod-cli}"
read -r -a common <<< "${XIMOD_ARGS:-}"
plugin="${1:?plugin}"
out="${2:?dossier de sortie}"
name="${3:-$(basename "${plugin%.*}")}"
version="${4:-1.0}"
author="${5:-}"
code=0
step() {
  "$cli" ${common[@]+"${common[@]}"} "$@"
  local rc=$?
  if [ "$rc" -ge 2 ]; then exit "$rc"; fi
  if [ "$rc" -gt "$code" ]; then code=$rc; fi
}
# 1. Ouvrir et appliquer la pile : vérifie le plugin et son dictionnaire.
step apply "$plugin"
# 2. Vérifier : le rapport à côté des sorties, pas dedans.
mkdir -p "$out"
"$cli" ${common[@]+"${common[@]}"} check "$plugin" --report "$out.check.csv"
rc=$?
if [ "$rc" -ge 2 ]; then exit "$rc"; fi
if [ "$rc" -eq 1 ] && [ "${XIMOD_STRICT:-0}" = "1" ]; then code=1; fi
# 3. Les sorties, depuis le dictionnaire du mod.
step export "$plugin" --output dsd --statuses validated,pending -o "$out"
step export "$plugin" --output override --statuses validated,pending -o "$out"
# 4. Le manifeste (le plugin source et son CRC-32) puis le paquet.
step manifest "$out" --input "$plugin"
step package "$out" --name "$name" --version "$version" --author "$author" -o "$out.zip"
exit "$code"
