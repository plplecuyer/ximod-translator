# SWF Explorer

Outil de **texte SWF** pour **XIMOD Translator** : extraire les textes traduisibles
d'un fichier Flash `.swf`, les traduire, puis les **réinjecter** dans un SWF valide.
Écrit en Rust. Inspiré de JPEXS/FFDec, mais **volontairement ciblé sur le
round-trip de texte** (pas un décompilateur complet FLA/ActionScript).

## Ce qu'il fait

- **Lecture du conteneur** : Flash `FWS`/`CWS` (Skyrim, Fallout) **et Scaleform
  GFx `GFX`/`CFX` (Starfield)** — même flux de tags, les tags Scaleform (codes
  ≥ 1000) sont préservés tels quels. Le conteneur et la compression d'origine
  sont conservés à la réécriture. `ZWS` (LZMA) est détecté et signalé
  (ré-enregistrer en zlib).
- **Round-trip conservateur** : les tags non modifiés sont **préservés octet pour
  octet** (un chargement→sauvegarde sans édition redonne un corps identique). Seuls
  les tags dont on change le texte sont réécrits.
- **Sources de texte** :
    - **DefineEditText** (champs dynamiques, HTML) — **extraction + réinjection**.
      C'est là qu'est le texte des interfaces Bethesda et de la plupart des mods.
    - **Chaînes du pool ActionScript 3** (`DoABC`/`DoABC2`) — **extraction +
      réinjection**. Le bytecode ABC référence les chaînes par *index*, donc les
      réécrire ne casse jamais le code.
    - **Chaînes ActionScript 2** (`DoAction`/`DoInitAction` : *push* et *constant
      pool*) — **extraction + réinjection**. Un désassembleur/réassembleur AVM1
      **recalcule tous les décalages** (sauts `Jump`/`If`, tailles de blocs
      `DefineFunction`/`With`/`Try`) après édition. Filet de sécurité : un flux
      n'est réécrit que si son réassemblage sans édition est **identique à
      l'octet près** ; sinon il est laissé tel quel (jamais de corruption).
      **⚠ Attention** : en AS2, les chaînes servent aussi d'**identifiants de code**
      (noms de fonctions, variables, propriétés comme `_global`, `SetPlatform`…).
      Ne traduisez **que le vrai texte affiché** — modifier un identifiant
      casserait le script. En pratique, le texte d'UI est surtout dans les
      DefineEditText et les fichiers `translate_*.txt`, les littéraux AS2 étant
      majoritairement du code.
    - **Texte statique DefineText/DefineText2** — **extraction seule** (indices de
      glyphes convertis en caractères via la table de codes de la police). La
      réinjection exige un remappage des glyphes.
- **Clés Scaleform `$`** : les chaînes du type `$SKILLS`, `$Gold`… sont
  **signalées** (colonne `cle_scaleform`). Dans les jeux Bethesda elles sont
  traduites via `Interface/Translations/*.txt`, **pas** dans le SWF.
- Chaque texte porte un indicateur **`reinjectable`** pour que XIMOD sache
  lesquels peuvent être réécrits.

## Interface graphique

Lancer l'exécutable sans argument (ou double-clic) ouvre la **fenêtre** :
*Ouvrir SWF…*, un tableau des textes (id, type, réinjectable, clé `$`, texte
original, et une colonne **Traduction** éditable), puis *Enregistrer le SWF
traduit…*. On peut aussi **glisser-déposer** un `.swf` sur la fenêtre, filtrer,
masquer les non-réinjectables ou les clés `$`, et **exporter/importer** les
traductions en CSV/JSON pour les traiter dans Excel ou XIMOD. On peut aussi
ouvrir un fichier directement : `swf-explorer chemin.swf`.

## Utilisation (CLI)

```
swf-explorer --list    <in.swf>
swf-explorer --extract <in.swf> <out.csv|out.json>
swf-explorer --inject  <in.swf> <edits.csv|edits.json> <out.swf>
```

Le fichier d'extraction contient une colonne/champ **`traduction`** vide.
Remplissez-la, puis repassez le même fichier à `--inject` : seules les lignes
avec une `traduction` non vide **et** un type réinjectable sont écrites.

- **CSV** : séparateur `;` (compatible Excel FR), colonnes
  `id;type;reinjectable;categorie;cle_scaleform;html;contexte;texte;traduction`.
- **JSON** : tableau d'objets aux mêmes champs.

La colonne **`categorie`** est une heuristique d'aide au tri des chaînes
ActionScript : `code` (identifiant — ne pas traduire), `texte` (texte affiché —
à traduire), `incertain` (mot isolé — à revoir). Dans l'interface, une case
**« Masquer le code (AS) »** filtre les chaînes classées `code`, et la colonne
« Cat. » est colorée. Pour DefineEditText et le texte statique, la catégorie est
toujours `texte`.

L'`id` (ex. `et:31`, `as3:5:34`) est **stable** entre l'extraction et la
réinjection ; ne le modifiez pas.

## Intégration XIMOD

La bibliothèque expose `swf::Swf::parse` / `Swf::write` et
`text::extract` / `text::inject` (map `id → nouveau texte`), pour brancher
directement le pipeline de traduction de XIMOD sans passer par les fichiers CSV.

## Compiler

```
cargo run -- --extract interface/statsmenu.swf textes.csv
```

Nécessite Rust 1.97+ (édition 2024).

## Limites connues (V1)

- Texte statique DefineText : extraction seulement (réinjection = remappage de
  glyphes, à venir).
- Réinjection AS2 : un flux qui ne se réassemble pas à l'identique est laissé tel
  quel (jamais corrompu) ; et attention aux chaînes qui sont des identifiants de
  code (voir ci-dessus).
- `ZWS`/LZMA non pris en charge (ré-enregistrer en zlib).
- Les tags `DoAction` imbriqués dans des sprites ne sont pas parcourus (les UI
  Bethesda placent leur ActionScript au niveau racine).
