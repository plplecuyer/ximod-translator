# ximod-cli — scripts et exemples

`ximod-cli` est l'exécutable console de XIMOD Translator (chapitre « La ligne
de commande » du manuel). Il lit le même `Config.ini` que l'interface (ou celui
du dossier donné à `--config`) et ne l'écrit jamais.

| Fichier | Contenu |
|---|---|
| `rebuild.sh` | Reconstruire une traduction publiée : apply, check, export DSD et override, manifest, package (bash, Linux / macOS / Git Bash). |
| `rebuild.ps1` | Le même script en PowerShell (Windows). |
| `batch-example.json` | Un fichier de lot : deux travaux, un motif `files`, des étapes avec options. |
| `github-actions.yml` | Une intégration continue : reconstruire les sorties à chaque mise à jour du dictionnaire du mod, échouer si `check` trouve des erreurs. |

Codes de retour : 0 fait, 1 fait avec réserves (erreurs de vérification,
lignes non traduites avec `--strict`, translate arrêté par `--max-chars`),
2 non fait (fichier introuvable, option inconnue, format invalide, service
injoignable, écriture refusée).

Exemples :

```
ximod-cli info "Apocalypse - Magic of Skyrim.esp"
ximod-cli --to fra apply "Apocalypse - Magic of Skyrim.esp" --write-dict
ximod-cli check "Apocalypse - Magic of Skyrim.esp" --report check.csv
ximod-cli export "Apocalypse - Magic of Skyrim.esp" --output dsd --statuses validated,pending -o out
ximod-cli manifest out --input "Apocalypse - Magic of Skyrim.esp"
ximod-cli package out --name "Apocalypse" --version 9.45 --author Patrice -o Apocalypse-FR.zip
XIMOD_KEY_DEEPL=… ximod-cli translate Mod.esp --service deepl --max-chars 50000 --write-dict
ximod-cli --json batch batch.json
```
