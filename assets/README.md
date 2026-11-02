# assets/

Ressources chargées au runtime (jamais embarquées dans le binaire).

```
assets/
├── data/
│   ├── Countries.json          ✅ fourni
│   ├── Languages.json          ✅ fourni
│   ├── CountryLanguages.json   ⬜ optionnel (dégradation en liste vide)
│   ├── CountryNames.json       ✅ noms de pays par langue
│   ├── GamesDLC.json           ✅ DLC connus par jeu (STRINGS à indexer)
│   ├── games/<Jeu>.json        ✅ fiches de jeu (S-05 ; version intégrée en secours)
│   └── fields/<Jeu>.json       ✅ champs traduisibles par jeu (A-06 ; version intégrée en secours)
├── fonts/                      ⬜ polices Noto (partagées avec XIMOD Architect)
├── icons/
│   ├── app_icon.png            ⬜ icône fenêtre (Linux/dev)
│   ├── ximod-translator.ico    ⬜ icône Windows (embarquée par build.rs)
│   ├── ximod-translator.icns   ⬜ icône macOS
│   └── ximod-translator_*.png  ⬜ tailles Linux (16..512)
├── images/
│   ├── icons/                  ⬜ Arrow_*.png, delete.png
│   ├── svg/                    ⬜ 244 drapeaux de pays
│   └── splash.png              ⬜ écran d'ouverture
└── locales/<iso639-3>/main.ftl ✅ eng, fra fournis
```

Les fichiers `data/games/` et `data/fields/` sont aussi embarqués dans la crate
`ximod-games` : un fichier absent ou illisible est remplacé par sa copie
intégrée (et signalé dans `logs/ximod_startup.log`). Un fichier de champs
utilisateur (`<config>/data/fields/<Jeu>.json`, écrit par Outils ▸ Données ▸
Champs traduisibles par jeu…) l'emporte entièrement sur le fichier livré.

Les gros binaires (polices, drapeaux, icônes, splash) sont **communs à XIMOD
Architect** : recopiez-les depuis son arborescence `assets/`. Tout ce qui manque
dégrade proprement — polices intégrées d'egui, pas de splash, icône par défaut.
