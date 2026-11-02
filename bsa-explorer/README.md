# BSA/BA2 Explorer

Explorateur et extracteur autonome d'**archives Bethesda** (`.bsa` / `.ba2`),
écrit en Rust + egui. Destiné à être intégré plus tard dans **XIMOD Translator**
comme outil. Aucune dépendance au jeu, à son exécutable, à F4SE/SKSE ni à un
runtime .NET : les archives sont lues directement.

Inspiré du *BSA Browser* (C#/.NET) de wrye-bash/AlexxEG, mais réécrit en Rust en
s'appuyant sur la crate [`ba2`] (pure Rust, licence 0BSD) — la bibliothèque de
l'auteur des outils C++ `bsa`/`ba2` de référence.

## État — jalon 1

Réalisé :

- **Ouverture de plusieurs archives à la fois** (`Fichier → Ouvrir…`,
  **glisser-déposer** sur la fenêtre, ou en argument :
  `bsa-explorer <archive> [autre…]`). Chaque archive ouverte a son **onglet**
  (nom + format), fermable par la croix ✖. Détection automatique du format et de
  la variante (BSA 103/104/105, BA2 GNRL, BA2 DX10).
- **Arborescence des dossiers** à gauche, repliable, avec le nombre de fichiers
  par sous-arbre ; l'entrée **(racine)** affiche toute l'archive.
- **Table des fichiers** virtualisée : `☐ | Nom | Taille | Compr. | Type`, avec
  **cases à cocher** pour la sélection, **filtre texte** (recherche sur tout le
  chemin, dans toute l'archive), et option **« Inclure les sous-dossiers »**. Le
  survol d'une ligne montre le **chemin complet** (et, pour une texture, ses
  dimensions / nombre de mips / format DXGI).
- **Aperçu intégré** (panneau de droite) :
    - **fichiers texte** (`txt`, `ini`, `json`, `xml`, `psc`, `cfg`…) décodés et
      affichés (aperçu tronqué au-delà de 200 Ko) ;
    - **images** `png` / `jpg` / `bmp` / `tga` ;
    - **textures DDS** (BC1…BC7 et non compressées) décodées en RGBA et
      affichées — y compris les textures **BA2 DX10** dont l'en-tête `.dds` est
      reconstruit à la volée. Les grands aperçus sont réduits à 1024 px.
- **Écoute audio** des voix / sons : sélectionner un fichier audio affiche un
  lecteur (Écouter / Pause / Stop, temps écoulé, barre de progression déplaçable).
    - **Formats standard** `.ogg` / `.wav` / `.flac` / `.mp3` (ex. voix `.ogg` et
      musique `.wav` de Fallout 3) : **lus directement**, sans aucun outil externe.
    - **`.wem` Wwise Vorbis** (voix Fallout 4 / Starfield) : **décodé en Rust pur,
      intégré à l'application — aucun outil externe requis**. La reconstruction de
      l'Ogg standard se fait via la crate [`ww2ogg`] (codebooks embarqués), puis
      le décodage Vorbis via [`lewton`]. L'app essaie automatiquement les jeux de
      codebooks (standard, aoTuV) et le mode inline.
    - **`.xwm` / `.fuz`** (voix Skyrim / Fallout 3-4) et **`.wem` non-Vorbis**
      (Opus/XMA) : décodés via un **outil externe détecté automatiquement** —
      **xWMAEncode** (`.xwm`/`.fuz`, requis pour Skyrim LE), **ffmpeg**
      (`.xwm`/`.fuz`) ou **vgmstream-cli** (tout), cherché à côté de l'exécutable
      puis dans le PATH. Le `.fuz` (conteneur `FUZE` + LIP + `.xwm`) est déballé
      automatiquement. Si aucun outil n'est présent, seul le `.wem` Wwise Vorbis
      reste écoutable (décodeur intégré) et le panneau indique quoi installer.

  Décodage xWMA (`.xwm`/`.fuz`) — stratégie par ordre d'essai :
    0. **xWMAEncode** (l'outil Microsoft), s'il est présent à côté de l'exécutable
       ou dans le PATH : c'est le **seul décodeur qui gère correctement TOUS les
       xWMA**, y compris les voix **Skyrim LE (2011) à 32 kHz** qu'aucun autre outil
       ne sait décoder. Utilisé **en priorité** dès qu'il est disponible ;
    1. sinon, le flux brut est confié au convertisseur externe (vgmstream/ffmpeg) :
       **plein succès pour Fallout 3/4 et Skyrim SE** (44,1 kHz). Skyrim LE, lui, en
       ressort muet/en erreur et bascule vers les voies suivantes ;
    2. **sous Windows**, une tentative via le décodeur **Media Foundation** ;
    3. en dernier recours, une **reconstruction WMAv2** (débit réel restauré +
       options d'encodage) repassée par **ffmpeg**.
  Pourquoi Skyrim LE est un cas à part : ces voix sont du xWMA 32 kHz que **seul le
  décodeur xWMA propre à Microsoft** (celui de XAudio2, qu'utilise Skyrim en jeu,
  et donc xWMAEncode) décode correctement. Le décodeur `wmav2` de ffmpeg — et donc
  vgmstream, qui repose dessus — comme le décodeur WMA « standard » de Windows les
  rendent **grésillants ou muets**. D'où l'obligation de passer par **xWMAEncode**
  pour ces fichiers : placez `xWMAEncode.exe` à côté de l'exécutable.
  Détail annexe : l'encodeur xWMA de Microsoft inscrit un **débit moyen erroné**
  pour certaines combinaisons canaux/fréquence ; l'app **restaure le débit réel**
  (table calquée sur vgmstream) avant les voies 2 et 3.
  L'app **valide toujours la présence de son réel** (ni vide ni muet) avant de
  lire, donc plus de « silence trompeur ». Le panneau indique le décodeur utilisé
  et rappelle, pour les `.xwm`/`.fuz`, de fournir `xWMAEncode.exe` pour Skyrim LE.
- **Comparaison de deux archives** (*Fichier → Comparer des archives…*, actif dès
  que deux archives sont ouvertes) : une fenêtre liste les fichiers **ajoutés**,
  **modifiés**, **supprimés** et **inchangés** entre l'archive A (gauche) et B
  (droite), avec un code couleur, des cases pour filtrer par catégorie, un filtre
  texte, les compteurs, et un **export texte**. La comparaison se fait par
  **chemin + taille décompressée** (rapide, sans extraction ; chemins comparés
  sans tenir compte de la casse). Aussi en CLI :
  `bsa-explorer --compare <archiveA> <archiveB>`.
- **Extraction** vers un dossier, **arborescence interne conservée** : soit la
  **sélection cochée**, soit **toute l'archive**. Pour une texture DX10, le
  fichier écrit est un `.dds` complet et valide. Chemins « nettoyés » (pas de
  `..` ni de racine absolue) pour rester dans le dossier de sortie.
- **Export de la liste des fichiers** (*Fichier → Exporter la liste…*), au choix
  **toute l'archive** ou **la vue filtrée** (dossier + filtre texte courants). Le
  format suit l'extension choisie dans la boîte d'enregistrement :
    - **`.csv`** (séparateur `;`, compatible Excel FR) — colonnes *Chemin, Nom,
      Taille, Compressé, Type, Largeur, Hauteur, Mips, DXGI* (les quatre dernières
      renseignées pour les textures DX10) ;
    - **`.txt`** — liste alignée lisible (indicateur compressé, taille, type,
      chemin), identique à `--list`.
- **Ligne de commande** :
    - `bsa-explorer --list <archive>` — liste chaque fichier (compressé, taille,
      type, chemin) ;
    - `bsa-explorer --export-list <archive> <sortie.csv|.txt>` — écrit la liste
      complète (format d'après l'extension) ;
    - `bsa-explorer --extract <archive> <dossier> [sous-chaîne]` — extrait tout
      (ou les chemins contenant la sous-chaîne), arborescence conservée.
- Barre d'état : archive, format/variante, nombre de fichiers, taille totale
  décompressée, nombre de fichiers affichés.
- Les menus se referment dès que la souris quitte le sous-menu (même
  comportement que XIMOD Translator et ESM Explorer).

Formats pris en charge : **BSA Morrowind (TES3)**, **BSA** (Oblivion, Fallout
3/NV, Skyrim, Skyrim SE) et **BA2** (Fallout 4, Fallout 76, Starfield), y compris
les archives de textures DX10. Le BSA Morrowind (version `0x100`, non compressé)
est lu **nativement** ici, car la crate `ba2` ne le gère pas. Les archives **GNMF
(PS4)** ne sont pas prises en charge pour l'instant.

À venir (jalons suivants) : intégration comme outil de XIMOD Translator.

## Gros fichiers

L'ouverture est **à faible mémoire** : chaque archive est **mappée en mémoire**
(`mmap`) et jamais lue entièrement en RAM. Même les archives de plusieurs
gigaoctets (ex. `Starfield*.ba2`, textures Fallout 4) s'ouvrent instantanément —
seules les pages réellement lues (table des fichiers, puis un fichier à
l'extraction) sont chargées à la demande. On peut donc ouvrir plusieurs grosses
archives en même temps sans saturer la mémoire.

## Architecture

Les boutons et indicateurs de l'interface utilisent des **icônes PNG** (croix de
fermeture, chevrons de dépliage, coche « compressé », tiret « non compressé »)
plutôt que des caractères Unicode — elles sont dans `assets/images/icons/` et
**embarquées dans le binaire** (`include_bytes!`), donc rien à installer.

```
src/
  archive.rs   lecture/extraction : `ba2` (BSA tes4 + BA2 fo4, GNRL & DX10) et
               parseur natif du BSA Morrowind (TES3) ; archive mappée (mmap)
  audio.rs     .wem Wwise Vorbis en Rust pur (ww2ogg + lewton) ;
               .xwm/.fuz et .wem non-Vorbis via vgmstream/ffmpeg
  compare.rs   comparaison de deux archives (ajoutés/modifiés/supprimés)
assets/images/icons/   icônes de l'interface (PNG 32×32, embarquées)
  tree.rs      construction de l'arborescence à partir des chemins plats
  preview.rs   décodage d'un fichier en aperçu (texte / image / DDS)
  app.rs       application egui (onglets, arbre, table, aperçu, extraction)
  main.rs      point d'entrée + modes CLI (--list, --extract)
```

La couche `archive.rs` garde les **mêmes formes publiques** (`Archive`,
`ArchiveEntry`) que le module d'archives d'ESM Explorer, pour une fusion future
indolore dans XIMOD Translator.

## Compiler / lancer

```
cargo run                          # lance l'interface
cargo run -- <archive.bsa|.ba2>    # ouvre une (ou plusieurs) archive(s)
cargo run -- --list <archive>      # liste en console
cargo run -- --extract <archive> <dossier> [sous-chaîne]
```

Nécessite Rust 1.97+ (édition 2024).

[`ba2`]: https://crates.io/crates/ba2
[`ww2ogg`]: https://crates.io/crates/ww2ogg
[`lewton`]: https://crates.io/crates/lewton
