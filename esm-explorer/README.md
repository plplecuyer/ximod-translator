# ESM Explorer

Explorateur autonome de plugins Bethesda (`.esp` / `.esm` / `.esl`), écrit en
Rust + egui. Destiné à être intégré plus tard dans **XIMOD Translator** comme
outil. Aucune dépendance au jeu, à son exécutable, à F4SE/SKSE, ni à un runtime
.NET : les plugins sont lus directement.

## État — jalons 1 & 2

Réalisé :

- Ouverture d'un plugin (`Fichier → Ouvrir…`, **glisser-déposer** d'un fichier
  sur la fenêtre, ou en argument : `esm-explorer <plugin>`) et **détection
  automatique du jeu**
  (Morrowind → Starfield) à partir du magic, des masters et de la form version.
- **Object Window** : liste des types d'enregistrements à gauche (avec compteurs),
  table `EDID | Form ID | Name | Type` à droite, **filtre texte** et **tri par
  colonne** (clic sur l'en-tête, sens inversé au second clic). Rendu virtualisé.
  Les colonnes rognent le texte trop long (points de suspension) et affichent le
  **texte complet en infobulle** au survol (Object Window et Cell View).
- **Cell View** : sélecteur de **World Space** (avec l'entrée spéciale
  *Interiors*), liste des cellules `EditorID | FormID | Name | Location` filtrable,
  et **références** de la cellule sélectionnée `EditorID | FormID | Type`. La
  hiérarchie worldspace → cellule → référence est reconstruite en parcourant
  l'arbre GRUP ; le *Type* d'une référence est résolu vers le type de son objet
  de base quand celui-ci est défini dans le même plugin (sinon la signature de la
  référence — REFR/ACHR/…). Les coordonnées de grille servent de *Location* pour
  les cellules extérieures.
- **File Explorer** : colonne de droite listant les **types de GRUP**
  (signatures d'enregistrement présentes, ex. ACTI, WEAP, QUST…) avec une case à
  cocher chacune, boutons **Select All / Select None**, filtre, et **deux boutons
  d'extraction** des types cochés — au choix **Fichier texte (.txt)** ou
  **Tableur (.xlsx)** :
    - *Fichier texte* : FormID, EDID, puis chaque sous-champ (texte lisible tel
      quel, valeurs 4 octets en hexadécimal — FormID/id de chaîne —, autres en
      taille d'octets).
    - *Tableur* : une **feuille par type sélectionné**, colonnes `FormID |
      EditorID | Nom | Description | Modèle | Icône | Catégorie` (voir plus bas).
  L'exportation tableur est donc **pilotée par la sélection du File Explorer** :
  seuls les types cochés sont exportés. Extraction texte aussi en ligne de
  commande : `esm-explorer --extract <plugin> <sortie.txt> [SIG,SIG,…|ALL]`.
- **Résolution des noms localisés (STRINGS)** : pour un plugin localisé
  (Skyrim SE, Fallout 4, Starfield), les champs texte contiennent un id de chaîne
  4 octets ; l'app charge les fichiers `<Plugin>_<Langue>.STRINGS/DLSTRINGS/
  ILSTRINGS`, qu'ils soient **en vrac** (à côté du plugin ou dans un dossier
  `Strings/`) ou **dans un BSA/BA2** voisin (les gros jeux, ex. la localisation
  Starfield, les empaquettent). Les trois fichiers sont fusionnés en une table
  `id → texte` (les ids sont uniques dans le plugin). La colonne **Name** (Object
  Window et Cell View) et l'extraction texte affichent alors le vrai libellé. Un
  **sélecteur de langue** liste les langues disponibles et recharge à la volée ;
  la barre d'état indique la langue et le nombre de STRINGS chargées. L'ouverture
  d'archives est volontairement restreinte (nom exact du plugin, ou archive de
  localisation) pour ne jamais ouvrir les énormes archives de textures. Les ids
  appartenant à un **master** (non fournis par le plugin) restent affichés `[id]`
  en attendant la résolution de l'ordre de chargement.
- **Détail d'un enregistrement** : cliquer une ligne (Object Window) ou une
  référence (Cell View) ouvre une fenêtre flottante montrant l'en-tête
  (signature, FormID, flags, « compressé »), la liste des sous-champs
  `Champ | Taille | Valeur` — valeur décodée : STRINGS résolus pour les champs
  texte localisés, texte inline, ou interprétations d'un champ 4 octets
  (`0x… · u32 · f32 · FormID`), aperçu hexa pour le binaire — et une **vue
  hexadécimale du corps** (repliable, avec gouttière ASCII). La sélection d'une
  cellule y est reflétée aussi.
- **Export tableur (.xlsx)** : bouton **Tableur (.xlsx)** du File Explorer (voir
  ci-dessus). Le classeur (ouvert par Excel **et** LibreOffice Calc) contient une
  **feuille par type sélectionné** (WEAP, ARMO, ALCH…) avec les colonnes `FormID |
  EditorID | Nom | Description | Modèle | Icône | Catégorie` (Nom/Description
  résolus via STRINGS, Modèle/Icône = chemins `.nif`/`.dds` ; les colonnes
  Modèle/Icône/Catégorie restent vides pour les types qui n'en ont pas). Les
  **armes sont triées par fonction** :
  la catégorie (pistolet, fusil, épée, dague, arc…) est déduite des mots-clés
  `KWDA`. Ceux-ci sont résolus soit depuis les **KYWD propres au fichier chargé**
  — cas d'un master de base ouvert directement (Starfield.esm, Skyrim.esm…) —,
  soit en lisant le **master de base** présent dans le dossier Data — cas d'un
  mod qui référence les mots-clés du jeu. Les définitions du fichier chargé
  priment. Si aucune source de mots-clés n'est disponible (mod dont le master de
  base est absent du dossier, et qui ne définit pas ses propres KYWD), la
  catégorie indique « (master de base introuvable) ». (Aussi en CLI :
  `esm-explorer --xlsx <plugin> <sortie.xlsx>` — exporte alors les types d'objet
  standard présents.)
  Note : la « représentation graphique » est donnée en **chemin** de modèle ; il
  n'y a pas de vignette d'image (les objets modernes n'ont qu'un modèle 3D `.nif`,
  pas d'icône 2D).
- Barre d'état : fichier, jeu détecté, form version, langue/STRINGS ou inline,
  nombre d'enregistrements, worldspaces et cellules.

À venir (jalons suivants) : résolution de l'ordre de chargement (masters, pour
les STRINGS et les types d'objets de base des références).

## Gros fichiers (Starfield.esm ~1,35 Go)

L'ouverture est **paresseuse et à faible mémoire** :

- le fichier est **mappé en mémoire** (`mmap`), jamais lu entièrement en RAM ;
- un **seul passage d'indexation ne lit que les en-têtes** de records/GRUP —
  aucune décompression, aucune copie de corps, pas de double parsing. Il construit
  l'index (signature, FormID, flags, position du corps) et la hiérarchie
  worldspace → cellule → référence ;
- les champs d'un enregistrement (EDID, nom FULL, coordonnées de cellule,
  références d'une cellule) sont **décodés à la demande** et mis en cache : comme
  les tables sont virtualisées, seules ~50 lignes sont décodées par frame ;
- filtrer/trier par **nom** sur tout le fichier décode le sous-ensemble courant
  **par petits paquets bornés en temps par frame** (avec un pourcentage dans la
  barre d'état) — l'interface ne se fige jamais. Trier par FormID/Type ou filtrer
  par type reste instantané (ordres pré-calculés à l'indexation).

Au-delà de ~400 000 records, la table FormID→record (résolution du *type* de base
d'une référence) est omise pour borner la mémoire ; les références montrent alors
leur propre signature (REFR/ACHR/…).

## Architecture

Le parseur est **vendorisé** depuis XIMOD Translator (`src/esm/`), réduit au
chemin de lecture, avec les **mêmes types** (`Plugin` / `Record` / `SubRecord` /
`Game`). Lors de l'intégration dans XIMOD, ce module sera remplacé par
`ximod_translator::plugin` sans toucher à l'interface.

```
src/
  esm/            parseur vendorisé (autonome, sans dépendance XIMOD)
    games.rs      détection du jeu + tables de champs (copie conforme)
    record.rs     modèle Plugin/Record/SubRecord
    reader.rs     lecture TES4 (GRUP, compression zlib, XXXX)
    tes3.rs       lecture Morrowind (TES3)
  app.rs          application egui (Object Window)
  main.rs         point d'entrée
examples/
  esm_dump.rs     test headless : parse + aperçu console
```

## Compiler / lancer

```
cargo run                       # lance l'interface
cargo run --example esm-dump -- <chemin-du-plugin>   # aperçu console
```

Nécessite Rust 1.97+ (édition 2024).
