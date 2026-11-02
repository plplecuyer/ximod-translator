# Détection des jeux installés — Conception

Document de conception pour la détection automatique des jeux Bethesda pris en
charge par **ximod‑translator**, toutes plateformes de distribution (Steam, GOG,
GOG Galaxy, Epic Games, Xbox Game Pass PC, CD/DVD, installation manuelle) et tous
OS (Windows, Linux, macOS), avec enregistrement des emplacements dans
`Config.ini`.

Ce document précède l'implémentation : il fixe l'algorithme, les chemins, les
identifiants, le schéma de configuration et la stratégie Rust.

---

## 1. Objectifs et principes

- **Couverture** : retrouver, sans intervention de l'utilisateur, le dossier
  d'installation de chaque jeu supporté, quelle que soit la boutique et l'OS.
- **Zéro faux positif** : un jeu n'est retenu que si son **fichier maître** est
  présent (p. ex. `Skyrim.esm`). Un chemin déclaré par une boutique mais dont le
  fichier maître manque (jeu désinstallé, dossier vidé) est rejeté.
- **Robustesse avant exhaustivité** : on préfère ne rien déclarer plutôt que
  déclarer un mauvais chemin. Chaque source est facultative et isolée : l'échec
  d'un lecteur (registre absent, base SQLite verrouillée, `WindowsApps`
  inaccessible) n'interrompt pas les autres.
- **Sans privilèges** : aucune élévation (admin/root) requise. Les emplacements
  non lisibles sont ignorés proprement.
- **Bornée et rapide** : pas de balayage complet du disque par défaut. Les
  sources déclaratives (manifestes, registre, VDF) sont privilégiées ; le
  balayage de dossiers est limité en profondeur et en emplacements.
- **Repli manuel** : l'utilisateur peut toujours désigner un dossier à la main ;
  la même vérification par fichier maître s'applique.

---

## 2. Jeux pris en charge et identifiants

Le tableau ci‑dessous fait le lien entre l'énumération `Game` existante
(`src/games/mod.rs`), l'identifiant Steam, le nom de dossier usuel sous
`steamapps/common`, le sous‑dossier de données et le **fichier maître** servant
de preuve d'installation.

| `Game` (id)   | Steam AppID        | Dossier `common`              | Données      | Fichier maître   |
|---------------|--------------------|-------------------------------|--------------|------------------|
| `Morrowind`   | 22320 (GOTY)       | `Morrowind`                   | `Data Files` | `Morrowind.esm`  |
| `Oblivion`    | 22330 (GOTY)       | `Oblivion`                    | `Data`       | `Oblivion.esm`   |
| `Skyrim`      | 72850              | `Skyrim`                      | `Data`       | `Skyrim.esm`     |
| `SkyrimSE`    | 489830             | `Skyrim Special Edition`      | `Data`       | `Skyrim.esm`     |
| `Fallout3`    | 22300 / 22370 GOTY | `Fallout 3` / `Fallout 3 goty`| `Data`       | `Fallout3.esm`   |
| `FalloutNV`   | 22380              | `Fallout New Vegas`           | `Data`       | `FalloutNV.esm`  |
| `Fallout4`    | 377160             | `Fallout 4`                   | `Data`       | `Fallout4.esm`   |
| `Fallout76`   | 1151340            | `Fallout76`                   | `Data`       | `SeventySix.esm` |
| `Starfield`   | 1716740            | `Starfield`                   | `Data`       | `Starfield.esm`  |

Notes :

- Les **noms de dossier** ne servent que de repli. La source de vérité est le
  champ `installdir` du manifeste Steam (`appmanifest_<appid>.acf`) — on ne se
  fie jamais uniquement au nom.
- **GOG / Epic** utilisent d'autres identifiants (`productId` GOG, `AppName` /
  `CatalogItemId` Epic). Ils vivent dans la même table de correspondance mais
  sont **optionnels** : la vérification par fichier maître permet de détecter un
  jeu même sans connaître son id boutique. Les id GOG/Epic connus seront
  renseignés au fil de l'eau ; un id inconnu n'empêche pas la détection par
  dossier + fichier maître.
- **Skyrim LE vs SE** partagent `Skyrim.esm` : la désambiguïsation est traitée en
  §8.

Ces identifiants sont regroupés dans une table statique unique
(`GAME_IDS`) pour n'avoir qu'un seul point de maintenance.

---

## 3. Modèle de données

```rust
/// D'où provient une détection (sert à prioriser et à tracer dans Config.ini).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Steam,
    GogRegistry,
    GogGalaxy,
    Epic,
    Xbox,
    FolderScan,
    Manual,
}

/// Un jeu retrouvé sur le disque, déjà validé (fichier maître présent).
pub struct DetectedGame {
    pub game: crate::games::Game,
    pub root: PathBuf,       // dossier d'installation (…/Skyrim Special Edition)
    pub data_dir: PathBuf,   // sous-dossier de données (…/Data)
    pub source: Source,
}

/// Correspondance statique jeu ↔ identifiants boutiques ↔ preuves de présence.
struct GameIds {
    game: crate::games::Game,
    steam_appids: &'static [u32],
    gog_product_ids: &'static [u64],
    epic_names: &'static [&'static str], // sous-chaînes du DisplayName Epic
    common_dirs: &'static [&'static str],// noms de dossier usuels (repli/scan)
    data_subdir: &'static str,           // "Data" ou "Data Files"
    master_file: &'static str,           // "Skyrim.esm", …
}
```

---

## 4. Algorithme général (pipeline)

```
détecter_tous() -> Vec<DetectedGame>
  candidats = []
  candidats += steam::detecter()          // §5.1
  candidats += gog_registre::detecter()   // §5.2  (Windows)
  candidats += gog_galaxy::detecter()     // §5.3  (Windows/macOS)
  candidats += epic::detecter()           // §5.4
  candidats += xbox::detecter()           // §5.5  (Windows)
  candidats += scan_dossiers::detecter()  // §5.6  (borné)

  # 1) Validation : ne garder que ceux dont le fichier maître existe
  candidats.retain(|c| existe_insensible_casse(c.data_dir.join(master_file(c.game))))

  # 2) Déduplication par jeu, en gardant la meilleure source
  #    (priorité §7) puis le chemin le plus court/le plus « propre »
  resultat = dedup_par_jeu(candidats, priorité)

  return resultat
```

Chaque source renvoie des **candidats** (jeu supposé + dossier). La validation
par fichier maître est **centralisée** : les lecteurs de sources n'ont pas à la
refaire. La comparaison de nom de fichier est **insensible à la casse** (Linux).

---

## 5. Détection par plateforme

### 5.1 Steam (tous OS) — source principale

**a. Localiser la racine Steam :**

- **Windows** : registre
  `HKCU\Software\Valve\Steam` → valeur `SteamPath` (ou
  `HKLM\SOFTWARE\WOW6432Node\Valve\Steam` → `InstallPath`). Repli :
  `C:\Program Files (x86)\Steam`.
- **Linux** : `~/.steam/steam`, `~/.steam/root`, `~/.local/share/Steam`, Flatpak
  `~/.var/app/com.valvesoftware.Steam/data/Steam`, Snap
  `~/snap/steam/common/.local/share/Steam`.
- **macOS** : `~/Library/Application Support/Steam`.

**b. Énumérer les bibliothèques :** lire
`<steam>/steamapps/libraryfolders.vdf` (format VDF Valve). Chaque entrée donne un
`path` de bibliothèque (les jeux ne sont pas tous dans la racine Steam ; souvent
sur un autre disque).

**c. Pour chaque bibliothèque et chaque AppID connu :** lire
`<lib>/steamapps/appmanifest_<appid>.acf` (VDF), extraire `installdir`. Le dossier
du jeu est `<lib>/steamapps/common/<installdir>`. On associe l'AppID au `Game`
via `GAME_IDS`.

**d. Proton/Wine (Linux) :** les jeux Windows lancés via Proton ont malgré tout
leurs fichiers dans `<lib>/steamapps/common/<installdir>` (chemin natif Linux) —
la détection Steam fonctionne donc **sans traiter les préfixes Wine**.

> Recommandation : s'appuyer sur la crate **`steamlocate`** (multi‑OS : gère
> racine Steam, `libraryfolders.vdf` et `appmanifest`) pour éviter de
> réécrire/maintenir un parseur VDF et la logique par OS. Repli maison possible
> avec un petit parseur VDF si l'on veut zéro dépendance.

### 5.2 GOG — registre (Windows)

- `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<gameId>` → valeurs `path` (dossier
  d'installation), `exe`, `gameName`. (Sur systèmes 32 bits : sans
  `WOW6432Node`.)
- On parcourt les sous‑clés `<gameId>` ; si `<gameId>` figure dans
  `gog_product_ids`, on associe le `Game` ; sinon, on retient tout de même le
  `path` comme **candidat à valider** par fichier maître (permet de détecter un
  jeu GOG dont on n'a pas l'id).

### 5.3 GOG Galaxy — base SQLite (Windows/macOS)

- Emplacement de la base :
  - **Windows** : `C:\ProgramData\GOG.com\Galaxy\storage\galaxy-2.0.db`
  - **macOS** : `~/Library/Application Support/GOG.com/Galaxy/storage/galaxy-2.0.db`
- Lecture **en lecture seule** (ouvrir en mode `immutable`/`ro` pour éviter tout
  verrou pendant que Galaxy tourne). La table des produits installés
  (`InstalledBaseProducts`, jointe aux détails/limitedDetails pour le chemin)
  fournit `productId` + chemin d'installation.

> ⚠️ Le **schéma de `galaxy-2.0.db` n'est pas documenté** et varie selon les
> versions de Galaxy. L'implémentation doit :
> 1. introspecter les tables/colonnes présentes (`PRAGMA table_info`) et
>    s'adapter (ne pas coder en dur des noms de colonnes non garantis) ;
> 2. rester tolérante : si la requête échoue, la source est simplement ignorée ;
> 3. dans tous les cas, **valider par fichier maître** le chemin obtenu.
>
> Coût : ajoute la dépendance **`rusqlite`** (feature `bundled` → compile
> `libsqlite3`, build plus lourd). À isoler derrière une **feature Cargo
> optionnelle** `gog-galaxy` pour que ceux qui n'en veulent pas gardent un build
> léger. Repli sans dépendance : lire via l'outil `sqlite3` s'il est présent
> (moins fiable), sinon se contenter du registre GOG (§5.2).

### 5.4 Epic Games

- **Windows** : manifestes JSON dans
  `C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests\*.item`. Champs utiles :
  `InstallLocation`, `DisplayName`, `AppName`, `CatalogItemId`,
  `MainGameCatalogItemId`.
- **macOS** :
  `~/Library/Application Support/Epic/EpicGamesLauncher/Data/Manifests/` (et
  `/Users/Shared/Epic/...`).
- **Linux** : pas de launcher Epic officiel ; prise en charge via **Heroic /
  Legendary** (`~/.config/legendary/installed.json`,
  `~/.config/heroic/gog_store`/`legendaryConfig`). Optionnel.
- Association : par `AppName`/`CatalogItemId` si connu, sinon par correspondance
  de `DisplayName` (sous‑chaîne) **et** validation par fichier maître dans
  `InstallLocation`.

### 5.5 Xbox Game Pass PC (Windows) — fiabilité limitée

- Les jeux « installés » modifiables sont rangés dans un dossier choisi par
  l'utilisateur (souvent `XboxGames`, ou `WindowsApps` sur un disque). Chaque
  racine de lecteur contient un fichier binaire **`.GamingRoot`** (magie `RGBX`
  suivie d'un chemin en UTF‑16) pointant vers le dossier des jeux de ce lecteur.
- Algorithme : pour chaque lecteur, lire `.GamingRoot` → dossier des jeux →
  sous‑dossiers → `Content` → **valider par fichier maître**.
- Limites : `WindowsApps` est protégé (propriété `TrustedInstaller`, souvent
  illisible sans élévation) ; certaines éditions Store chiffrent/verrouillent les
  données. On **n'exige pas** d'y accéder : en cas d'échec de lecture, la source
  est ignorée. À considérer comme « best effort ».

### 5.6 CD/DVD et installation manuelle — balayage borné + repli

- **Balayage borné** des emplacements d'installation courants, avec profondeur
  limitée (≤ 3–4 niveaux) et correspondance par **fichier maître** :
  - **Windows** : `C:\Program Files`, `C:\Program Files (x86)`,
    `C:\Games`, `C:\GOG Games`, `C:\Bethesda*`, et la racine de chaque lecteur
    fixe détecté ; plus les dossiers Bethesda Launcher historiques.
  - **Linux** : `~/Games`, `~/GOG Games`, `/opt`, `~/.wine/drive_c/...`
    (préfixes Wine courants), bibliothèques Lutris/Heroic.
  - **macOS** : `/Applications`, `~/Applications`, préfixes CrossOver
    (`~/Library/Application Support/CrossOver/Bottles/*/drive_c/...`).
- **Repli manuel** : entrée d'interface « Ajouter un jeu… » → l'utilisateur
  pointe le dossier ; validation par fichier maître ; enregistrement avec
  `Source::Manual`. C'est le filet de sécurité universel (CD/DVD monté, copie
  réseau, emplacement exotique).
- Le balayage complet du disque n'est **pas** fait automatiquement (coût/temps) ;
  il peut être proposé en option explicite « recherche approfondie ».

---

## 6. Spécificités par OS

- **Sensibilité à la casse (Linux)** : toutes les recherches de fichiers
  (`Skyrim.esm`, `Data`) se font en **comparaison insensible à la casse** en
  listant le dossier, car les copies de jeux Windows y ont des casses variées
  (`data/`, `Data/`, `SKYRIM.ESM`).
- **Proton/Wine** : voir §5.1.d — pas de traitement spécial pour Steam. Pour GOG
  registre / Epic sous Wine, il faudrait lire le registre du préfixe Wine
  (`system.reg`) : considéré **hors périmètre initial** (repli : scan + manuel).
- **macOS** : la plupart des jeux Bethesda sont Windows‑only ; en pratique on
  détecte ce qui est présent via Steam/CrossOver. Chemins `~/Library/...`
  spécifiques déjà listés.
- **Registre Windows** : accès via la crate `winreg`, **compilée uniquement sous
  `#[cfg(windows)]`** ; sur les autres OS ces lecteurs sont des no‑op.

---

## 7. Priorité des sources et déduplication

Quand un même jeu est trouvé par plusieurs sources, on garde une seule entrée
selon la priorité (de la plus fiable à la moins fiable) :

```
Steam  >  GogGalaxy  >  GogRegistry  >  Epic  >  Xbox  >  FolderScan  >  Manual*
```

`Manual` prime cependant sur tout si l'utilisateur a explicitement fixé le
chemin (drapeau « verrouillé »), pour ne pas écraser un choix délibéré. À
priorité égale, on préfère le chemin qui **valide** le fichier maître et, en cas
d'égalité, le plus court.

---

## 8. Désambiguïsation

- **Skyrim (LE) vs Skyrim Special Edition** : même `Skyrim.esm`. On distingue par
  (dans l'ordre) : l'**AppID** (72850 vs 489830) ou le **nom de dossier**
  (`Skyrim` vs `Skyrim Special Edition`), puis, en dernier recours, la présence
  de binaires spécifiques (`SkyrimSE.exe` pour SE, `TESV.exe` pour LE) ou des
  `ccBGSSSE*.esl`/CC (SE/AE uniquement).
- **Oblivion vs Oblivion Remastered** : le Remastered a une arborescence
  différente (moteur UE5, données sous `OblivionRemastered/Content/...` et non un
  `Oblivion.esm` classique à la racine `Data`) ; il n'est **pas** assimilé à
  `Oblivion` tant que le parseur de plugins ne le gère pas. On ne le retient que
  si `Data/Oblivion.esm` classique est présent.
- **Éditions GOTY** : mêmes fichiers maîtres ; AppIDs multiples listés dans
  `steam_appids`. Aucun traitement particulier.

---

## 9. Schéma `Config.ini`

Ajout d'une section `[Games]` (le reste du fichier — `[General]`,
`[Translation]`, `[Bases]`… — est inchangé). Une paire de clés par jeu détecté,
indexée par l'`id` stable du `Game` :

```ini
[Games]
AutoDetectAtStartup=1
GamePath_SkyrimSE=D:\SteamLibrary\steamapps\common\Skyrim Special Edition
GameSource_SkyrimSE=Steam
GamePath_Fallout4=C:\Program Files (x86)\Steam\steamapps\common\Fallout 4
GameSource_Fallout4=Steam
GamePath_Morrowind=/home/user/GOG Games/Morrowind
GameSource_Morrowind=FolderScan
GameLocked_Morrowind=1     ; chemin fixé manuellement, non écrasé par l'auto-détection
```

- `GamePath_<Id>` : dossier **racine** d'installation (le sous‑dossier de données
  se déduit via `data_subdir`, mais peut être recalculé/validé à la lecture).
- `GameSource_<Id>` : trace la provenance (`Steam`, `GogGalaxy`, `GogRegistry`,
  `Epic`, `Xbox`, `FolderScan`, `Manual`).
- `GameLocked_<Id>` (optionnel) : `1` = choix manuel à préserver.
- `AutoDetectAtStartup` : lance la détection au démarrage si aucun jeu n'est
  encore enregistré (ou jamais, selon préférence).

Extension de `AppConfig` (dans `src/config.rs`) : un
`games: BTreeMap<String /*id*/, GameEntry>` avec
`GameEntry { path: String, source: String, locked: bool }`, sérialisé/parsé dans
`to_ini`/`parse_ini` sur le même modèle que `[Bases]` et les couleurs de statut.
Ces chemins alimentent ensuite les valeurs par défaut des sélecteurs (import
STRINGS d'un jeu, construction de dictionnaire de jeu, etc.).

---

## 10. Implémentation Rust

### 10.1 Découpage en modules

```
src/games/
  mod.rs            (existant : enum Game, ids, master files)
  ids.rs            (table GAME_IDS : AppID/GOG/Epic/dossiers/maître/données)
  detect/
    mod.rs          (DetectedGame, Source, detect_all(), dedup, validation)
    steam.rs        (steamlocate ou parseur VDF maison)
    gog_registry.rs (#[cfg(windows)] winreg)
    gog_galaxy.rs   (#[cfg(feature = "gog-galaxy")] rusqlite)
    epic.rs         (serde_json sur *.item)
    xbox.rs         (#[cfg(windows)] .GamingRoot)
    scan.rs         (walkdir borné + repli manuel)
```

### 10.2 Dépendances

| Besoin              | Crate                    | Portée / remarque                              |
|---------------------|--------------------------|------------------------------------------------|
| Steam (VDF, libs)   | `steamlocate`            | multi‑OS ; sinon parseur VDF maison            |
| Registre Windows    | `winreg`                 | `#[cfg(windows)]` uniquement                   |
| GOG Galaxy SQLite   | `rusqlite` (`bundled`)   | derrière la feature `gog-galaxy` (build lourd) |
| Manifs Epic (JSON)  | `serde_json`             | **déjà** dépendance du projet                  |
| Balayage dossiers   | `walkdir`                | profondeur bornée                              |

Les lecteurs spécifiques à un OS sont compilés conditionnellement ; sur un OS où
une source n'existe pas, la fonction renvoie `Vec::new()`.

### 10.3 Esquisse d'API

```rust
// src/games/detect/mod.rs
pub fn detect_all() -> Vec<DetectedGame> {
    let mut c = Vec::new();
    c.extend(steam::detect());
    #[cfg(windows)] c.extend(gog_registry::detect());
    #[cfg(feature = "gog-galaxy")] c.extend(gog_galaxy::detect());
    c.extend(epic::detect());
    #[cfg(windows)] c.extend(xbox::detect());
    c.extend(scan::detect());          // bornée
    validate_and_dedup(c)              // fichier maître + priorité §7
}

/// Fusionne les résultats dans la config (sans écraser les chemins verrouillés)
/// puis sauvegarde Config.ini.
pub fn detect_and_save(cfg: &mut crate::config::AppConfig) -> usize { … }
```

Validation centralisée :

```rust
fn master_present(g: Game, root: &Path) -> Option<PathBuf> {
    let data = find_child_ci(root, data_subdir(g))?;      // insensible casse
    let esm  = find_child_ci(&data, master_file(g))?;     // insensible casse
    esm.exists().then_some(data)
}
```

### 10.4 Exécution asynchrone / UI (à implémenter ensuite)

- La détection tourne dans un **thread** (pas sur la boucle egui), avec un canal
  renvoyant la liste ; l'UI affiche une progression et un récapitulatif
  (jeu → chemin → source) avant d'écrire dans `Config.ini` — cohérent avec les
  autres dialogues récents (import).
- Entrée de menu proposée : **Outils → « Détecter les jeux installés… »**, plus
  « Ajouter un jeu… » (repli manuel) et l'édition des chemins dans les Réglages.

---

## 11. Robustesse, performance, sécurité

- **Isolation des erreurs** : chaque source est enveloppée ; toute erreur (I/O,
  registre, SQLite, JSON) est journalisée (`tracing`) et convertie en « aucun
  candidat », jamais en panique.
- **Coût borné** : sources déclaratives d'abord ; le balayage de dossiers est
  limité en profondeur et en emplacements ; option explicite pour une recherche
  approfondie.
- **Aucune écriture** hors `Config.ini` ; la base GOG Galaxy est ouverte en
  lecture seule.
- **Pas d'élévation** : les emplacements protégés (`WindowsApps`) sont tentés
  puis ignorés s'ils sont illisibles.
- **Idempotence** : relancer la détection mérge sans dupliquer ; respecte
  `GameLocked_*`.

---

## 12. Plan de tests

- **Parseurs** (tests unitaires sur fixtures, multi‑OS) :
  - `libraryfolders.vdf` et `appmanifest_<appid>.acf` → `installdir` ;
  - manifeste Epic `*.item` (JSON) → `InstallLocation` + association ;
  - `.GamingRoot` (magie `RGBX` + chemin UTF‑16) → dossier des jeux ;
  - introspection + requête `galaxy-2.0.db` sur une base d'exemple (feature).
- **Validation** : arbres de dossiers factices (avec/sans fichier maître,
  casses variées) → détection correcte, zéro faux positif.
- **Désambiguïsation** : dossiers `Skyrim` vs `Skyrim Special Edition` avec le
  même `Skyrim.esm` → bon `Game`.
- **Config.ini** : aller‑retour `[Games]` (parse ↔ to_ini), préservation des
  chemins verrouillés, fusion sans doublon.
- **Priorité** : même jeu vu par plusieurs sources → la source attendue gagne.

---

## 13. Limites connues

- GOG Galaxy : schéma de base non garanti entre versions (mitigé par
  l'introspection et le repli registre/scan).
- Xbox Game Pass : accès `WindowsApps` souvent impossible ; couverture partielle.
- Epic sous Linux : dépend de Heroic/Legendary.
- GOG/Epic sous Wine (Linux/macOS) : registre du préfixe non lu au départ (repli
  scan + manuel).
- Oblivion Remastered : non assimilé à `Oblivion` tant que le moteur UE5 n'est
  pas géré côté plugins.

---

## 14. Périmètre validé pour l'implémentation

Sur la base des choix faits : **Steam (tous OS)**, **Epic + GOG (registre) +
balayage de dossiers**, **GOG Galaxy (SQLite, derrière une feature)** et
**Xbox Game Pass PC (best effort)** — tous convergeant vers la validation par
fichier maître et l'enregistrement dans `Config.ini` `[Games]`.

Étapes suggérées, dans l'ordre : (1) `ids.rs` + validation + `[Games]` dans la
config ; (2) Steam ; (3) scan borné + repli manuel ; (4) GOG registre + Epic ;
(5) GOG Galaxy (feature) ; (6) Xbox ; (7) UI de détection.
