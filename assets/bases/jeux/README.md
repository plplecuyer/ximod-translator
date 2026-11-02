# Bases de traduction des jeux

Déposez ici les **bases officielles des jeux** que vous fournissez (fichiers
**.eet**, .sst, .dsd, .json ou .xml), une par jeu et par langue.

Au chargement d'un mod, XIMOD Translator recherche dans ces bases les lignes
partageant les mêmes indicateurs (**GRUP, Champ, ID, EDID**) et la même source :
il traduit alors la ligne automatiquement et la marque **Validé**.

## Conversion en base XIMOD

Le menu *Options → Compiler les bases des jeux (→ base XIMOD)* lit tous les
fichiers de ce dossier et les regroupe dans une base native XIMOD
`base_jeux.ximod.json` (JSON pivot), plus rapide à charger. Vous pouvez ensuite
ne conserver que ce fichier compilé.

L'emplacement de ce dossier est configurable dans `Config.ini`
(section `[Bases]`, clé `GameBasesDir`).
