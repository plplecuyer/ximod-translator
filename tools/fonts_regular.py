#!/usr/bin/env python3
"""Ne garder que la police Regular de chaque famille Noto citée (D-104).

Pour chaque dossier assets/fonts/Noto_xxx dont une police est citée par les
données (assets/data/*.json, clé « font ») :

  1. supprime à la racine du dossier les polices variables
     (« …-VariableFont_wght.ttf », « …-VariableFont_wdth,wght.ttf », italiques
     comprises) et le fichier README.txt (OFL.txt, la licence, est gardé) ;
  2. déplace la police citée de static/ vers la racine du dossier ;
  3. supprime le sous-dossier static/ et tout son contenu ;
  4. met à jour les chemins « Noto_xxx/static/… » des données et des
     métadonnées « # @font = » des fichiers de langue (assets/locales).

Un dossier dont aucune police n'est citée n'est pas touché (il est signalé),
pas plus que le dossier books/ des polices de livre. Le programme peut être
relancé : ce qui est déjà fait est laissé tel quel.

Usage (depuis la racine du dépôt ximod-translator) :
    python tools/fonts_regular.py               simulation : affiche ce qui serait fait
    python tools/fonts_regular.py --appliquer   fait les changements
    python tools/fonts_regular.py <racine> [--appliquer]
"""
import json
import shutil
import sys
from pathlib import Path

FONT_EXT = (".ttf", ".otf", ".ttc")


def die(msg):
    print(f"ERREUR : {msg}")
    sys.exit(1)


def size_of(path):
    if path.is_file():
        return path.stat().st_size
    return sum(f.stat().st_size for f in path.rglob("*") if f.is_file())


def mo(n):
    return f"{n / 1048576:.1f} Mo"


def collect_refs(node, out):
    """Les valeurs des clés « font » d'un document JSON, à toute profondeur."""
    if isinstance(node, dict):
        for k, v in node.items():
            if k == "font" and isinstance(v, str) and v.lower().endswith(FONT_EXT):
                out.add(v.replace("\\", "/"))
            else:
                collect_refs(v, out)
    elif isinstance(node, list):
        for v in node:
            collect_refs(v, out)


def read_text(path):
    # newline="" garde les fins de ligne du fichier (CRLF d'un dépôt Windows).
    with open(path, encoding="utf-8", newline="") as f:
        return f.read()


def write_text(path, text):
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    apply = "--appliquer" in sys.argv[1:]
    root = Path(args[0]).resolve() if args else Path.cwd().resolve()
    fonts = root / "assets" / "fonts"
    data = root / "assets" / "data"
    locales = root / "assets" / "locales"
    if not fonts.is_dir() or not data.is_dir():
        die(f"{root} n'est pas la racine du dépôt (assets/fonts ou assets/data introuvable)")

    data_files = sorted(data.glob("*.json"))
    refs = set()
    for f in data_files:
        try:
            collect_refs(json.loads(read_text(f)), refs)
        except json.JSONDecodeError as e:
            die(f"{f.name} illisible : {e}")
    by_family = {}
    for r in refs:
        by_family.setdefault(r.split("/")[0], []).append(r)

    print(("Simulation" if not apply else "Application") + f" sur {fonts}")
    before = size_of(fonts)
    moves = {}  # ancien chemin cité -> nouveau chemin cité
    actions = 0

    for family in sorted(p for p in fonts.iterdir() if p.is_dir() and p.name.startswith("Noto_")):
        cited = by_family.get(family.name, [])
        if not cited:
            print(f"  {family.name} : aucune police citée, dossier laissé tel quel ({mo(size_of(family))})")
            continue
        steps = []
        for f in sorted(family.iterdir()):
            if f.is_file() and "VariableFont" in f.name and f.suffix.lower() in FONT_EXT:
                steps.append(("suppr", f))
            elif f.is_file() and f.name.lower() == "readme.txt":
                steps.append(("suppr", f))
        static = family / "static"
        for rel in cited:
            parts = rel.split("/")
            if len(parts) == 3 and parts[1] == "static":
                src = fonts / rel
                dst = family / parts[2]
                if src.is_file():
                    if dst.exists():
                        if dst.read_bytes() != src.read_bytes():
                            die(f"{dst} existe déjà et diffère de {src}")
                    else:
                        steps.append(("deplacer", src, dst))
                elif not dst.is_file():
                    die(f"police citée introuvable : {rel}")
                moves[rel] = f"{parts[0]}/{parts[2]}"
            elif not (fonts / rel).is_file():
                die(f"police citée introuvable : {rel}")
        if static.is_dir():
            # Garde-fou : on ne supprime qu'un dossier « static » de assets/fonts.
            if static.parent.parent != fonts or static.name != "static":
                die(f"chemin inattendu : {static}")
            steps.append(("suppr_dossier", static))
        others = [f.name for f in family.iterdir()
                  if f.is_file() and f.suffix.lower() in FONT_EXT and "VariableFont" not in f.name
                  and f"{family.name}/{f.name}" not in set(moves.values()) | set(cited)]
        others_dirs = [d.name for d in family.iterdir() if d.is_dir() and d.name != "static"]
        for s in steps:
            actions += 1
            if s[0] == "suppr":
                print(f"  {family.name} : supprimer {s[1].name} ({mo(s[1].stat().st_size)})")
                if apply:
                    s[1].unlink()
            elif s[0] == "deplacer":
                print(f"  {family.name} : déplacer static/{s[1].name} → {s[2].name}")
                if apply:
                    shutil.move(str(s[1]), str(s[2]))
            else:
                print(f"  {family.name} : supprimer le dossier static/ ({mo(size_of(s[1]))})")
                if apply:
                    shutil.rmtree(s[1])
        if others or others_dirs:
            print(f"  {family.name} : à voir, non cités et laissés : {', '.join(sorted(others + [d + '/' for d in others_dirs]))}")

    # Les chemins cités : données et métadonnées des fichiers de langue.
    texts = list(data_files)
    if locales.is_dir():
        texts += sorted(locales.glob("*/*.ftl"))
    for f in texts:
        text = read_text(f)
        new = text
        for old, rel in moves.items():
            new = new.replace(f'"{old}"', f'"{rel}"')
            new = new.replace(f"@font = {old}", f"@font = {rel}")
        if new != text:
            actions += 1
            print(f"  chemins mis à jour : {f.relative_to(root)}")
            if apply:
                write_text(f, new)

    if not apply:
        print(f"{actions} action(s) prévue(s). Relancez avec --appliquer pour les faire.")
        return
    # Vérification : chaque police citée existe à son nouveau chemin.
    refs_after = set()
    for f in data_files:
        collect_refs(json.loads(read_text(f)), refs_after)
    missing = [r for r in sorted(refs_after) if not (fonts / r).is_file()]
    if locales.is_dir():
        for f in sorted(locales.glob("*/*.ftl")):
            for line in read_text(f).splitlines():
                if line.startswith("# @font = "):
                    rel = line[len("# @font = "):].strip()
                    if rel and not (fonts / rel).is_file():
                        missing.append(f"{rel} ({f.relative_to(root)})")
    after = size_of(fonts)
    print(f"{actions} action(s) faite(s). assets/fonts : {mo(before)} → {mo(after)}.")
    if missing:
        die("polices citées introuvables après coup : " + "; ".join(missing))
    print(f"Vérifié : les {len(refs_after)} polices citées sont présentes.")


if __name__ == "__main__":
    main()
