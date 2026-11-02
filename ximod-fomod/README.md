# ximod-fomod

Bibliothèque partagée par **XIMOD Architect** et **XIMOD Translator** pour tout
ce qui touche aux installateurs FOMOD (F-03, octobre 2026) :

| module          | origine dans Architect 2.0.0            | contenu |
|-----------------|-----------------------------------------|---------|
| `model`         | `src/models/ximod.rs`                   | le modèle de l'installateur (`Ximod`, étapes, groupes, options…) |
| `xml`           | `src/xml/{mod,validate,fidelity}.rs`    | lecture / écriture d'`info.xml` et `ModuleConfig.xml`, arbre de validation |
| `xml::patch`    | `src/xml/patch.rs`                      | le **patcheur sans perte** : seules les chaînes visées changent |
| `translate`     | `src/models/translate.rs`               | le document `.ximod-translation`, statuts, mémoire, glossaire, contrôles |
| `package`       | `src/cli_translate.rs` (hors CLI)       | lire un FOMOD, extraire / mettre à jour / appliquer une traduction, empaqueter |
| `export`, `archive` | `src/export.rs`, `src/archive.rs` (sans BA2) | écrivains zip / 7z |
| `archive_open`  | `src/archive_open.rs`                   | extraction du dossier `fomod/` d'une archive de mod |
| `lang`          | `src/i18n/mod.rs` (table des langues)   | ISO 639-1 ↔ 639-3, endonymes |

Le code est celui d'Architect, à trois adaptations près :

* `crate::APP_NAME / APP_VERSION / APP_URL` → `ximod_fomod::set_tool(nom, version, url)`
  à l'initialisation, lu par `ximod_fomod::tool()` (commentaire « Created with … »
  des XML écrits, champ `tool` des documents) ;
* `crate::config::AppConfig::config_dir()` → `ximod_fomod::set_data_dir(Some(dossier))`
  (mémoire de traduction, glossaires, dossier de travail des archives) ;
* `backups::BACKUPS_DIR` et `simulate::SCENARIOS_DIR` → constantes de la crate.

## Adoption par Architect

1. Ajouter `ximod-fomod = { path = "../ximod-translator/ximod-fomod" }` (ou la
   version publiée) aux dépendances.
2. Appeler `ximod_fomod::set_tool(APP_NAME, APP_VERSION, APP_URL)` et
   `ximod_fomod::set_data_dir(AppConfig::config_dir())` au démarrage.
3. Remplacer les modules copiés par des réexports :
   `pub use ximod_fomod::model as ximod;` dans `models/mod.rs`,
   `pub use ximod_fomod::xml::*;` dans `xml/mod.rs`,
   `pub use ximod_fomod::translate;`, `pub use ximod_fomod::archive_open;`,
   `pub use ximod_fomod::package::*;` dans `cli_translate.rs` (la partie CLI reste
   dans Architect), `export` et `archive` de même (le paquetage BA2 reste dans
   Architect : `package_ba2`, `Ba2Options`, `Ba2Game`).
4. Les tests des modules déplacés vivent dans la crate (84 tests).

Les deux applications lisent et écrivent alors le même fichier
`fomod/translations/<mod>.<langue>.ximod-translation`.
