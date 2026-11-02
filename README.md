# XIMOD Translator

Outil de traduction de mods pour les jeux Bethesda — Morrowind, Oblivion (et
Oblivion Remastered), Skyrim (LE et SE/AE), Fallout 3, Fallout: New Vegas,
Fallout 4, Fallout 76 et Starfield — sous Windows, Linux et macOS. Compagnon de
**XIMOD Architect**, écrit en **Rust** avec l'interface **egui**, traduit en
33 langues d'interface.

Version courante : **0.3.1** (29 octobre 2026). Le détail des versions est dans
[`CHANGELOG.md`](CHANGELOG.md) ; la documentation en ligne est le
[wiki](../../wiki), en anglais ; le manuel complet, en français et en anglais,
est livré dans `Manuals/` avec chaque paquet (**F1** dans l'application).

## Ce qu'il fait

- **Ouvre** les plugins (`.esp`, `.esm`, `.esl`), les fichiers STRINGS, les
  plugins localisés, les fichiers d'interface (MCM, INI, JSON, XML, formats
  texte) et les dictionnaires des autres outils (xTranslator `.sst` / `.xml`,
  ESP-ESM Translator `.eet`, Dynamic String Distributor).
- **Montre** chaque texte traduisible dans une grille avec un statut (À
  traduire, Validé, En attente, Traduit par similarité, Proposition modifiée,
  Personnalisé, Ignoré), des filtres, des compteurs, une colonne de relecture,
  un historique et une annulation sans limite.
- **Pré-remplit** la traduction depuis une pile de dictionnaires : la
  traduction du jeu lui-même (base vanilla construite depuis vos fichiers), les
  dictionnaires d'autres mods, votre dictionnaire personnel et le dictionnaire
  de travail du mod. Les assistants hors ligne (glossaire, motifs, dérivations)
  et les services en ligne (DeepL, Azure, Google, tout serveur compatible
  OpenAI, y compris un modèle local) complètent.
- **Exporte** un module complet, un correctif (override), des STRINGS, un
  fichier DSD pour SKSE ou un fichier texte traduit ; vérifie le résultat
  (balises, nombres, longueurs, encodages) ; écrit un manifeste ; empaquette le
  tout avec un installateur FOMOD pour Mod Organizer 2 et Vortex.
- **Fait tout cela sans fenêtre** aussi : `ximod-cli` enchaîne chaque étape, par
  lots, pour l'intégration continue.

## Télécharger

Chaque [version publiée](../../releases) fournit : un installateur et une
archive ZIP pour Windows x64, une archive tar.gz pour Linux x64, un `.dmg` universel
pour macOS (Apple Silicon et Intel dans le même bundle ; non signé : clic
droit ▸ Ouvrir au premier lancement). Chaque paquet contient les deux exécutables
(`ximod-translator` et `ximod-cli`), les ressources, les manuels et les
exemples de la ligne de commande (`docs/cli/`).

**Aucune base vanilla n'est livrée** : elle se construit en quelques secondes
depuis vos propres fichiers de jeu (*Dictionnaires ▸ Créer un dictionnaire à
partir d'un jeu…*) et s'applique ensuite d'elle-même à chaque mod de ce jeu.

Un `Config.ini` à côté de l'exécutable rend l'application **portable** ; sinon
elle écrit dans `%APPDATA%\XIMOD Translator`, `~/.config/ximod-translator` ou
`~/Library/Application Support/XIMOD Translator`.

## Premier mod en six étapes

1. Choisir la langue **Cible** dans la barre d'outils (par pays) ; le **Jeu** et
   la **Source** sont lus dans le fichier à l'ouverture.
2. Ouvrir le plugin du mod (*Fichier ▸ Ouvrir…*, récents ou glisser-déposer) :
   les lignes connues de la base du jeu, du dictionnaire du mod et du
   dictionnaire personnel sont pré-remplies.
3. Taper les traductions dans la colonne **Traduction** ; une ligne tapée
   devient *Validé*, `Ctrl+Z` annule, `Ctrl+↓` valide et passe à la suivante.
   Relire les lignes pré-remplies et les valider (`F5`).
4. Vérifier l'orthographe (*Options ▸ Correcteur orthographique…*) et la qualité
   (`F12`).
5. Choisir la **Sortie** que le jeu accepte, puis *Fichier ▸ Exporter la
   traduction…* : seules les lignes validées partent par défaut.
6. *Outils ▸ Empaqueter une traduction…* pour une archive avec installateur
   FOMOD.

Le dictionnaire du mod (`<mod>_<SRC>-<DST>.ximdb`) est enregistré
automatiquement : rouvrir le plugin, ou une version plus récente, restaure le
travail avec ses statuts, sa provenance et son historique.

## Compiler

Rust **1.97 ou plus** (édition 2024). Sous Linux (Debian / Ubuntu) :
`libgtk-3-dev libxkbcommon-dev libwayland-dev libasound2-dev libssl-dev pkg-config`.

```bash
cargo build --release                 # target/release/ximod-translator et ximod-cli
cargo run                             # l'interface
cargo run -- MonMod.esp               # ouvre un ou plusieurs fichiers au démarrage
cargo run --bin ximod-cli -- --help   # la ligne de commande
cargo test --workspace                # tests unitaires et d'intégration sur fichiers réels
cargo clippy --all-targets --workspace -- -D warnings
```

La fonctionnalité `gog-galaxy` (`--features gog-galaxy`) compile le lecteur
SQLite des bases de GOG Galaxy et de Lutris ; les binaires livrés l'incluent.
Les exécutables embarquent les modèles de détection de langue (environ 300 Mo
chacun) : une version optimisée se compile en plusieurs minutes.

`.github/workflows/ci.yml` exécute build, `clippy -D warnings` et tests sur
Ubuntu, Windows et macOS à chaque push ; `release.yml` se déclenche sur une
étiquette `vX.Y.Z` égale à la version de `Cargo.toml`, construit les trois
paquets (Windows, Linux, macOS universel), assemble les paquets (`packaging/stage.sh`) et publie la page de
version dont le corps est la section correspondante de `CHANGELOG.md`.

## Organisation du dépôt

```
src/            la bibliothèque (tm, plugin, games, pile de dictionnaires, export, online, i18n, cli…) et ui/
src/bin/cli/    ximod-cli
ximod-games/ ximod-fomod/ esm-explorer/ bsa-explorer/ swf-explorer/   crates de l'espace de travail
assets/         locales (33), data, fonts, dictionaries (Hunspell), icons, images ; bases/ reste vide ici
tests/          tests d'intégration et fixtures
examples/       petits outils (dump d'un plugin, aperçus, bancs d'essai)
tools/          aides i18n, générateurs de fixtures, simulateur de service en ligne pour les tests
docs/cli/       scripts et exemples de la ligne de commande
packaging/      stage.sh, installateur Windows (Inno Setup), bundle macOS, fichier .desktop
Manuals/        les manuels livrés dans chaque paquet
```

Conventions : aucun texte visible codé en dur (tout passe par les fichiers
Fluent `assets/locales/<code>/main.ftl`, traduits dans les 33 langues à chaque
livraison, test `tests/no_hardcoded_text.rs`) ; aucune clé d'API dans
`Config.ini`, les profils, les journaux ou les sorties (trousseau du système) ;
toute opération sur des lignes est une étape d'annulation et laisse une
provenance ; les fichiers de données livrés sont en lecture seule, une copie
utilisateur l'emporte. Voir la page *Building from Source* du wiki.

## Licence et crédits

Distribué sous la licence du fichier [`LICENSE`](LICENSE). Le moteur de mémoire
de traduction descend de `xmod_tm` ; la connaissance des formats de xTranslator
et d'ESP-ESM Translator vient de la lecture de leurs fichiers, pas de leur
code. Polices Noto (SIL Open Font License) ; drapeaux partagés avec
XIMOD Architect.
