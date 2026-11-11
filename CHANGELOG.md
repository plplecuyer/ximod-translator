# Journal des versions

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/) ; les
versions suivent [SemVer](https://semver.org/lang/fr/). La section de la
version étiquetée devient le corps de la page de version (S-02).

## [Unreleased]

### Ajouté (étape 3, lot 1)
- Journal de la session (E-01, décision D-108) : *Aide › Journal…*
  (Ctrl+Maj+J) et un bouton *Journal* dans la barre d'état, qui n'apparaît
  qu'après la première opération et compte les avertissements et erreurs
  non lus (« Journal (2 ⚠) »). Le journal garde, dans la langue de
  l'interface, le démarrage, chaque ouverture (jeu, langues, lignes,
  encodage, durée), la pile de dictionnaires appliquée — une entrée par
  dictionnaire avec ses lignes, puis le total, les restantes et les
  verrouillées —, le changement de cible ou de sortie, les vérifications,
  chaque fichier écrit par un export, et tout message, avertissement, erreur
  ou alerte (10 000 entrées au plus, en mémoire). Filtres par niveau, par
  document et par texte (sans casse ni accents) ; *Voir les lignes* pose une
  puce « Journal : … » dans la barre de filtres (anomalies d'une
  vérification, lignes qu'un export refuse pour l'encodage) ; *Copier la
  sélection*, *Tout copier* et *Enregistrer…* en texte tabulé ; *Effacer*.
  Chaque entrée va aussi dans `logs/ximod.log` : le test du chemin de base
  du 7 novembre 2026 avait montré que ce fichier s'arrêtait après le
  démarrage. Fenêtre : `[Window] JournalX/Y/W/H`. Manuel § 4.1, 4.5, 4.7,
  17, 18.1.
- `ximod-cli` : `--verbose` écrit désormais le journal de la session sur la
  sortie d'erreur (une entrée par ligne, « heure ⇥ niveau ⇥ document ⇥
  message », dans la langue de `--lang`), et l'ajoute à l'objet de réponse
  avec `--json` (clé `journal`) ; la nouvelle option `--log <fichier>`
  l'ajoute à un fichier, avec ou sans `--verbose`. La trace technique ne
  suit plus `--verbose` mais la seule variable `RUST_LOG`. Aucune clé
  d'API ni texte envoyé en ligne n'entre dans le journal (test).

### Modifié
- Boutons en relief (F-07, demande du 11 novembre 2026) : egui ne dessinant
  pas de relief, un bouton a désormais un fond « levé », entre celui du
  panneau et celui des champs, et un contour sombre ; les listes
  déroulantes prennent le fond creusé des champs de saisie. Un bouton se
  distingue ainsi d'un champ et d'une liste, dans les deux thèmes.

### Corrigé
- Windows : *Traduire la ligne en ligne* (Ctrl+G) sans service configuré
  ouvrait Google Traduction sans le texte ni la langue cible. L'adresse
  passait par `cmd /C start`, qui lit chaque « & » comme une fin de
  commande ; elle est désormais ouverte par l'appel du système
  (ShellExecute, bibliothèque `webbrowser`), entière, sur les trois
  systèmes. Même correction pour la fenêtre de traduction des FOMOD.
- Intégration continue et livraison : GitHub a retiré les runners
  `macos-14` le 2 novembre 2026 (« The configuration
  'macos-14-arm64-us-default' is not supported ») ; `ci.yml` et
  `release.yml` passent à `macos-15` (Apple Silicon, paquet universel
  toujours réuni par `lipo`).
- Intégration continue : Clippy 1.99 (règle `chunks_exact_to_as_chunks`)
  refusait dix appels `chunks_exact(2)` / `chunks_exact(4)` (décodage
  UTF-16 des formats texte, EET, SST, lanceurs, FOMOD ; mots-clés de
  l'explorateur ESM) ; ils utilisent `as_chunks` (Rust 1.88+, compatible
  avec la version minimale 1.97). `actions/checkout` passe en v5 dans
  les deux workflows (dépréciation de Node.js 20).
- macOS : l'écran d'accueil natif (Cocoa) ne compilait pas — la bibliothèque
  `cocoa` est dépréciée et ses symboles ont disparu des versions courantes.
  Comme XIMOD Architect, macOS n'affiche plus l'image d'accueil : la
  préparation du démarrage (détection des jeux, dictionnaires) est attendue
  sans fenêtre, puis la fenêtre principale s'ouvre. Les dépendances `cocoa`
  et `objc` sont retirées (manuel § 2.2).
- Windows : Clippy 1.99 (`collapsible_if`, `too_many_arguments`,
  `unnecessary_mut_passed`) sur le code propre à Windows — décodage xWMA de
  l'explorateur d'archives, lecture du registre (Steam, GOG), écran
  d'accueil Win32, mesure de l'écran — et sur la lecture des bases GOG
  Galaxy / Lutris (fonctionnalité `gog-galaxy`) ; conditions réécrites en
  chaînes de `let`, références non mutables.
- Correcteur orthographique : avec le jeu complet des dictionnaires Hunspell
  (plusieurs régions par langue : en_AU, en_CA, en_GB, en_US…), la langue
  cible prenait le premier dictionnaire lu dans le dossier, donc l'anglais
  d'Australie pour « eng ». Le choix suit désormais une région préférée par
  langue (en → US, pt → BR, sv → SE…), puis la région qui répète la langue
  (fr_FR, de_DE), puis un code nu, dans un ordre qui ne dépend plus du
  système de fichiers (test `spell_dict` de la CI Linux).
- Tests de la ligne de commande (`tests/cli.rs`) : ils tournaient en mode
  utilisateur et indexaient donc, comme bases livrées, les bases vanilla du
  dépôt de travail (`assets/bases/jeux`) quand il y en a — trois tests
  échouaient sur un poste de développement avec une base Skyrim. Chaque
  test a désormais son dossier portable (Config.ini + assets/bases vides).
- Livraison macOS : le runner Intel `macos-13` n'existe plus sur GitHub (le
  job attendait un exécutant sans fin). Un seul paquet macOS désormais,
  universel : les deux architectures sont compilées sur le runner Apple
  Silicon et réunies par `lipo` dans un même bundle,
  `XIMOD-Translator-<version>-macos-universal.dmg` (manuel § 2.1), doublé
  du bundle seul en archive, `…-macos-universal.app.zip`, comme pour
  XIMOD Architect.
- Livraison Windows : Inno Setup refusait l'icône de l'application comme
  icône de l'installateur (« Resource update error: File is too large » :
  le fichier .ico partagé avec Architect contient des images 512 et
  1024 px). L'installateur a sa propre icône, `packaging/windows/setup.ico`,
  limitée aux tailles 16 à 256 px. `actions/upload-artifact` et
  `download-artifact` passent en v5 (dépréciation de Node.js 20).
- Dépôt : un `.gitattributes` fixe les fins de ligne — sources normalisées
  en LF, fixtures des tests, ressources et manuels pris octet pour octet.
  Sans lui, Git sous Windows convertissait les fins de ligne à la
  validation et à l'extraction : la fixture CSV à fins de ligne CRLF
  arrivait en LF sur Linux et macOS (test `structured`), et les listings
  de régression des champs arrivaient en CRLF sur Windows (test
  `fields_regression`, désormais insensible aux fins de ligne).
- macOS : Clippy 1.99 (`bind_instead_of_map`) sur la recherche de l'icône
  dans le bundle. Windows : le test du cache des noms comparait un chemin
  trouvé sans tenir compte de la casse du système de fichiers ; le test des
  lanceurs (Heroic) écrivait des chemins à barres obliques inverses dans
  des fichiers JSON ; le test des textes codés en dur comparait un chemin
  relatif avec un suffixe à barres obliques.

### Modifié
- Dépendances mises à jour (`cargo update` du 29/10/2026, compatibles
  Rust 1.97) : utf8_iter 1.0.4 → 1.1.0, qui ajoute utf_types 1.0.0 ;
  zcheapstr 1.1.0 → 1.2.0. Aucun changement de code.
- Dépendances mises à jour (`cargo update` du 02/11/2026, compatibles
  Rust 1.97) : crc32fast 1.5.3 → 1.5.4, encoding_rs 0.8.42 → 0.8.44,
  log 0.4.34 → 0.4.35, webbrowser 1.2.5 → 1.2.6, et la famille
  wasm-bindgen 0.2.129 → 0.2.130 (js-sys et web-sys 0.3.107,
  wasm-bindgen-futures 0.4.80). Aucun changement de code.

## [0.3.1] - 2026-10-29

Les deux décisions de la recette de l'étape 2, pour le chemin de base.

### Modifié
- Statut d'une traduction tapée (décision D-94, manuel § 9) : en quittant
  la cellule — grille ou fenêtre d'édition —, une ligne qui n'avait pas de
  traduction devient Validé ; une traduction automatique retouchée
  (Traduit par similarité, En attente) devient Proposition modifiée ;
  Validé et Personnalisé ne changent pas ; une traduction effacée ramène
  la ligne à À traduire. Le statut est posé dans la même opération
  annulable que la saisie, doublons de la vue Uniques compris. Les dix
  lignes tapées du chemin de base s'exportent sans réglage ; les
  remplissages automatiques restent à valider.
- Ouvrir sans jeu choisi (décision D-95) : Fichier › Ouvrir…, Récents et
  le glisser-déposer ne sont plus grisés tant que la liste Jeu est vide.
  Un plugin s'ouvre avec le jeu de son en-tête, qui passe dans la liste
  Jeu pour les fichiers suivants ; un fichier qui ne dit pas son jeu
  (texte, STRINGS seuls) s'ouvre avec le jeu de la liste, et, la liste
  vide, un message demande de le choisir (`alert-game-unknown`, 33
  langues).

## [0.3.0] - 2026-10-29

Fin de l'étape 2 (« enrichissement ») : les 34 exigences de l'étape à
l'état FAIT — 32 réalisées en cinq lots, B-05 et A-12 par anticipation à
l'étape 1. Deux exécutables : l'interface `ximod-translator` et la ligne de
commande `ximod-cli`. Les fichiers de travail de l'étape 1 (dictionnaires
du mod de format 2, pivots) s'ouvrent sans perte et passent au format 4 à
leur premier enregistrement.

### Corrigé (recette de l'étape 2)
- Choisir un pays (bouton Cible) : les noms des pays sont dans la langue
  de l'interface et triés sur eux (ils restaient en français quelle que
  soit la langue) ; le filtre prend aussi le nom anglais, les endonymes et
  le code, comme la base pays / langues (E-11).

### Ajouté (étape 2, lot 5)
- Base pays / langues (E-11, décision D-84) : Options › Propriétés…
  ouvre une fenêtre indépendante, en lecture seule, reprise de XIMOD
  Architect, qui explore les fichiers de référence. Onglet Pays : filtre
  sur le nom affiché, le nom anglais, les noms officiels et le code (sans
  casse ni accents), drapeau, langues officielles (codes ISO 639-3 /
  639-1, nom du pays dans la langue, police, interface disponible) et
  autres langues parlées. Onglet Langues : filtre sur le nom et les codes
  (code exact en tête), endonyme, codes, police, interface, pays où la
  langue est parlée (officiels en tête, nom local, drapeau). Un clic sur
  une langue ou un pays ouvre l'autre onglet ; flèches, Tab, Page
  précédente / suivante, Début, Fin, Entrée ; listes virtuelles (6 777
  langues) ; géométrie mémorisée (`[Window] PropsX/Y/W/H`).
  `CountryNames.json` est désormais lu (noms des pays dans la langue de
  l'interface, anglais à défaut).
- Profils compatibles OpenAI (D-03, décisions D-85, D-86) : un fichier
  JSON par profil (`assets/data/online/profiles/` : openai, local pour
  Ollama, mistral ; copies et profils de l'utilisateur dans
  `data/online/profiles/`) — nom, point d'accès, modèle, en-têtes,
  consigne avec `%lang_source%` / `%lang_dest%` (syntaxe d'ESP-ESM
  Translator) ou `{src}` / `{dst}`, `%game%`, `%n%`, format de la
  réponse (tableau JSON ou lignes séparées par des repères numérotés),
  température, jetons max, `top_p`, pénalités, garde-fous propres (lignes
  par lot, caractères par requête, requêtes par minute, délai). Chaque
  profil est un service : sa ligne dans la liste, sa clé dans le
  trousseau (`openai:<id>`), son cache, son nom dans la provenance
  (« en ligne (Ollama local) »). Une réponse incomplète est refusée et le
  lot renvoyé ligne par ligne. Éditeur dans Paramètres › En ligne
  (nouveau, dupliquer, supprimer, restaurer, validation, Tester… avec le
  profil tel que saisi).
- Traduction de masse (D-04) : Traduire en ligne reçoit la portée « Tout
  le document » (« n lignes à traduire sur N »), indépendante des filtres,
  proposée à l'ouverture sans sélection puis mémorisée
  (`[Online] LastScope`). Le bloc « Ce qui sera envoyé » dit les lignes
  écartées (verrouillées, Ignoré, jamais affichées ; déjà traduites) et
  celles que le cache servira ; le bilan sépare reçues, refusées (par
  raison) et non envoyées après un arrêt. Fermer est grisé pendant
  l'envoi. Le calcul de la portée, la préparation et la pose des réponses
  sont dans la bibliothèque (`online::mass`), commune au dialogue et à la
  future commande `translate` (D-06).
- Importateur générique texte / XML / JSON (D-08, décisions D-87, D-88) :
  aux formats texte par ligne s'ajoutent trois types structurés — JSON
  par chemins (`$..text`, `[*]`, `..`, « | »), XML par chemins (`//b`,
  `@attr`, `text()`, `[@k='v']`), CSV par colonnes (nom ou numéro,
  séparateur deviné, en-tête) — avec une description des champs (textes,
  identifiant, enregistrement, champ, exclusion, traduction) résolus par
  colonne, propriété, chemin, valeur fixe, automatique, nom de la
  propriété ou regex (exclusion, extraction par groupe `text`). Formats
  livrés : MCM Helper (`MCM/Config/*/config.json`), JSON SKSE (tableau
  id / text), XML (textes et attributs), CSV à colonnes. Nouveau dialogue
  Fichier › Importer avec un format… (aussi ouvert par Ouvrir… pour un
  fichier qu'aucun motif précis ne prend) : format, description des
  champs, bilan (valeurs lues, lignes par champ, valeurs écartées,
  unicité des identifiants, cible) et aperçu des cinquante premières
  lignes ; dernier format retenu par extension (`[Import] Last.<ext>`).
  L'éditeur Formats texte reçoit le type, la table des champs et l'essai
  structuré. L'écriture remplace les seules valeurs traduites à leur
  position (échappement JSON, entités XML, guillemets CSV) dans
  `<nom>_<langue>.<ext>`, ou dans le fichier même quand la traduction y
  est une propriété ou une colonne (lue alors au statut Validé) ; un
  fichier modifié depuis son ouverture n'est pas écrasé. Charger dans la
  relecture apparie une copie traduite d'un fichier texte par champ,
  identifiant et rang.
- Analyser un dossier (D-07, décision D-90) : Fichier › Analyser un
  dossier… ouvre une fenêtre indépendante qui parcourt un dossier
  (sous-dossiers en option) et liste plugins, fichiers d'interface et
  formats texte reconnus, scripts (option), DSD et dictionnaires ; pour
  chaque fichier, en arrière-plan avec progression et arrêt, la pile de
  dictionnaires résolue comme à l'ouverture et appliquée à blanc (rien
  n'est ouvert ni écrit), les règles d'exclusion posées : lignes,
  traduites par la pile (détail par dictionnaire), restantes,
  verrouillées, dictionnaire du mod (reconnu, autre version, absent) et
  état (traduit, presque fini, à traduire, mise à jour, source de
  dictionnaire). Double-clic, Ouvrir la sélection, Tout ouvrir (à
  traduire), export CSV ; `[Scan] LastDir / Subdirs / Scripts`,
  `[Window] ScanX/Y/W/H`. L'ouverture sans fenêtre est dans la
  bibliothèque (`headless`), commune à l'analyse et à la future ligne de
  commande.
- Comparer deux dictionnaires (D-07, décision D-89) : Dictionnaires ›
  Comparer deux dictionnaires… met côte à côte deux fichiers de
  traduction ou documents ouverts (appariement par enregistrement et
  texte, ou texte seul ; casse et espaces de bord ignorés en option) :
  sources communes, identiques, divergentes, seulement dans A ou B, en
  quatre onglets ; double-clic vers la ligne d'un document ouvert ;
  rapport CSV ; fusion en dictionnaire XIMOD dont les lignes divergentes
  portent la marque ambiguë ; `[Window] CompareX/Y/W/H`.
- Ligne de commande (D-06, décisions D-91 à D-93) : un second
  exécutable, `ximod-cli` (`ximod-cli.exe`, programme console), livré à
  côté de l'interface, qui appelle les mêmes fonctions de la
  bibliothèque — `info`, `import`, `apply`, `assist`, `translate`,
  `check`, `export`, `manifest`, `compare`, `scan`, `convert`, `package`,
  `batch`. Il lit le Config.ini de l'interface (ou celui de
  `--config <dossier>`) sans jamais l'écrire, n'ouvre aucune fenêtre, ne
  touche ni aux récents ni à l'autosauvegarde. Options communes
  `--game`, `--from`, `--to`, `--json` (un objet JSON par commande ou par
  étape sur stdout, les messages sur stderr), `--quiet`, `--verbose`,
  `--lang`, `--no-color`, `--help` dans la langue des messages ; codes
  de retour 0 fait, 1 avec réserves, 2 non fait. `apply` ouvre comme
  l'interface (dictionnaire du mod reconnu, pile, règles), `--dict`,
  `--base`, `--no-personal`, `--mode`, `--overwrite`, et écrit sur
  `--write-dict` le dictionnaire du mod et l'historique ; `assist` et
  `translate` (`--max-chars` obligatoire, rien envoyé au-delà, clés du
  trousseau, de secrets.json ou de `XIMOD_KEY_<SERVICE>`, jamais
  affichées) font une opération chacun, lignes En attente ; `check`
  rend 1 sur une erreur (balises, nombres, encodage) ; `export` écrit
  strings, hybrid, override, full ou dsd depuis le dictionnaire du mod
  avec la politique des statuts, sans jamais remplacer le plugin source.
  Fichier de lot JSON (format 1) validé en entier avant la première
  étape, motifs `files`, sorties dans `<out>/<plugin>/`, `stopOnError`.
  Manifeste `manifest.json` (format 1, module `manifest`) : plugin source
  et CRC-32, dictionnaire du mod et ses statuts, pile, statuts exportés,
  sorties avec taille, CRC-32 et lignes, vérifications. `docs/cli/` :
  `rebuild.sh`, `rebuild.ps1`, `batch-example.json`, `github-actions.yml`.
  84 messages `cli-*` dans les 33 langues.


### Ajouté (étape 2, lot 4)
- Matrices de remplacement (B-17) : suites nommées de règles appliquées
  en une fois depuis Rechercher et remplacer (choix de la matrice, sens
  direct ou inverse, aperçu, une seule opération annulable « Matrice
  <nom> »). Les lignes touchées gardent leur statut et prennent la
  provenance « remplacement (<matrice>) » (code `replace:<matrice>`,
  filtre Provenance ▸ Remplacement) ; lignes ignorées, jamais affichées en
  jeu et sans traduction exclues. Matrices rangées par langue cible :
  `assets/data/matrices/<code>.json` livrés (fra : typographie française,
  guillemets « », apostrophes ; deu : „…“ et ’ ; spa : « » et ¿ ¡ ; eng
  vide) et `all.json` commun (espaces superflus), copie utilisateur dans
  `data/matrices/` de la configuration. Nouvelle fenêtre Outils ›
  Données › Matrices de remplacement… : langue, liste des matrices
  (ajouter, dupliquer, supprimer), règles ordonnées (mode, casse, mot,
  inverse dérivé ou explicite pour une regex), aperçu dans les deux sens,
  test sur la sélection, enregistrement validé (jamais de fichier
  invalide) avec contrôle d'idempotence, restauration de la version
  livrée, import d'un fichier de règles A-11 ; pris en compte sans
  redémarrage. « Ajouter à une matrice… » range le motif courant dans une
  matrice existante ou nouvelle. Dernière matrice retenue par langue
  (`[Replace] LastMatrix`), géométrie `[Window] MatricesX/Y/W/H`.
  Fonctionne aussi dans l'onglet Tous les mods.
- Appariement normalisé des dictionnaires (B-19, décisions D-71 et D-72) :
  dans chaque dictionnaire de la pile, après le texte seul et avant les
  anciennes données, un niveau qui retrouve une source identique aux
  nombres, à la ponctuation de phrase, aux espaces et à la casse près
  (NFKC ; balises comparées telles quelles ; crochets, guillemets et
  symboles devant être les mêmes). La traduction est adaptée : nombres
  reportés d'après leur valeur (ordre et virgule décimale de la
  traduction respectés), ponctuation finale (pleine chasse après un texte
  CJK), casse de la première lettre ou du texte en capitales ; sinon
  « nombres non reportés », que la vérification Nombres signale. Lignes
  En attente, jamais Validé ; provenance
  `dict:<rôle>:norm:<fichier>|rep:<codes>|src:<source>` (« normalisée :
  nombres, ponctuation », libellé long avec la source retrouvée), filtre
  Provenance › Normalisée, compte dans la barre d'état, ambiguïté quand
  plusieurs sources de même forme donnent des traductions différentes.
  Vaut aussi pour les anciennes données et le dictionnaire personnel (En
  attente). Mode « Normalisé » dans Appliquer les dictionnaires, inclus
  dans Strict comme à l'ouverture ; case de Paramètres › Avancé
  (`[Fuzzy] Normalize=1`). Recette : Skipper 0.3.0 → 0.5.3 (aucun faux
  appariement, 20 lignes dérivées toutes retrouvées), base texte seul de
  Skyrim sur les traductions françaises du lot 4.
- Règles d'exclusion (B-18, décisions D-73 à D-75, D-83) : un fichier
  unique `assets/data/rules/exclusions.json` (copie utilisateur dans
  `data/rules/`) réunit les motifs et les fonctions des scripts de
  l'ancien `papyrus_rules.json` (une copie utilisateur de celui-ci est
  fusionnée une fois, puis renommée `.bak`) et des règles nouvelles pour
  le texte des enregistrements et des fichiers texte : clé $, chemin
  (sans espace : antislash, deux barres obliques ou extension), sans
  lettre, identifiant (soulignement, lettre suivie d'un chiffre ou trois
  majuscules internes), FormID ou couleur hexadécimale, adresse, balises
  seules, clés techniques des MCM `config.json` et clés d'identifiants des
  JSON. Règles déduites du document (EDID du plugin pour les chaînes de
  script et les fichiers texte, clés du fichier d'interface, noms de
  fichiers du mod) ; règles apprises par la généralisation proposée après
  un Ignoré à la main (texte exact, début commun au choix, champ ou clé —
  grisé pour un champ affiché), locales dans l'en-tête du dictionnaire du
  mod (`rules`, `rulesOff`, aussi dans le pivot) ou globales dans la
  copie utilisateur. Les règles ne verrouillent que les lignes que
  personne n'a décidées, posées après les dictionnaires ; la main
  l'emporte. Fenêtre Outils › Règles d'exclusion… : Règles actives
  (origine, portée, jeu, comptes, exemple ; décocher désactive la règle
  pour le document, Voir les lignes, Rendre globale / locale,
  Réappliquer), Éditeur (validation, essai sur le document, fonctions,
  restauration, prise en compte sans redémarrage), Suggestions (champ
  jamais traduit par la base du jeu, groupes d'Ignoré à la main).
  Opérations annulables Règle activée / désactivée, Généralisation,
  Règles réappliquées ; `[Rules] ProposeGeneralization`,
  `[Window] RulesX/Y/W/H`. Mesuré sur les traductions humaines : SS2
  1 732 lignes verrouillées dont 14 traduites par le traducteur, Cheat
  Terminal 51 dont 2, Settlement Management Software 26 dont 0, MCM
  config.json de SS2 1 041 dont 0, CACO 74 dont 0, RFM 0, Skipper (JSON)
  1 731 sur 1 788.
- Glossaire de termes par groupe (B-15, décisions D-76, D-77, D-80,
  D-81) : un glossaire par paire de langues, commun aux jeux,
  `assets/data/assist/glossary_<SRC>-<DST>.json` (livrés : ENG-FRA,
  289 termes en huit groupes — matériaux, couleurs, races, titres et
  métiers, lieux et factions, armures, armes, modificateurs — et ENG-ZHO,
  40 termes repris de la traduction chinoise d'Immersive Weapon
  Integration), copie utilisateur dans `data/assist/`, et un glossaire
  par jeu facultatif `glossary_<Jeu>_<SRC>-<DST>.json` dont les termes
  passent devant. Chaque terme : original, traduit (vide : le mot est
  omis), classe (nom, lié, adjectif, couleur, matière, origine, sans
  classe), mot entier, position (début, fin), casse, genre, formes d'un
  adjectif. Composition par classes (section `composition` de
  `patterns_<SRC>-<DST>.json`, livrés ENG-FRA et ENG-ZHO) : nom en tête,
  ordre des classes de la cible, accord en genre et en nombre, matières
  successives fusionnées, séparateurs « , » et « - » d'une variante
  omis ; sans nom, ordre de la source. Nouveau dialogue Dictionnaires ›
  Appliquer les assistants… (portée, écrasement, aperçu ; jamais à
  l'ouverture) : seules les lignes entièrement couvertes sont remplies,
  En attente, provenance « règle : composition » ou « règle :
  glossaire » (codes `rule:compose`, `rule:glossary`, filtre Provenance ›
  Règle) ; lignes verrouillées et Ignoré intactes ; une seule opération
  annulable « Assistants (n lignes) ». F9 traduit par le glossaire une
  source entièrement couverte, et le prend comme premier niveau de
  fragments ; l'onglet Vocabulaire montre les termes de la ligne avec
  leur groupe. Ctrl+Alt+G (ou menu contextuel de la source) : Ajouter au
  glossaire, sans ouvrir la fenêtre. Nouvelle fenêtre Outils › Données ›
  Glossaire, motifs et dérivations… : paire et fichier (paire ou jeu),
  groupes, termes filtrables, terme en cours avec ses formes, essai avec
  le chemin suivi, import CSV/TSV dans un groupe, validation (original
  vide, en double dans son groupe, formes sans forme m.), restauration,
  prise en compte sans redémarrage ; `[Window] AssistX/Y/W/H`. Recette :
  Immersive Armors (865 ARMO:FULL) — 398 lignes avec 12 matériaux et les
  types d'équipement, 727 avec un glossaire de 100 termes, 838 avec le
  glossaire livré (396 identiques à la traduction française DSD) ;
  Immersive Weapon Integration en chinois, « Nord Steel Warhammer » →
  « 诺德钢制战锤 », 166 lignes dont 146 identiques à la traduction CHS.
- Motifs à variables (B-16, décisions D-78, D-81) : règles « %A% of %B% »
  → « %A% de %B% » du fichier `patterns_<SRC>-<DST>.json` (copie
  utilisateur dans `data/assist/`), essayées dans l'ordre après la
  composition (motif 1), chacune avec sa longueur maximale. Chaque
  variable capture au moins un mot et se traduit par un terme du
  glossaire, par les dictionnaires de la pile (texte seul, puis forme
  normalisée), par un autre motif (motif récursif, trois niveaux) ou par
  la composition des termes ; `vars` restreint les résolveurs
  (`glossary`, `glossary:adjective`, `bases`, `patterns`, `pattern:<n>`,
  `copy` pour un nom propre). Accord par la variable de tête (`head`,
  sinon la dernière de la cible), `%A:cap%` et formes imposées
  `%A:f%` ; aux jonctions créées par le motif, article accordé (« le » →
  « la », « les », rien devant un nom propre), élision devant une voyelle
  ou un h muet (liste des h aspirés), contractions (« de le » → « du »…).
  Livrés pour ENG-FRA : « of », « of the », parchemin, potion, possessif
  « 's », parenthèses. Lignes En attente, provenance « règle : motif n »
  (code `rule:pattern:<n>`) ; case « Motifs à variables » d'Appliquer
  les assistants, aperçu par motif. F9 essaie les motifs avant les
  fragments ; l'onglet Vocabulaire montre le motif qui s'applique.
  Nouvel onglet Motifs de la fenêtre Glossaire, motifs et dérivations :
  motifs ordonnés (▲ ▼), motif en cours (source, cible, longueur,
  récursif, tête, résolveurs), règles de la langue, essai avec le chemin
  complet, validation (variables des deux côtés, longueur de 1 à 500,
  pas de motifs qui se rappellent), restauration. Recette : Immersive
  Weapons (237 WEAP:FULL), glossaire de 100 termes, 153 lignes dont 142
  identiques à l'une des traductions DSD ; Immersive Armors avec le
  glossaire et les motifs livrés, 849 lignes dont 408 identiques.
- Classe de terme « peuple » (B-16) : les adjectifs de peuple ou de lieu
  (nordique, elfique, dwemer, impérial…), placés contre le nom, avant les
  autres adjectifs (« Lance nordique affûtée en acier »).
- Dérivation d'objets par règles (B-21, décisions D-79, D-82) : règles
  par jeu `derive_<Jeu>.json` (livrés : Skyrim, SkyrimSE, Fallout4,
  Oblivion, Starfield ; copie utilisateur dans `data/assist/`) — un
  objet dérivé (`SCRL:FULL` « Scroll of %A% ») dont %A% est résolu par le
  nom traduit d'un parent (`SPEL:FULL`, `MGEF:FULL`) cherché dans le
  document, les autres fichiers ouverts de la même paire, puis les
  dictionnaires de la pile (signature, sinon texte seul). Motif cible
  par langue (règle inactive pour une langue absente), préfixes et
  suffixes retirés du parent (`strip`, « Invocation - »), `%A:low%` sauf
  nom propre, élision aux jonctions, autres variables par le glossaire
  (degré %G% d'une potion) accordées au genre déclaré par la règle ; le
  tome (`"ref": "teaches"`) prend d'abord le sort qu'il enseigne (BOOK
  DATA, par FormID). Lignes En attente, provenance « règle : dérivation
  scroll (de <EDID du parent>) » (`rule:derive:<id>:<edid>`), ambiguës
  quand deux parents de même nom ont des traductions différentes. Suivi
  du parent : quand sa traduction change, les lignes dérivées encore En
  attente sont recalculées dans la même opération annulable, la barre
  d'état le dit ; une ligne validée ou retouchée ne suit plus. Case
  « Dérivations » d'Appliquer les assistants (après le glossaire et les
  motifs, dont les traductions servent de parents), aperçu par règle,
  parents sans traduction et introuvables ; avertissement quand ni
  glossaire, ni motif, ni règle n'existe pour la paire et le jeu. Onglet
  Dérivations (jeu) de la fenêtre Glossaire, motifs et dérivations :
  règles ordonnées, règle en cours (objet, parents, sort enseigné,
  motifs, préfixes à retirer, genre, résolveurs), règles inactives pour
  la langue signalées, essai sur le document actif, validation
  (signatures AAAA:BBBB, identifiant unique, variables), restauration.
  Recette : Apocalypse - Magic of Skyrim, sorts et effets traduits par la
  traduction française chargée comme dictionnaire du mod, 326 des 327
  parchemins et tomes dérivés (136 SCRL, 190 BOOK), tous identiques à la
  traduction humaine ; Complete Alchemy and Cooking Overhaul, 345
  potions, poisons et breuvages dérivés, 355 avec la base Skyrim SE dans
  la pile (« Healing » → « Soin »), 256 identiques à la traduction DSD.

### Modifié (étape 2, lot 5)
- Traduction en ligne : la connexion OpenAI unique de l'étape 1 devient
  le profil « openai » ; `[Online] Service=openai` est lu comme
  `openai:openai`, la clé, le point d'accès et le modèle de l'étape 1
  sont repris tant que le profil n'a pas de copie utilisateur ;
  `assets/data/online/prompt.txt` n'est plus livré (sa consigne est dans
  le profil). Tester… : un nom de modèle contenant « : »
  (`mistral-nemo:latest`) est désormais reconnu dans la liste du serveur.
- Traduire en ligne : une ligne non envoyée parce que l'envoi a été
  arrêté n'est plus comptée parmi les refusées, mais à part (« non
  envoyées (arrêt) ») ; la portée par défaut est « Tout le document » au
  lieu de la vue filtrée.
- Formats texte : `text_formats.json` passe au format 2 (champ `kind`) ;
  un fichier de la première version est lu et réécrit sans perte. La
  boîte « Quel format pour … ? » est remplacée par le dialogue Importer
  avec un format ; un `.xml` qui n'est pas un XML de xTranslator s'ouvre
  désormais comme fichier texte.
- Dictionnaire du mod : son écriture (lignes, anciennes données,
  en-tête) est une fonction de la bibliothèque (`mod_dict`), commune à
  l'interface et à la ligne de commande. L'ouverture sans fenêtre
  (`headless`, D-07) pose aussi le verrou des chaînes VMAD et reprend les
  règles que le dictionnaire du mod retient, comme l'interface : l'analyse
  d'un dossier compte ces lignes parmi les verrouillées.
- Empaqueter une traduction : un `manifest.json` parmi les fichiers va
  dans `fomod/manifest.json`. Chaîne de livraison : `ximod-cli` et
  `docs/cli/` dans les archives Windows, Linux et macOS, l'installateur
  et le bundle (`Contents/MacOS/ximod-cli`) ; le manuel prend un chapitre
  18 « La ligne de commande » (les chapitres suivants sont renumérotés).

### Modifié (étape 2, lot 4)
- Composition ENG-FRA livrée : l'ordre des classes est nom, lié,
  adjectif, couleur, origine, matière (origine avant matière, comme la
  traduction française d'Immersive Armors : « Masque du Tribunal en
  bronze »), au lieu de l'exemple de la spécification (matière avant
  origine) ; le fichier des motifs le règle par paire.
- Provenance : la catégorie « règle » du filtre Provenance, réservée au
  lot 1, regroupe désormais les lignes des assistants.
- Composition ENG-FRA : ordre nom, lié, peuple, adjectif, couleur,
  origine, matière ; les races du glossaire livré passent dans la classe
  peuple. Dialogue Appliquer les assistants : la case « Composition par
  classes » devient « Motifs à variables » (composition et motifs) ; sans
  elle, le glossaire seul garde l'ordre de la source.
- Rechercher et remplacer : les boutons « Appliquer un fichier de
  règles… » et « Enregistrer comme règle… » (A-11) sont retirés,
  remplacés par les matrices ; un fichier de règles existant s'importe
  comme matrice.
- Appliquer les assistants : une ligne qu'une règle de dérivation sait
  dériver prend la dérivation plutôt que le motif générique (« Scroll of
  %A% ») ou le glossaire ; les comptes de l'aperçu en tiennent compte.
- Pile d'annulation : une opération peut recevoir après coup les lignes
  qui suivent leur parent (dérivations), annulées avec elle.
- Scripts : « Outils › Scripts › Ouvrir les règles des scripts » ouvre
  l'éditeur des règles d'exclusion ; `papyrus_rules.json` n'est plus livré
  (ses règles sont dans `exclusions.json`, comportement inchangé pour les
  chaînes de script).

### Ajouté (étape 2, lot 3)
- Pages de code et forçage (D-09, décisions D-59 et D-60) : treize pages
  en plus d'UTF-8 et Windows-1252 — Windows-1250, 1251, 1253 à 1258, 874,
  Shift_JIS, GBK, EUC-KR, Big5 (bibliothèque encoding_rs) — déduites des
  langues du document (`assets/data/codepages.json`) : la langue source
  décide la page de lecture d'un plugin d'un jeu ancien quand elle est
  connue, sinon la détection par le détecteur de langue sur les pages
  candidates (un plugin russe est relu en 1251), sinon 1252 ; la langue
  cible décide la page d'écriture de tout ce que l'export écrit (plugin,
  patch, STRINGS de Skyrim LE, fichiers texte) et du compteur d'octets.
  Fichier ▸ Encodage du fichier… (jeux en page de code seulement) :
  lecture et écriture automatiques ou forcées, aperçu décodé, Relire ;
  forçage mémorisé dans l'en-tête du dictionnaire du mod (`enc_in`,
  `enc_out`) et le pivot. Relecture et réécriture identiques octet pour
  octet (octets non définis gardés en zone privée). La vérification
  « Caractère non encodable » (ex-« Caractères hors Windows-1252 ») nomme
  la page ; l'export refuse les lignes fautives avec « Remplacer par ? et
  exporter ». Barre d'état « Lu en … », page dans le badge de l'onglet.
  43 messages dans les 33 langues.
- Localiser un mod (C-09, décisions D-61 et D-62) : pour un plugin aux
  textes en ligne d'un jeu à STRINGS (Skyrim, Skyrim SE, Fallout 4,
  Starfield), la sortie s'appelle « STRINGS (localiser le plugin) » ;
  l'export écrit le plugin localisé (drapeau Localized, un identifiant par
  champ attribué de 1 à n dans l'ordre du fichier, 0 pour un champ vide,
  en-tête TES4 en texte) et ses fichiers `Strings/<plugin>_<Langue>.STRINGS`,
  `.DLSTRINGS`, `.ILSTRINGS` dans la langue cible, et dans la langue source
  si la case est cochée (par défaut, `[Export] LocalizeSourceStrings`).
  Les enregistrements compressés modifiés sont réécrits décompressés ; un
  plugin déjà localisé est refusé. Le dialogue d'export liste les fichiers
  produits, la table utilisée et les champs de texte que la table ne
  connaît pas. Table des champs localisables (`assets/data/lstrings/`,
  copie utilisateur `data/lstrings/`) dérivée des maîtres officiels à
  Dictionnaires › Compiler les bases des jeux, fichier livré en secours ;
  colonne « Liste » en lecture seule dans l'éditeur des champs
  traduisibles ; la délocalisation s'en sert pour distinguer un
  identifiant d'un drapeau. Fallout 4 : TERM:UNAM, OMOD:DESC et FLST:FULL
  ajoutés aux champs traduisibles. Localisation puis délocalisation
  identiques enregistrement pour enregistrement sur six plugins de
  recette. 17 messages dans les 33 langues.
- Chaînes VMAD des scripts (C-08, décision D-63) : les propriétés chaîne
  des scripts attachés aux enregistrements de Skyrim, Fallout 4 et
  Starfield (bloc VMAD versions 2 à 6, tableaux de chaînes, structures de
  Fallout 4, alias de quête ; fragments d'INFO, PACK, PERK, QUST, SCEN,
  TERM et TMLM lus et laissés en identifiants) deviennent des lignes
  `REC:VMAD`, index = rang dans le bloc, commentaire `script.propriété`.
  Verrouillées par défaut (Ignoré, « verrouillée : VMAD désactivé »),
  déverrouillées par Outils › Scripts › Traduire les chaînes VMAD de ce
  fichier, opération annulable mémorisée dans l'en-tête du dictionnaire du
  mod (`vmad`) et le pivot (`meta.vmad`) ; case cochée, les règles de
  `assets/data/scripts/papyrus_rules.json` (clé $, chemin ou fichier, sans
  lettre, mot isolé) verrouillent les identifiants probables, qu'une saisie
  ou un statut déverrouille. Écriture dans le plugin complet, le patch de
  surcharge et les plugins délocalisé ou localisé : seules les chaînes
  traduites changent, un bloc sans ligne exportée est recopié octet pour
  octet, un bloc illisible est conservé tel quel (compteur « Blocs VMAD non
  lus » et journal). Format 2 du fichier de champs (joker `*:VMAD`),
  provenance et filtre « Verrouillée », compteur « Chaînes VMAD » dans la
  barre d'état, avertissement du dialogue d'export pour les sorties STRINGS,
  hybride et DSD. 22 messages dans les 33 langues.
- Scripts Papyrus compilés (C-06, décisions D-64, D-65 et D-68) : lecture
  des `.pex` de Skyrim (3.1, 3.2), Fallout 4 et 76 (3.9) et Starfield
  (3.12) — en-tête, table de chaînes, débogage, objets, structures,
  propriétés, états, fonctions et instructions — et classement de chaque
  chaîne en identifiant ou littéral ; seuls les littéraux sont des lignes
  (`SCRIPT:STR`, EDID = le script, index = l'indice dans la table,
  commentaire : fonction et appel). Outils › Scripts › Scripts du mod… :
  fenêtre indépendante qui trouve les scripts libres de `Scripts/` et ceux
  des archives BSA/BA2 du mod (le libre l'emporte), leurs sources `.psc`,
  et les liste (format, chaînes, textes, traduits, source, état) ; dans le
  dossier `Data` du jeu, seuls les scripts que le plugin nomme. Document
  « <plugin> — scripts », un par plugin, avec son dictionnaire du mod
  `<plugin>_scripts_<SRC>-<DST>.ximdb` ; un `.pex` s'ouvre aussi seul.
  Règles de verrouillage `papyrus_rules.json` (fonctions dont les arguments
  sont des identifiants, clé $, chemin, sans lettre, mot isolé ; chaîne
  partagée avec un nom : « identifiant du script »), copie utilisateur
  ouverte par Outils › Scripts › Ouvrir les règles des scripts. Sortie
  « Scripts (.pex) » : les scripts traduits écrits en fichiers libres dans
  `Scripts/`, la table de chaînes seule réécrite (une chaîne partagée avec
  un nom est ajoutée en fin de table), un script sans ligne exportée non
  écrit. Onglet Script de la fenêtre d'édition : fonction, usages, partage,
  et la source `.psc` autour du littéral. Les 1 506 scripts des mods de
  recette sont lus et réécrits à l'identique. 42 messages dans les 33
  langues.
- Scripts des jeux anciens et sources Papyrus (C-07, décisions D-66 et
  D-67) : pour Oblivion, Fallout 3 et New Vegas, les textes des
  instructions `Message` et `MessageBox` (message et boutons) et de
  `MessageEx` / `MessageBoxEx` des extenseurs (boutons séparés par « | »)
  des scripts SCPT et des scripts de résultat des quêtes et des répliques
  deviennent des lignes `SCTX` du document du plugin (commentaire :
  instruction, rang du message ou du bouton, commentaire « ; » qui
  précède). Chaque texte est retrouvé dans le compilé SCDA (« %r » en saut
  de ligne) ; l'export corrige ensemble la source SCTX et le compilé
  (chaîne et longueur, longueur de l'instruction et du bloc `Begin`, taille
  dans SCHR), un script sans ligne exportée restant identique octet pour
  octet ; un texte introuvable dans le compilé est verrouillé « compilé
  introuvable ». Onglet Script : source du script en lecture seule,
  instruction en bleu, commentaires en gris, lignes du script, place dans
  le compilé ; la fenêtre Scripts du mod liste, pour ces jeux, les scripts
  du plugin avec des messages. Pour les `.pex`, l'option « Reporter aussi
  la traduction dans la source .psc » (`[Scripts] CopyToSource`, décochée)
  écrit à l'export la source avec les littéraux traduits. Les 570 messages
  des quatre plugins de recette (Oblivion, dont un plugin russe, et New
  Vegas) sont retrouvés dans le compilé et réécrits avec des compilés bien
  formés. 10 messages dans les 33 langues.

### Corrigé (étape 2, lot 3)
- Le séparateur de chemin de menu « ▸ » des messages (introuvable dans la
  police par défaut) devient « › » dans les 33 langues.
- Plugin localisé : un champ d'identifiant 0 (chaîne vide du moteur) n'est
  plus extrait comme ligne introuvable.
- Liste Sortie : elle suit le jeu du document. Les sorties que le jeu ne
  peut pas recevoir sont grisées et non sélectionnables, avec une
  infobulle (Morrowind : Module complet ; Oblivion, Oblivion Remastered,
  Fallout 3, New Vegas : Correctif et Module complet ; DSD réservé à
  Skyrim SE). Une sortie qui ne convient plus (session précédente, jeu
  changé) vide la liste, la barre d'état l'explique ; l'export refuse une
  sortie que le jeu ne peut pas recevoir. L'avertissement DSD de la barre
  d'outils disparaît, devenu sans objet.
- Fiches de Fallout 3 et New Vegas : pas de STRINGS (`localizedStrings`
  faux, sorties Correctif et Module complet), comme le moteur.

### Modifié (étape 2, lot 3)
- Français : les modes de sortie prennent un nom français — Hybride,
  Correctif (correctif de remplacement, « Override patch ») et Module
  complet (module d'extension complet, « Full plugin ») —, et les messages
  qui parlaient de « patch » disent « correctif ». Revient sur le choix du
  lot 2 de garder ces termes en anglais pour le français ; les 32 autres
  langues ne changent pas, et chacun peut ajuster les clés
  `output-mode-*` de son fichier de langue.

### Ajouté (étape 2, lot 2)
- Recherche avancée (A-19, décisions D-51 et D-52) : Outils ▸ Recherche
  avancée… (Ctrl+Alt+F), fenêtre indépendante à six critères combinés en
  ET — Source, Traduction, EDID ou FormID, GRUP, Champ, Mot-clé (source,
  traduction, EDID, signature, commentaire) —, chacun avec sa case `.*`
  (expression régulière) et sa case Casse ; expression invalide en rouge,
  l'erreur en info-bulle ; comparaison des groupes capturés entre source et
  traduction (identiques ou différents, dans l'ordre, espaces de bord
  retirés) ; statuts et lignes jamais affichées ; aperçu 250 ms après la
  frappe avec les premières paires de groupes différentes. « Appliquer à la
  grille » pose une pastille « Recherche avancée (n) » dans la barre de
  filtres, en ET avec elle, retirée d'un clic ou par Réinitialiser ;
  Sélectionner, Copier la liste (« FormID ⇥ EDID ⇥ source ⇥ traduction »).
  Préréglages nommés dans `data/search_presets.json` du dossier de
  configuration ; quatre livrés en lecture seule dans
  `assets/data/search_presets.json` (Nombres différents, Balises
  manquantes, Traduction égale à la source, Lignes sans EDID), un
  préréglage de l'utilisateur du même nom les remplace. Géométrie
  `[Window] SearchX/Y/W/H`. 41 messages dans les 33 langues.
- Concordance dans les dictionnaires (A-20, décision D-53) : Outils ▸
  Concordance… (Ctrl+K, aussi dans le menu de la ligne, dans la fenêtre
  d'édition et dans le menu contextuel de ses zones Source et Traduction),
  fenêtre indépendante qui cherche un terme — le texte sélectionné dans la
  fenêtre d'édition, sinon le mot sous le curseur, sinon la source de la
  ligne — dans les sources et les traductions des dictionnaires actifs du
  fichier, anciennes données et paires texte seul comprises ; « contient »
  sans casse, Mot entier, Dans les traductions aussi, Dictionnaire du mod
  seul ; deux caractères au moins. Une paire par (source, traduction) et
  par dictionnaire, dans l'ordre de la pile, avec l'enregistrement de la
  première ligne et le nombre de lignes, le terme surligné, 500 paires au
  plus. Calcul dans un fil d'arrière-plan annulé par la frappe suivante
  (un million de lignes en moins d'une seconde). Copier (ou double-clic) ;
  Dans la ligne insère la traduction à la place de la sélection ou au
  curseur de la zone Traduction (remplacement entier si la ligne est
  vide), opération annulable, provenance « saisie ». Géométrie
  `[Window] ConcordX/Y/W/H`. 29 messages dans les 33 langues.
- Sauvegarde partielle (B-10, décision D-55) : Fichier ▸ Enregistrer une
  partie…, dialogue modal qui écrit une partie du document dans un format
  de travail — portée (sélection, vue filtrée, tout le document), statuts
  (les sept boutons de la barre de filtres), GRUP (pastilles des
  signatures du document, Tous) et champ ; lignes jamais affichées en jeu
  exclues ; aperçu du nombre de lignes par GRUP et par statut, bouton grisé
  à zéro. Formats d'Enregistrer sous… par l'extension (.json pivot avec
  provenance, relecture et historique des lignes écrites, .sst, .eet,
  .dsd, .csv, .xml), STRINGS refusé ; option « Lignes sans traduction :
  conserver la source ». Fichier proposé « <document>_partie.json » dans
  le dossier du dernier enregistrement. Le document ouvert n'est ni
  modifié ni renommé, le dictionnaire du mod n'est pas écrit ; choix
  retenus par fichier pour la session. Le fichier se recharge par Charger
  dans la relecture…. 25 messages dans les 33 langues.
- Sauvegarde de l'original à l'export (E-06, décision D-58) : quand le
  fichier choisi pour Exporter la traduction… existe (plugin, STRINGS,
  fichier texte, JSON DSD), il est d'abord copié dans `backups/` à côté de
  lui sous `<nom>_<AAAAMMJJ-HHMMSS>.<ext>` ; l'écriture n'a lieu que si la
  copie a réussi, sinon l'export s'arrête et le dit, le fichier existant
  intact. Rotation après l'écriture : les N copies les plus récentes par
  fichier (5 par défaut, de 1 à 50). Option « Conserver l'original avant
  d'écraser un fichier existant » dans Paramètres ▸ Avancé ▸ Sorties
  (`[Output] KeepOriginal`, `OriginalCopies`) ; décochée, la copie reste
  faite quand l'export remplace le plugin ouvert, que le mode Full plugin
  propose désormais par défaut (le dialogue d'export le rappelle). La
  notification nomme la copie ; Fichier ▸ Ouvrir le dossier des
  sauvegardes de l'export. 9 messages dans les 33 langues.
- Reconnaissance du dictionnaire du mod (B-12, décision D-56) : l'en-tête
  du `.ximdb` reçoit l'identité du plugin pour lequel il est écrit —
  `plugin`, `size`, `crc` (CRC-32, crate crc32fast) et `written` — lue
  dans les premiers octets ; format 4 inchangé, clés facultatives, gardées
  quand l'éditeur de dictionnaire réécrit le fichier. Quand le dictionnaire
  du mod est connu (ouverture, ou choix de la Cible), même taille et même
  CRC → « dictionnaire du mod reconnu » ; différents → message (tailles,
  CRC, date d'écriture, lignes au texte changé) : Continuer, Mise à jour du
  mod…, Ouvrir sans dictionnaire du mod (écarté pour la session, réécrit au
  prochain enregistrement après confirmation) ; une fois par plugin et par
  version du dictionnaire. Plugin renommé : le dictionnaire d'un fichier
  identique est proposé et renommé avec son `.ximhist` et ses copies.
  Option `[Dictionaries] CheckCrc` (Paramètres ▸ Avancé). 17 messages dans
  les 33 langues.
- Propagation entre fichiers et onglet « Tous les mods » (B-25, décision
  D-57) : une traduction écrite dans un fichier ouvert remplit les lignes de
  même source des autres fichiers ouverts (mêmes langues source et cible)
  qui n'en ont pas ou n'ont qu'une traduction « par similarité » — En
  attente, provenance « propagation (<fichier>) », opération annulable de
  chaque fichier ; ni les annulations ni les propagations ne sont
  propagées. Option `[Editing] PropagateAcrossFiles` (Paramètres ▸
  Général) et entrée du menu contextuel « Appliquer aux fichiers
  ouverts ». Dès deux fichiers ouverts, l'onglet « Tous les mods (n) »
  réunit leurs lignes avec la colonne Fichier (filtrable) : saisie,
  statuts et Ctrl+Z vont au fichier d'origine, réglages de la barre en
  lecture seule, actions de dictionnaire refusées (à faire dans l'onglet
  du fichier). Ctrl+Tab / Ctrl+Maj+Tab parcourent les onglets ; vue,
  hauteurs de lignes et filtres gardés par onglet. 15 messages dans les 33
  langues.
- Lignes communes (C-04, décision D-54) : onglet « Communes (n) » de la
  fenêtre d'édition, entre Vocabulaire et Dialogue — les autres lignes de
  même source (sans les espaces de début et de fin) du fichier puis des
  autres fichiers ouverts aux mêmes langues (dans « Tous les mods », les
  autres fichiers après celui de la ligne), avec statut, fichier,
  enregistrement, EDID, traduction (bleue si elle diffère, grise sinon) et
  provenance ; section repliée « Dictionnaires (n) » des traductions
  distinctes de la pile ; un clic montre les différences caractère par
  caractère ; Aller à la ligne / double-clic active l'onglet du fichier et
  y place la fenêtre ; Appliquer à toutes (Ctrl+F6). Au plus 200 lignes et
  20 traductions. Index des sources par fichier, construit à la première
  demande et refait quand les lignes sont réextraites. Le dialogue de
  propagation (A-12) compare désormais les sources sans leurs espaces de
  début et de fin et accueille les lignes des autres fichiers ouverts
  (colonne Fichier, une opération par fichier) ; la propagation entre
  fichiers de B-25 ne reprend plus ce que ce dialogue a décidé. Dialogue,
  Communes et Vocabulaire tiennent lieu des trois onglets de l'exigence.
  18 messages dans les 33 langues.

### Ajouté (étape 2, lot 1)
- Statuts de provenance (B-09) : chaque ligne porte la provenance de sa
  traduction (saisie, dictionnaire avec rôle, fichier et niveau,
  heuristique avec son score, en ligne avec le service, propagation,
  remplacement, fichier cible, importée avec le format), fixée par
  l'opération qui écrit la traduction et elle seule ; changer le statut la
  garde, retoucher la traduction la fait passer en « saisie », la vider
  l'efface ; elle fait partie de l'état annulable (Ctrl+Z / Ctrl+Y).
  Info-bulle « Statut — provenance » de la cellule de statut et onglet
  Enregistrement pour toutes les catégories ; colonne facultative
  Provenance (libellé court, « identique » pour une traduction égale à la
  source) ; liste Provenance : de la barre de filtres (15 choix, remise à
  Toutes par Réinitialiser). Enregistrée dans le dictionnaire du mod
  (`.ximdb` format 4 : huitième élément `{p, r}` par ligne, formats 1 à 3
  lus comme avant), le JSON pivot (champs `prov` et `review`) et le XML
  natif (éléments `<prov>` et `<review>`) ; une ligne retrouvée par le
  dictionnaire du mod reprend sa provenance. La table de provenance de
  session du document disparaît.
- Options ▸ Colonnes (décision D-42) : sous-menu à cases Commentaire et
  Provenance, aussi au clic droit sur l'en-tête de la grille ; clés
  `[Columns] ShowComment` et `ShowProvenance`, l'ancienne
  `[Editing] ShowComment` étant reprise. L'entrée « Afficher la colonne
  Commentaire » disparaît du menu Options.
  Le sous-menu reste ouvert quand la souris passe du menu Options à ses
  cases, comme les sous-menus du menu Dictionnaires (correction du
  23/10/2026 : les cases ne se laissaient pas cocher à la souris).
- 40 messages nouveaux dans les 33 fichiers de langue ; `origin-tip` prend
  la forme `{ $status } — { $provenance }`.
- Colonne Relecture (A-17) : texte d'un relecteur sur chaque ligne,
  colonne facultative (Options ▸ Colonnes ▸ Relecture, `[Columns]
  ShowReview`) entre Traduction et Provenance, éditable comme la traduction
  (bleu quand elle diffère, gris quand elle est égale), champ Relecture
  dans la fenêtre d'édition ; menu de la ligne ▸ Relecture ▸ Traduction =
  Relecture (Ctrl+Maj+R : statut Validé, provenance « relecture »,
  décision D-45), Relecture = Traduction, Effacer la relecture, chacune une
  opération annulable sur la ligne ou la sélection ; Fichier ▸ Charger dans
  la relecture… (XML, JSON, SST, EET, DSD, .ximdb, STRINGS de la langue
  cible ; appariement enregistrement-champ-source ou texte seul ; aperçu
  des lignes appariées, différentes, sans correspondance, déjà relues) ;
  bouton « ≠ Relecture » de la barre de filtres, en attendant la rangée de
  filtres par colonne d'A-10 ; Enregistrer sous… dans un format sans
  relecture le signale. Les exports et les dictionnaires l'ignorent.
  29 messages nouveaux dans les 33 langues.
- Historique horodaté (A-18) : chaque opération annulable laisse une entrée
  par ligne changée (date et heure, identité de la ligne, traduction,
  statut et relecture avant / après, provenance, opération), annulations
  comprises (« Annulation : … ») ; journal en ajout seul écrit à
  l'enregistrement du dictionnaire du mod dans `<dictionnaire>.ximhist`
  (JSON Lines, décision D-46), porté par l'autosauvegarde (une reprise
  retrouve les entrées perdues) et par le JSON pivot (champ `history`) ;
  `[History] Enabled` et `MaxEntries` (20 000, de 1 000 à 200 000 ;
  Paramètres ▸ Avancé). Outils ▸ Historique… (portée ligne, sélection ou
  fichier, filtre par opération, détail, « source différente », Exporter…
  en CSV, `[Window] HistX/Y/W/H`), onglet Historique de la fenêtre
  d'édition, menu de la ligne ▸ Historique ; Revenir à cette version
  (confirmation si la source a changé) et Revenir à la traduction
  précédente (Ctrl+Alt+Z), en une opération annulable « Retour arrière
  (historique) », provenance « historique ». Le pré-remplissage de
  l'ouverture n'est pas inscrit. 41 messages nouveaux dans les 33 langues.
- Compteurs par statut (A-15, décision D-50) : une pastille par statut dans
  la barre d'état (glyphe et nombre dans la couleur du statut, atténuée à
  zéro), comptant tout le document (les lignes jamais affichées en jeu
  seulement quand elles sont montrées), recalculée avec la vue ;
  info-bulle « Statut : N lignes, dont M affichées » ; clic = ce seul
  statut (second clic : filtre relâché), Ctrl+clic = ajout au filtre, le
  même que les boutons de la barre de filtres ; sans glyphe en fenêtre
  étroite, repliées en « Statuts ▾ » en dessous de 520 px. « Total N ·
  M affichées (U uniques, R à traduire) » remplace les anciens compteurs ;
  la mention « traduites » disparaît. 5 messages dans les 33 langues ; les
  clés `counter-shown`, `counter-unique`, `counter-remaining` et
  `counter-translated` sont retirées.
- Boutons de statut par ligne (A-04, décision D-48) : colonne facultative
  Options ▸ Colonnes ▸ Boutons de statut (`[Columns] ShowStatusButtons`,
  0 par défaut), à droite de St : quatre petits boutons dans la couleur
  de leur statut (✓ Validé, ⋯ En attente, ★ Personnalisé, ⊘ Ignoré) ; un
  clic donne le statut à la ligne, ou à toute la sélection si la ligne
  en fait partie, par le même chemin que F5 à F8 (opération annulable,
  compte dans la barre d'état, proposition de propagation après ✓) ; une
  ligne non sélectionnée est d'abord sélectionnée seule ; bouton du
  statut courant encadré, statuts impossibles grisés avec leur raison en
  info-bulle (« Une ligne sans traduction ne peut pas être Validé. »),
  pas de boutons sur les lignes jamais affichées en jeu ; info-bulles
  « Valider — F5 »… 6 messages dans les 33 langues.
- Filtres par colonne (A-10, décision D-47) : le bouton « Filtres par
  colonne » de la barre de filtres (Ctrl+Maj+F, `[Editing] ColumnFilters`,
  0 par défaut) affiche une rangée de champs sous les en-têtes de la
  grille — GRUP, Champ, EDID, Source, Traduction et les colonnes
  facultatives Relecture, Provenance, Commentaire ; rien sous St ni sous
  les boutons de statut —, combinés en ET entre eux et avec la barre ;
  « contient » sans la casse, ou expression régulière avec la case `.*`
  de la barre (un seul mode pour la rangée) ; expression invalide en
  rouge avec l'erreur en info-bulle, ce seul filtre ignoré ; EDID cherche
  aussi le FormID, Provenance le libellé court, Commentaire le texte de
  la colonne ; 250 ms après la frappe ou Entrée, Échap vide le champ, la
  gomme ⌫ vide la rangée, Réinitialiser aussi ; masquer la rangée vide
  ses champs. Grille vide et compteur « affichées » : « filtres : GRUP
  « ARMO », Champ « FULL » ». Le bouton « ≠ Relecture » de la barre
  passe dans la cellule Relecture de la rangée (bouton « ≠ ») ; la clé
  `filter-review-diff` est retirée. 4 messages dans les 33 langues.
- Ambiguïtés (B-08, décision D-44) : à l'application des dictionnaires
  (ouverture, Appliquer les dictionnaires…), une ligne remplie au niveau
  « même texte » ou « texte seul » dont la source a au moins deux
  traductions distinctes dans les dictionnaires actifs — plusieurs dans
  un même dictionnaire comprises ; les niveaux qui identifient
  l'enregistrement et le dictionnaire personnel ne sont jamais ambigus —
  reçoit la candidate du dictionnaire prioritaire au statut En attente,
  avec la marque ≠ dans la cellule de statut et la provenance
  « … · ambiguë (n) » (suffixe `|amb:n` du code, enregistré avec la
  provenance, restauré à la réouverture) ; le message de fin compte les
  lignes ambiguës ; bouton Ambiguës de la barre de filtres ; onglet
  Vocabulaire : section « Traductions candidates (n) » avec dictionnaires,
  nombre d'occurrences et Utiliser (En attente, provenance du
  dictionnaire choisi) ; la marque disparaît à la saisie, à Utiliser, à
  Validé ou Personnalisé, Ctrl+Z la ramène. L'index d'un dictionnaire
  garde toutes les traductions distinctes d'une source (les doublons
  d'une même clé compris), le mode Texte seul en profite. 6 messages
  dans les 33 langues.
- Diff caractère par caractère (A-08, décision D-49) : dans l'onglet
  Vocabulaire, le cadre « Différences caractère par caractère » compare la
  source de la ligne à celle d'une suggestion (la première, puis celle que
  l'on clique, sa ligne encadrée) ou la traduction de la ligne à une
  candidate (B-08) : vert ce que la ligne a en plus, rouge ce que l'entrée
  a en plus, espace « · », saut de ligne « ↵ », tabulation « → » dans les
  écarts, bouton ⇄ Inverser (lignes, couleurs et légende), lisible en thème
  clair et sombre. Module `diff` : Myers sur les graphèmes (bibliothèque
  `similar` 3.2, fonctionnalité `unicode`) puis nettoyage sémantique — un
  commun de moins de deux caractères entre deux écarts est absorbé, un
  écart dans un mot ou un nombre s'étend au mot ou au nombre entier — ;
  au-delà de 5 000 caractères ou de 50 ms, comparaison mot par mot et le
  cadre le dit. La fenêtre Mise à jour du mod surligne de même l'ancienne
  et la nouvelle source des lignes modifiées. 9 messages dans les 33
  langues.
- Grille : la hauteur des lignes tient compte de la marge des zones de
  saisie (une traduction qui passait à la ligne était coupée).

### Corrigé (étape 2, lot 1)
- Barres d'outils et de filtres : les rangées se chevauchaient selon la
  largeur de la fenêtre (signalé les 25 et 26/10/2026). Deux causes : un
  séparateur vertical prenait toute la hauteur restante du panneau (il a
  maintenant la hauteur d'un contrôle) ; et un libellé placé en fin de
  rangée, juste après un séparateur, faisait couler son texte sur la
  rangée suivante, qu'egui plaçait alors sans l'écart entre rangées. Les
  libellés ne coulent plus (ils passent entiers à la rangée suivante), un
  libellé et sa liste passent ensemble à la ligne (les listes d'egui ne
  passent jamais à la ligne d'elles-mêmes et débordaient du bord droit),
  et aucun séparateur ne commence une rangée. Toutes les rangées font
  30 px, avec ou sans retour à la ligne, vérifié de 900 à 1 400 px.
- Fenêtre « Choisir un pays » : le curseur est dans le champ Filtre à
  l'ouverture (demande du 25/10/2026) ; on tape le pays et Entrée prend le
  premier drapeau.
- Fenêtre d'édition : les onglets du bas reçoivent 40 % de la hauteur
  supplémentaire d'une fenêtre agrandie (ils restaient à 140 px, les
  candidates et les différences n'avaient pas de place).
- Fenêtre « Choisir un pays » : Entrée sur le drapeau en surbrillance
  choisissait toujours le pays de la langue d'interface, même ouverte
  depuis le bouton Cible ou l'éditeur de traduction de l'interface ; elle
  suit maintenant l'usage pour lequel elle a été ouverte, comme le clic.
- Onglet Vocabulaire : les candidates, les suggestions et les fragments
  défilent ensemble, la barre de défilement toujours visible, au lieu de
  recouvrir les boutons de statut dans une fenêtre basse.
- Les boutons de statut de la barre de filtres (« Statuts : ») étaient
  presque invisibles : un glyphe pastel seul sur fond clair ou sur la
  sélection bleue (signalé le 25/10/2026). Chaque statut est maintenant un
  bouton au fond de sa couleur et au glyphe sombre, comme la colonne St et
  les boutons de statut par ligne, encadré tant que le statut est affiché,
  en gris quand il est masqué.
- Échap dans le champ Filtre de la barre ne le vidait pas (egui retire le
  focus sur Échap avant le dessin de l'image) : le champ est vidé quand il
  perd le focus sur Échap, dans la barre comme dans la rangée des filtres
  par colonne.
- Lenteur sur un gros fichier (signalée le 24/10/2026 : 10 à 15 s pour
  réagir à un menu ou à la fermeture) : la grille recalculait à chaque
  image la hauteur de toutes les lignes visibles en les mettant en page une
  à une (50 ms par image pour 50 000 lignes, 800 ms à chaque changement de
  largeur des colonnes). Les hauteurs sont maintenant conservées d'une image
  à l'autre et recalculées seulement quand le document, la vue, les
  largeurs ou la police changent (seules les lignes dont le texte a changé
  le sont après une saisie), par un retour à la ligne sur les avances des
  glyphes sans mise en page (43 ms pour les 50 000 lignes). La barre de
  filtres ne parcourait plus le document à chaque image pour ses listes
  GRUP et Champ. Diagnostic : `[Diagnostics] FrameLog=<ms>` dans
  Config.ini (ou `XIMOD_FRAME_LOG=<ms>` dans l'environnement) inscrit au
  journal les images et les sections plus lentes que ce seuil.
- Le journal de l'application est écrit dans `logs/ximod.log` du dossier
  de configuration (réécrit à chaque démarrage), en plus de la console que
  la version Windows n'a pas.
- L'application redessinait sa fenêtre sans arrêt, même au repos (80 images
  par seconde, un cœur occupé en permanence et le gestionnaire de fenêtres
  avec lui) : le titre de la fenêtre était envoyé à chaque image, et un
  ordre de fenêtre demande à egui une image de plus aussitôt. Le titre
  n'est envoyé que lorsqu'il change ; au repos, plus aucune image n'est
  dessinée. Ce défaut datait de la première version.
- Le presse-papiers n'est plus lu par le fil de l'interface : la lecture
  (pour les entrées Coller du menu de la ligne) se fait sur un fil à part,
  au plus une fois par seconde tant qu'un menu est ouvert ; un propriétaire
  du presse-papiers lent ne peut plus figer l'application.
- Diagnostic : avec `FrameLog`, toutes les fenêtres et les opérations qui
  sortent du processus (Config.ini, presse-papiers, dates des fichiers,
  dictionnaire du mod, historique, dialogues d'ouverture et d'enregistrement)
  sont chronométrées une à une (« slow io … », « slow section … »).

### Modifié (étape 2, lot 1)
- Barre d'outils 2 px moins haute (30 px) ; les deux rangées de la barre
  de filtres passent à la même hauteur qu'elle (demande du 25/10/2026).
- Nouvelle dépendance `similar` 3.2.1 (MIT / Apache-2.0, sans dépendance
  hors `unicode-segmentation`) pour les différences caractère par
  caractère (A-08, décision D-49).
- Dépendances mises à jour (`cargo update`, compatibles Rust 1.85) :
  bcdec_rs 0.2.1, clipboard-win 5.4.3, memmap2 0.9.12, smallvec 1.16.4,
  tinystr 0.8.5, tokio 1.54.1, webbrowser 1.2.5, webpki-roots 1.0.11,
  yoke 0.8.4, zerovec 0.11.9, zerovec-derive 0.11.7 ; cc reste épinglé
  à 1.8.0.
- Version minimale de Rust portée de 1.85 à 1.97 (`rust-version` des six
  paquets de l'espace de travail ; décision de Patrice du 23/10/2026). Le
  résolveur de Cargo, qui retenait les versions compatibles 1.85, prend
  30 mises à jour de plus : calloop 0.14.5, encoding_rs 0.8.42, icu_* 2.4.0,
  idna_adapter 1.2.2, image 0.25.10, moxcms 0.8.1, num-conv 0.2.2,
  time 0.3.55, uuid 1.29.0, wayland-protocols 0.32.13, zbus 5.19.0,
  zvariant 5.15.1, zopfli 0.8.4… ; les changements de version majeure
  (egui 0.32, ureq 3, zip 7, keyring 4…) demandent une adaptation du code
  et restent à décider. Les `if` imbriqués sont réécrits en chaînes
  `if let … && …` (édition 2024, `clippy --fix` : 200 réécritures environ,
  sans changement de comportement), trois `% n == 0` deviennent
  `is_multiple_of`.

## [0.2.0] - 2026-10-23

### Ajouté (étape 1, lot 5)
- Traduction en ligne (D-01) : services DeepL (gratuit et pro), Azure
  Translator, OpenAI et points d'accès compatibles (LM Studio, Ollama,
  vLLM…), Google Cloud Translation, derrière un même connecteur (client HTTP
  `ureq` avec rustls, fil d'arrière-plan) ; Paramètres ▸ En ligne (services,
  clés masquées, points d'accès, formalité, région, modèle, Tester…, service
  par défaut) ; Ctrl+G, menu de la ligne ▸ Traduire en ligne et bouton En
  ligne de la fenêtre d'édition pour la ligne courante ou la sélection ;
  Ctrl+Maj+G, Outils ▸ Traduire en ligne… pour un lot (portée, lignes à
  traduire seulement, estimation lignes / sources distinctes / caractères /
  requêtes, progression, Arrêter, journal des lignes reçues ou refusées) ;
  lignes reçues En attente avec la provenance « en ligne (service) »
  (décision D-32), en une opération annulable ; confirmation avant de
  remplacer une ligne validée ou personnalisée ; cache des réponses
  `cache/online/<service>_<SRC>-<DST>.ximdb` (décision D-38) ; sans service,
  Ctrl+G ouvre le navigateur comme avant (décision D-36) ; la fenêtre
  Traduire un FOMOD utilise le service quand il existe.
- Normalisation, protection et quotas (D-02) : alias, balises et gabarits
  protégés par jetons `<a_A>`…, nombres `<n_A>`… (décision D-37), sauts de
  ligne `<br_A>`…, transmis comme balises à ne pas traduire (DeepL
  tag_handling, Azure et Google html / notranslate, consigne OpenAI) et
  restaurés au retour ; réponse refusée (ligne non modifiée) quand un jeton
  manque, est en double ou altéré, quand elle est vide ou identique à la
  source ; quotas réglables (caractères par requête, lignes par lot,
  requêtes par minute) tenus par le fil d'arrière-plan, ligne trop longue
  refusée, seuil de caractères par session avec confirmation ; proxy
  (Paramètres ou HTTPS_PROXY) et délai d'attente ; clés dans le trousseau du
  système (`keyring` : Credential Manager, Keychain, Secret Service) avec
  repli `secrets.json` aux droits de l'utilisateur (décision D-28), jamais
  dans Config.ini ni dans le journal ; `[Online]` dans Config.ini.
- Langues d'interface (E-07) : 33 langues (ara bul ces dan deu ell eng est fin
  fra gle hrv hun ita jpn kor lav lit mlt nld nob nor pol por roh ron rus slk
  slv spa swe tur zho), un `assets/locales/<code>/main.ftl` complet par langue
  avec l'en-tête de métadonnées d'Architect (`# @language`, `# @font`,
  `# @langname`, `# @author`) ; 142 messages communs repris d'Architect, les
  autres (environ 1 180 par langue) traduits automatiquement et signalés
  « traduction automatique, à relire » dans l'en-tête (décision D-31), à
  relire par l'éditeur E-10 ; `nor` reprend `nob`. Repli message par message
  sur l'anglais, nombre de messages manquants journalisé au niveau debug.
  Paramètres ▸ Général ▸ Langue de l'interface : la liste n'est plus
  désactivée sans pays ; elle offre les langues du pays qui ont une interface
  puis toutes les langues de l'interface par endonyme. Changement de langue
  sans redémarrage, commentaires générés compris. `assets/data/
  CountryLanguages.json` (langues parlées par pays, repris d'Architect) est
  désormais livré : la liste des langues du pays était vide sans lui.
  Messages encore écrits en dur passés en clés Fluent (modes de sortie,
  barre d'état à l'ouverture, au rechargement et à l'enregistrement, erreurs,
  unité d'octets du compteur de longueur, aide du filtre). Manuel en anglais
  `Manuals/XIMOD_Translator_Manual_ENG.docx`, généré de la même façon que le
  français ; Aide ▸ Manuel cherche `…_<CODE ISO 639-3>` (FRA, ENG…), puis
  l'anglais (l'ancien nom `_GBR` reste reconnu). Polices attendues dans
  `assets/fonts/` : Noto_Sans, Noto_Sans_Arabic, Noto_Sans_JP, Noto_Sans_KR,
  Noto_Sans_SC (chemins de `Languages.json`) ; l'arabe s'affiche de gauche à
  droite jusqu'à E-08.
- Éditeur de traduction de l'interface (E-10), Outils ▸ Traduire
  l'interface…, fenêtre indépendante (décision D-33, `[Window] TrWin*`), port
  de `ui/translation.rs` d'Architect : drapeau → pays → langue à traduire
  (code ISO 639-3 obligatoire), langue affichée (n'importe quelle locale,
  saisies conservées au changement), endonymes du pays et de la langue,
  auteur, police (Parcourir… limité à assets/fonts, Google Fonts…) appliquée
  à l'aperçu ; table Clé / Libellé / Traduction en lignes de hauteur fixe,
  recherche, « Seulement les clés non traduites », progression ; variables et
  macros masquées en bordure (séparateurs de la traduction conservés) ou en
  marqueurs `[[n]]` au milieu, marqueur perdu ou en double en rouge et repli
  sur le libellé ; clés non traduisibles (`app-title`, `# @untranslatable`)
  et clés faites de seules variables cachées, messages à pluriel en lecture
  seule ; clavier (flèches, Page, Début / Fin, Entrée, Échap, Tab / Maj+Tab,
  Ctrl+S, Ctrl+F). Enregistrement dans l'ordre et avec les lignes vides de
  la référence anglaise (commentaires de la langue quand son fichier la
  suit), en-tête `# @language / @font / @langname / @author`, clé vide →
  libellé ; Languages.json et Countries.json modifiés seulement si un
  endonyme ou la police l'a été, `countryEndonyms.tsv` à côté du fichier ;
  `<config>/locales/<code>/` quand assets/ n'est pas inscriptible, lu en
  priorité par le chargeur (décision D-35) ; rechargement à chaud de la
  langue courante. Envoyer… : `ximod-translator_<code>.zip` (main.ftl +
  countryEndonyms.tsv, aucun fichier ISO) dans le dossier de configuration et
  courriel prérempli. Les 33 locales sont normalisées sur la structure de la
  référence (enregistrer sans rien changer réécrit le même fichier, test
  automatisé).
- Aucun texte dans le code : tous les textes visibles sont dans les
  `main.ftl` (145 messages ajoutés et 8 reformulés, traduits dans les 33 langues).
  Messages d'erreur des lecteurs (plugins, STRINGS, SST, EET, XML, BSA/BA2,
  locres, bases XIMOD, empaquetage, services en ligne…) transmis sous forme
  de clé + arguments (`l10n!`, module `l10n`) et traduits à l'affichage
  (`I18n::localize`, barre d'état, notifications, alertes) ; assemblages de
  messages par des modèles (`join-dash`, `join-colon`, `join-paren`,
  `join-dot`, `join-comma`) au lieu de ponctuation en dur ; unités de taille,
  format des dates, noms des touches de modification (Ctrl+Maj+E en
  français), noms des thèmes, des services en ligne, des sources de
  détection des jeux et des rôles de dictionnaire, abréviations des jeux,
  guillemets, commentaire « Gabarit d'objet » (non écrit dans les fichiers
  EET / XML). Test `tests/no_hardcoded_text.rs` qui échoue si un texte est
  de nouveau écrit dans le code. Restent en dur, volontairement : le journal
  de diagnostic (anglais), le contenu des formats de fichiers (balises XML,
  clés de Config.ini, en-tête des .ftl) et les identifiants techniques.
- Dépendances mises à jour (`cargo update`, compatibles Rust 1.85) :
  arboard 3.7.0, cc 1.9.0, rustls 0.23.46, rustls-webpki 0.103.16,
  serde_json 1.0.154, syn 3.0.9, tokio 1.54.0, webpki-roots 1.0.10,
  zip 2.6.1 et onze dépendances indirectes. Deux `if … && let` de la
  fenêtre FOMOD réécrits sans « let chains » (Rust 1.88), pour respecter
  `rust-version = "1.85"`.
- Build Windows réparé : cc 1.9.0 faisait échouer le build de directxtex
  (lib.exe, LNK1181 sur `Ole32.lib`). cc épinglé à 1.8.0 dans
  `Cargo.toml` (`=1.8.0`) pour que `cargo update` ne le remonte pas.
- crc32fast 1.5.2 → 1.5.3 (`cargo update`, compatible Rust 1.85).
- Modes de sortie : STRINGS, Hybrid, Override patch, Full plugin et
  DSD (SKSE) sont des termes techniques et restent en anglais dans les
  33 langues (français compris). Le conseil affiché avant l'écriture d'un
  plugin complet et le titre de la fenêtre d'export en mode override
  citent désormais « Override patch » sous ce même nom.
- Relecture des traductions automatiques (classeur « Traductions à
  vérifier », propositions DeepL et, pour le romanche, ChatGPT) : 37 lignes
  modifiées dans 23 langues — en-tête « St. » abrégé avec un point, endonymes
  plus courts (fin, hun, rus, tur), « Issejvja » pour Enregistrer en maltais,
  « B » pour l'octet (ara, fin), colonne « Tekst / Teksti » (dan, fin),
  « Tscherner… » (roh), « Författare » (swe).
- Arabe mis en attente jusqu'à l'affichage de droite à gauche (E-08,
  étape 3) : `assets/locales/ara/main.ftl` reste livré et modifiable dans
  l'éditeur de traduction, mais l'arabe n'est plus proposé comme langue de
  l'interface ni comme langue affichée de l'éditeur ; un Config.ini qui le
  désigne ouvre l'interface en anglais (`i18n::RTL_PENDING`). 32 langues
  proposées.

### Ajouté (étape 1, lot 4)
- Vue des dialogues (C-01) : passe de contexte du plugin en arrière-plan
  (quêtes et alias, sujets, répliques et réponses, scènes, PNJ, types de
  voix, factions, mots-clés…), index des noms des fichiers maîtres construit
  une fois et conservé dans `cache/names/` (décision D-19) ; fenêtre
  Outils ▸ Dialogues… (F10) — arbre quête → scènes et phases / catégories
  et sous-types → sujets → répliques avec pastille de statut et compteurs,
  détail de la réplique (locuteur, question du joueur, réponses avec
  émotion et note, conditions, voix), navigation dans la conversation
  (Ctrl+Alt+Gauche / Droite, PNAM, TCLT, scènes), synchronisation avec la
  grille dans les deux sens, filtres quête / PNJ / texte ; onglet Dialogue
  de la fenêtre d'édition ; données par famille de jeux
  `assets/data/dialogue/*.json` ; `[Window] DialogX/Y/W/H`,
  `[Dialogues] FollowGrid / Split`.
- Locuteurs et conditions en clair (C-02) : locuteur résolu par ANAM, par
  l'acteur de la scène (alias → acteur unique ou référence forcée) ou par
  les conditions (GetIsID, GetIsAliasRef, GetIsVoiceType, GetInFaction,
  GetIsRace), genre et type de voix ; conditions CTDA décodées par les
  tables `assets/data/conditions/*.json` (Skyrim et Fallout 4 complètes,
  Starfield d'après xEdit, Oblivion / Fallout 3 fonctions communes) et
  dites en clair pour une soixantaine de fonctions, forme brute au survol ;
  liste PNJ de la fenêtre, « Filtrer la grille sur ce PNJ », menu de la
  ligne ▸ Filtrer sur ce PNJ, pastille « PNJ : … » dans la barre de filtre.
- Colonne Commentaire (C-03) : septième colonne de la grille, masquée au
  départ (Options ▸ Afficher la colonne Commentaire, `[Editing]
  ShowComment`, décision D-22), générée à l'affichage et mémorisée par
  ligne — réplique « locuteur (genre) · quête · sujet · émotion · réponse
  n / N », sujet « quête · sujet · N répliques », quête « entrée de
  journal, étape N » / « objectif N », message / atout / sort « quête … »,
  objet « modèle : chemin », PNJ « homme / femme · race · voix », surcharge
  « maître : fichier » ; le commentaire de l'auteur (EET, XML) vient en
  tête ; passe de contexte étendue (MODL, QNAM, étape de chaque entrée de
  journal, CONT / FLOR / LIGH / DOOR / NOTE / TERM / SLGM / APPA…) ; cible
  « Dans : Commentaire » de la barre de filtre (regex et casse comme les
  autres, auteur + généré) ; ligne « Commentaire : … » de l'onglet
  Enregistrement ; Options ▸ Exporter aussi les commentaires générés
  (EET, XML) (`[Export] GeneratedComments`, décochée, décision D-23) pour
  Enregistrer sous… en EET, EET XML et XIMOD XML.
- Fichiers d'interface et formats texte (C-05) : les fichiers MCM
  (`Interface/Translations/<mod>_<langue>.txt`, `translate_<langue>.txt`),
  INI, JSON plat et CSV s'ouvrent comme des documents (moteur « texte » :
  colonnes Format / Champ / Clé, lignes étrangères conservées à leur place,
  badge « MCM · UTF-16 · ENG→FRA », barre d'état avec le fichier cible) ;
  formats décrits par expression régulière dans `data/text_formats.json`
  (groupes key / value, encodage auto / UTF-8 / UTF-16 / Windows-1252,
  fins de ligne, échappement json / c, motifs de nom), fournis en lecture
  seule et édités dans Outils ▸ Données ▸ Formats texte… (Nouveau,
  Dupliquer, Supprimer, Essai sur un fichier, refus d'une expression
  invalide ou sans groupes) ; boîte « Quel format ? » quand le nom ne
  décide pas ; langues des fichiers frères dans la liste Source (noms
  Skyrim, codes Fallout 4 / Starfield), fichier cible existant repris au
  statut Validé (provenance « fichier cible ») ; Sortie « Fichier texte »,
  Enregistrer sous… et Exporter la traduction… écrivent
  `<mod>_<langue cible>.txt` dans l'encodage et les fins de ligne du fichier,
  valeurs remplacées en place, case « Lignes non traduites : reprendre la
  source » (`[Export] UntranslatedCopy`, décision D-25) ; gabarits « {0} »
  / « %s » comptés comme balises (`fields/Text.json`) ; Fichier ▸ Ouvrir les
  fichiers d'interface du mod et signalement à l'ouverture d'un plugin
  (décision D-24) ; dictionnaire du mod du fichier texte dans la pile du
  plugin (rôle Externes) et réciproquement.
- Carte des voix (C-13) : à la première demande, les fichiers voix du mod
  sont listés sans extraction — fichiers libres sous
  `<Data>/Sound/Voice/<plugin>/` et archives du dossier du plugin nommées
  d'après lui (répertoire seul, par mmap ; les archives du jeu ne sont pas
  lues, décision D-27) — et associés aux réponses par le suffixe
  `_<FormID>_<n>` (24 bits bas, index de charge de l'auteur indifférent),
  type de voix ou race / sexe d'après les dossiers, `.lip` seul = lèvres
  seulement, Starfield non résolu (décision D-26) ; fenêtre Dialogues :
  ligne par réponse (✔ fichier, type et source libre / archive ; ✘ en
  rouge), info-bulle des chemins, « Ouvrir le dossier », compteurs de
  répliques non doublées, filtre « Non doublées », « Rafraîchir »,
  « Copier la liste des non doublées » (FormID ⇥ sujet ⇥ locuteur ⇥
  texte) ; onglet Dialogue et colonne Commentaire (« voix ✔ / ✘ ») ; un mod
  sans source ne marque rien ; données `dialogue.voice` par famille.

### Ajouté (étape 1, lot 3)
- Arbre des dictionnaires (B-02) : Dictionnaires ▸ Gérer les dictionnaires…
  — six rôles (Mod, Personnel, Texte seul, Jeu, Externes, Sauvegardes), cases
  et ordre de priorité, fichiers et dossiers ajoutés hors des dossiers XIMOD,
  mémorisés par jeu et paire de langues (`[Dictionaries]`) ; chaque
  dictionnaire est indexé séparément et relu s'il change ; la barre d'état
  d'ouverture énumère les dictionnaires qui ont rempli des lignes ; chaque
  ligne pré-remplie porte sa provenance (info-bulle de la cellule de statut,
  onglet Enregistrement de la fenêtre d'édition).
- Bases texte seul (B-05) : format .ximdb v3 — des lignes sans
  enregistrement ni champ, paires texte → texte ; rôle « Texte seul » de
  l'arbre, appariement par le texte quel que soit le champ (Traduit par
  similarité, provenance « texte seul ») ; Dictionnaires ▸ Créer un
  dictionnaire texte seul à partir d'un fichier de paires… (texte tabulé).
- Dictionnaire personnel éditable (B-04) : Ctrl+M / menu de la ligne ▸
  Ajouter à mon dictionnaire personnel (grille et fenêtre d'édition), en
  texte seul par défaut ou avec le champ (`[Personal] KeepField`),
  confirmation au-delà de vingt lignes, Remplacer / Conserver sur une paire
  déjà connue avec choix mémorisé pour la session ; éditeur de dictionnaire
  (Dictionnaires ▸ Mon dictionnaire personnel… / Modifier un dictionnaire…,
  bouton Modifier… de l'arbre) : filtre, table triable, Ajouter / Remplacer /
  Supprimer, Nouveau…, Enregistrer avec copie horodatée, lecture seule sur
  les dictionnaires fournis ; option « Alimenter mon dictionnaire personnel
  à chaque enregistrement » (`[Personal] AutoFill`) ; le dictionnaire
  personnel donne le statut Personnalisé sur correspondance exacte.
- Appliquer les dictionnaires (B-06) : dialogue Options ▸ Appliquer les
  dictionnaires… et menu de la ligne ▸ Appliquer les dictionnaires à la
  sélection… — la pile active ou un fichier, portée (document, vue,
  sélection), règle d'écrasement par statut, appariement Strict / Relax /
  Texte seul / FormID seul (texte changé → En attente), réinitialisation
  préalable, aperçu à la frappe (arrière-plan au-delà de 10 000 lignes),
  une opération annulable, provenance posée sur chaque ligne.
- Traductions obsolètes (B-07) : section « anciennes données » du
  dictionnaire du mod (clé `o` du format v3, lignes datées, uniques,
  purge par ancienneté `[Backup] OldDataMonths`), alimentée à chaque
  écriture par les lignes que le document n'a plus et vidée de celles qui
  reviennent ; les anciennes données servent après les lignes courantes
  (exact avec statut, texte par similarité, provenance « ancienne
  donnée ») et nourrissent le mode FormID seul ; fenêtre Dictionnaires ▸
  Mise à jour du mod… (ajoutées, supprimées, modifiées, inchangées ;
  Reprendre les anciennes traductions en En attente, une opération ;
  Oublier les supprimées) ; barre d'état à l'ouverture quand des lignes
  ont changé ; l'éditeur montre, supprime ou remet les anciennes lignes.
- Heuristique par mots (B-01) : onglet Vocabulaire de la fenêtre
  d'édition — les sources proches de la ligne dans tous les dictionnaires
  actifs (mots différents admis jusqu'au seuil `[Fuzzy] Threshold`, 20 %
  par défaut, Paramètres ▸ Avancé), avec score coloré, mots différents
  soulignés, traduction, dictionnaire et champ d'origine, anciennes données
  comprises ; Utiliser copie la traduction en En attente (annulable) ; index
  des mots construit en arrière-plan à la première demande et conservé pour
  la session. Mode Approchant du dialogue Appliquer les dictionnaires
  (meilleure source proche de toute la pile, Traduit par similarité,
  provenance « approchant (N %) », répartition par score dans l'aperçu).
- Traduction instantanée (B-14) : dans la fenêtre d'édition, F9 traduit la
  sélection de la zone Source (désormais sélectionnable, sélection
  conservée) ou toute la source en assemblant les fragments connus des
  dictionnaires actifs, texte seul et personnel en tête — texte entier
  d'abord, puis le plus long fragment à chaque mot, mots inconnus recopiés
  et comptés, balises, nombres et ponctuation recopiés, majuscule initiale
  selon la destination ; insertion au curseur de la Traduction ou
  remplacement après confirmation (« Ne plus demander pendant cette
  session ») ; En attente, annulable, bilan dans la ligne de message.
  Ctrl+F9 traduit en deux temps au curseur de la source. L'onglet
  Vocabulaire liste les fragments connus de la ligne et rappelle les
  raccourcis.
- Propagation aux lignes de même source (A-12, anticipé de l'étape 2) :
  Ctrl+F6 / menu de la ligne ▸ Appliquer à toutes les lignes de même
  source… (grille et fenêtre d'édition) ouvre un dialogue listant les
  autres lignes de même texte source (hors Ignoré et jamais affichées)
  avec enregistrement, champ, EDID, traduction actuelle et statut, cochées
  sauf les Validé / Personnalisé qui diffèrent ; Appliquer copie la
  traduction et le statut en une opération annulable ; plusieurs lignes
  sélectionnées forment un groupe par source. Option « Proposer la
  propagation aux lignes de même source après validation » (`[Editing]
  ProposePropagation`, Paramètres ▸ Général) : le dialogue après F5 ou
  Ctrl+↓ sur une ligne qui a des doublons différents.
- Dictionnaire du mod, mémoire de travail (B-03, décision D-02) : écrit
  automatiquement à chaque Enregistrer sous… et export réussi (option
  `[Backup] ModDictAuto`, Paramètres ▸ Avancé) et par Dictionnaires ▸
  Enregistrer le dictionnaire du mod, avec toutes les lignes traduites ou
  ignorées et leur statut, copies horodatées ; à la réouverture, appliqué en
  premier, chaque ligne reconnue exactement retrouve son statut (Validé, En
  attente même sans texte, Personnalisé, Ignoré…). Les bases partagées ne
  portent pas de statut et restent appliquées en Validé.

### Ajouté (étape 1, lots 1 et 2)
- Crate `ximod-games` et trait `Engine` : fiches de jeu (`assets/data/games`)
  et tables de champs traduisibles (`assets/data/fields`) en données, avec
  éditeur Outils ▸ Données ▸ Champs traduisibles par jeu… (S-05, A-06).
- Mode portable / par utilisateur pour la configuration, Paramètres ▸ Avancé
  (E-05).
- Dialogue « Lignes exportées » : seules les lignes Validé sont exportées par
  défaut, les autres statuts sur demande ; dictionnaire de mod `.ximdb` v2
  avec statut (S-04).
- Réglages Jeu / Source / Cible / Sortie par document, badges d'onglets,
  cache d'index et pile de dictionnaires mod → personnel → jeu (E-12).
- Annulation / rétablissement (Ctrl+Z / Ctrl+Y), marque « modifié » (A-01).
- Copies horodatées des dictionnaires de mod, autosauvegarde et reprise après
  plantage (B-11).
- Patch d'override délocalisé pour les plugins localisés (Skyrim SE,
  Fallout 4, Starfield), option ESL, noms de STRINGS Starfield (S-01).
- Détection des jeux via Heroic (GOG / Epic / Amazon) et Lutris (S-08).
- Ouverture des fichiers passés en argument ; vérification continue et
  livraison automatisées (S-02).
- Recette du lot 1 : les quatre listes de la barre d'outils sont vides au
  lancement et après la fermeture du dernier onglet ; la liste Source n'offre
  que les langues que le fichier fournit (STRINGS trouvés, langue déclarée,
  sinon détection dans le texte par Lingua, 75 langues) ; Cible devient un
  bouton qui ouvre la fenêtre des drapeaux et remplit la liste des langues
  officielles du pays choisi ; la Sortie est réclamée (vide) ou confirmée à
  l'export ; option de ligne de commande `--game <id>`.
- Lignes « jamais affichées en jeu » (mode de champ `unused`) : en-tête du
  plugin et noms réservés à l'éditeur, en rouge, non traduisibles, masquées
  par défaut (case « Jamais affichées en jeu » de la barre de filtres).
- Sélection de lignes dans la grille (clic, Maj+clic, Ctrl+clic) ; statuts au
  clavier F5–F8 sur la sélection ; Ctrl+↓ / Ctrl+↑ « valider puis suivante /
  précédente », le curseur de saisie suivant (A-03).
- Rechercher et remplacer (A-11) : fenêtre indépendante (Ctrl+H, Outils),
  cible, portée (document, vue, sélection), casse, mot entier, modes normal /
  étendu / regex avec groupes, aperçu à la frappe, F3 / Maj+F3, Remplacer
  et Tout remplacer en une opération annulable, Ignoré jamais touché, Validé
  passé En attente, fichiers de règles JSON (appliquer, enregistrer), dix
  derniers motifs ; Options ▸ Afficher les caractères spéciaux (¶ ↵ → °)
  dans la grille et la fenêtre d'édition sans toucher au texte.
- Vérifications qualité (D-05) : fenêtre Outils ▸ Vérifications qualité…
  (F12 ouvre et vérifie), neuf vérifications cochables mémorisées
  (`[Quality] Checks`) — alias, nombres, longueur, retour à la ligne,
  ponctuation finale, majuscule après un point, espaces, caractères hors
  Windows-1252, traductions différentes d'une même source —, portée
  document / vue / sélection, calcul en arrière-plan avec progression,
  tableau triable (ligne, EDID, vérification, détail), double-clic ou Entrée
  vers la fenêtre d'édition, Réparer les alias / les nombres sur la
  sélection, « Unifier sur la plus fréquente » en une opération annulable ;
  indicateur « [!] » dans la cellule de statut avec info-bulle, bouton
  « [!] Anomalies » de la barre de filtres ; l'indicateur et le tableau
  suivent les corrections ligne par ligne.
- Recherche étendue (A-09) : liste « Dans » (Tout, Source, Traduction,
  EDID ou FormID — hexadécimal de 6 à 8 chiffres comparé au FormID, avec ou
  sans index de master —, Signature), case « .* » pour une expression
  régulière (insensible à la casse, `(?-i)` pour la respecter), application
  250 ms après la dernière frappe ou sur Entrée, expression invalide
  signalée en rouge sans vider la grille, Ctrl+F et Échap sur le champ ;
  la vue filtrée n'est recalculée que lorsque le filtre ou le document
  change (50 000 lignes : quelques millisecondes).
- Dépendances mises à jour (cargo update du 09/10/2026 : arbitrary, either,
  objc2, toml, windows-core, zerocopy…) ; nouvelle dépendance `regex`.
- Alias et nombres (A-07) : comparaison des balises du moteur (clé `tags`
  des tables de champs : Alias*, Global*, Token*, BaseName, Pronoun*, balises
  de livre) et des nombres entre source et traduction ; « ⚠ » dans la grille
  (rouge : manquant ou inconnu, orange : autre ordre) avec le détail en
  info-bulle ; onglet Alias et nombres de la fenêtre d'édition avec
  « Rétablir les alias » et « Rétablir les nombres », opérations annulables.
- Air de famille avec XIMOD Architect (F-01 à F-06, étude du 08/10/2026) :
  raccourcis de menu centralisés et affichés (Ctrl+O, Ctrl+S, Ctrl+Maj+E,
  Ctrl+W, Ctrl+Q, Ctrl+, et F1 pour le manuel) ; fenêtre Paramètres avec
  Enregistrer / Annuler en panneau du bas, liseré de focus, aperçu du thème,
  fenêtre au premier plan au premier lancement ; ↑ ↓ PgUp PgDn Début Fin dans
  la grille ; « ● » dans le titre et « ● Modifié » dans la barre d'état ;
  notifications éphémères désactivables ([General] Toasts) ; croix d'effacement
  du filtre ; confirmations à bouton primaire en gras (Entrée / Échap) ;
  « Effacer » des récents en deux clics ; palette commune aux deux thèmes ;
  fermeture d'un document ou de l'application avec modifications non
  enregistrées confirmée (Enregistrer sous… / sans enregistrer / Annuler).
- Nouvelle crate `ximod-fomod`, extraite d'Architect 2.0.0 (modèle FOMOD,
  patcheur XML sans perte, document de traduction, mémoire, glossaire,
  empaquetage, extraction d'archives) ; la fenêtre « Traduire un FOMOD » est
  rebâtie dessus et devient celle d'Architect (table, détail, filtres, uniques,
  mémoire, glossaire, CSV, paquets zip / 7z, mise à jour depuis le dossier),
  sur le même fichier `.ximod-translation` ; l'aller-retour par liste
  numérotée et Ctrl+G sont conservés ; dépôt d'un dossier ou d'une archive,
  option `--fomod`. L'ancien module `fomod` et l'exemple `fomod_preview`
  sont retirés.
- Compteur de longueur (A-05) : caractères et octets dans l'encodage du jeu,
  rapportés à la limite du champ ; dans la fenêtre d'édition, compteur
  permanent (orange à 90 %, rouge au-delà) ; dans la grille, « 34 / 32 » en
  rouge et [!] en orange au bord de la cellule, détail en info-bulle. Tables
  de champs complétées : retour à la ligne des noms et descriptions, noms de
  Morrowind limités à 31 caractères, limites du Creation Kit en commentaire.
- Dépendances mises à jour (cargo update : async-recursion, cc, lazy_static,
  libc, tokio, unicase).
- Fenêtre d'édition (A-02) : fenêtre indépendante sur la ligne courante
  (Entrée, double-clic, Modifier…), source et traduction en zones larges,
  identité de la ligne, onglet Enregistrement, Précédent / Suivant
  (Alt+Gauche / Alt+Droite), statuts F5–F8, Ctrl+Entrée valide puis suivante,
  Échap conserve la saisie ; géométrie mémorisée ([Window] Edit*).
- Table des champs : le drapeau « retour à la ligne » devient tri-état
  (inconnu / oui / non) ; la fenêtre d'édition ne bloque Entrée que dans un
  champ déclaré mono-ligne.
- Grille : un clic sur le texte d'une cellule sélectionne bien la ligne (le
  texte n'est plus sélectionnable à la souris) ; menu de la ligne : raccourci
  affiché une seule fois, statuts sur pastille lisible, « Ctrl+Bas ».
- Menu contextuel de ligne unique (clic droit sur toute cellule) : édition
  (modifier, copier, coller dans la cellule ou la sélection, traduction =
  source), statut de la sélection, filtrer sur l'enregistrement ou la source,
  aperçu de livre, annuler / rétablir (M-01).
- Outils ▸ Traduire un FOMOD… : traduction des textes d'info.xml et
  ModuleConfig.xml, aide Google Traduction / IA (liste numérotée),
  enregistrement avec .bak (M-02).
- Outils ▸ Empaqueter une traduction… : dossier et archive zip pour MO2 /
  Vortex selon la famille du jeu, fomod/info.xml (M-03).
- Gabarits d'objets (Fallout 4, Fallout 76, Starfield) : les noms des
  gabarits « Editor Only » des WEAP / ARMO sont « jamais affichés en jeu » ;
  ceux des uniques légendaires restent traduisibles. Rappel à l'export : le
  plugin complet réécrit l'original, préférer le patch override pour publier.

### Modifié
- Fichier ▸ Appliquer une base de traduction… et Options ▸ Re-traduire les
  lignes auto (par similarité) sont remplacés par Options ▸ Appliquer les
  dictionnaires… (le second est le cas « Réinitialiser d'abord » + « Rien »).
- Grille : l'éditeur de la cellule Traduction occupe toute la hauteur de la
  ligne ; un clic en bas d'une ligne haute (longue source) ouvre la saisie.
- Fenêtre d'édition : les zones Source et Traduction laissent aux onglets de
  contexte la place de trois lignes ; rien ne passe plus sous la barre de
  boutons à la taille par défaut.
- Icônes : les flèches et les boutons Annuler / Rétablir utilisent la police
  d'icônes Phosphor (crate `egui-phosphor`, variante Regular, enregistrée en
  repli des familles proportionnelle et monospace) — annuler, rétablir,
  Précédent / Suivant, Monter / Descendre, flèches de tri, « → » des
  suggestions, des fragments, des messages et de la tabulation visible —
  ainsi que les chevrons des chemins de menu (« Outils ▸ Données ») et du
  bouton d'historique, les coches (statut Validé, toasts, FOMOD, archives),
  les croix de fermeture et des pastilles de filtre, les pastilles
  « modifié » et de sélection, les avertissements, le cadenas et les glyphes
  des statuts (à traduire, en attente, similarité, personnalisé, ignoré) et
  le retour chariot visible ; plus de carrés à la place des glyphes absents
  des polices embarquées (seuls Hack, monospace, les connaissait).
- Dépendances mises à jour (cargo update du 11/10/2026 : cc 1.7.0,
  serde_json 1.0.152, smallvec 1.16.3, syn 3.0.7, toml_edit 0.25.17,
  toml_parser 1.1.5) ; nouvelle dépendance `egui-phosphor` 0.7.
- Le changement de jeu ne ferme plus les fichiers ouverts : chaque document a
  ses réglages et se recharge seul « en tant que » le nouveau jeu.
- Par défaut, l'export n'emporte plus les lignes « Traduit par similarité »
  ni « En attente » : cochez-les dans le dialogue d'export si besoin.
- Les fichiers de secours, dictionnaires personnels et journaux sont écrits
  dans le dossier de configuration (mode utilisateur) et non plus à côté de
  l'exécutable.

## [0.1.0] - 2026-08-13

### Ajouté
- Première version : lecture des plugins TES4 / TES3, STRINGS, bases EET / SST
  / DSD / XML, explorateurs ESM / BSA / SWF, dictionnaires par jeu et par mod.
