# Dictionnaires du correcteur orthographique

XIMOD Translator utilise des dictionnaires **Hunspell** standards (les mêmes que
LibreOffice / OpenOffice / Mozilla), lus par la bibliothèque Rust `spellbook`
(pur Rust, MPL‑2.0 — aucune DLL propriétaire, multiplateforme).

## Convention

Déposez ici, côte à côte, une paire de fichiers par langue :

    <code>.aff   (règles d'affixes)
    <code>.dic   (liste des mots)

Le `<code>` est libre mais suit l'usage OpenOffice, par exemple :

    fr_FR.aff / fr_FR.dic     (français)
    en_US.aff / en_US.dic     (anglais — fourni d'origine)
    de_DE.aff / de_DE.dic     (allemand)
    es_ES.aff / es_ES.dic     (espagnol)
    it_IT.aff / it_IT.dic     (italien)

L'application choisit automatiquement le dictionnaire correspondant à la **langue
cible** de la traduction (barre d'outils « Cible »). La correspondance se fait sur
le code ISO : `fra` → `fr` → `fr_FR`, `eng` → `en` → `en_US`, etc.

## Où trouver des dictionnaires

- LibreOffice : https://github.com/LibreOffice/dictionaries
- Extensions OpenOffice : https://extensions.openoffice.org/
- Dictionnaire français « Dicollecte / Grammalecte » : https://grammalecte.net/

Copiez simplement les `.aff` et `.dic` de la langue voulue dans ce dossier, puis
cliquez sur **Recharger** dans la fenêtre du correcteur.

## Dictionnaire personnel

Les mots que vous ajoutez via **＋ Dictionnaire** sont enregistrés à côté de
`Config.ini` dans un fichier `personal_<langue>.dic` (un mot par ligne). Ils sont
rechargés automatiquement au démarrage et n'altèrent jamais les dictionnaires
d'origine.
