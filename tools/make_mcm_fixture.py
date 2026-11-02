#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""MCM interface-file fixture (lot 4, C-05).

Writes tests/fixtures/mcm/Data/: a copy of the dialogue plugin as
MonMod.esp and Interface/Translations/MonMod_english.txt (UTF-16LE with
mark, CRLF, 20 keys, 3 « ; » comments, 2 empty lines, one repeated key)
plus MonMod_french.txt (17 keys translated, one key the source lacks).
"""
import os, shutil

HERE = os.path.dirname(__file__)
ROOT = os.path.join(HERE, "..", "tests", "fixtures")
DATA = os.path.join(ROOT, "mcm", "Data")
TR = os.path.join(DATA, "Interface", "Translations")
os.makedirs(TR, exist_ok=True)
shutil.copyfile(os.path.join(ROOT, "skyrim_dialogue.esp"), os.path.join(DATA, "MonMod.esp"))

keys = [
    ("$MonMod_Title", "My Mod Settings", "Réglages de mon mod"),
    ("$MonMod_General", "General", "Général"),
    ("$MonMod_Enable", "Enable the mod", "Activer le mod"),
    ("$MonMod_Enable_Help", "Turns the whole mod on or off.", "Active ou désactive tout le mod."),
    ("$MonMod_Hotkey", "Hotkey", "Touche"),
    ("$MonMod_Hotkey_Help", "Press {0} to open the menu.", "Appuyez sur {0} pour ouvrir le menu."),
    ("$MonMod_Level", "Level {0} of {1}", "Niveau {0} sur {1}"),
    ("$MonMod_Items", "%s items found", "%s objets trouvés"),
    ("$MonMod_Reset", "Reset to defaults", "Rétablir les valeurs par défaut"),
    ("$MonMod_Reset_Confirm", "Reset all settings?", "Rétablir tous les réglages ?"),
    ("$MonMod_Yes", "Yes", "Oui"),
    ("$MonMod_No", "No", "Non"),
    ("$MonMod_Volume", "Volume", "Volume"),
    ("$MonMod_Volume_Help", "Sound volume of the mod, from 0 to 100.", "Volume sonore du mod, de 0 à 100."),
    ("$MonMod_Debug", "Debug messages", None),
    ("$MonMod_Debug_Help", "Shows the mod's messages in the console.", None),
    ("$MonMod_Version", "Version", "Version"),
    ("$MonMod_Author", "Author", "Auteur"),
    ("$MonMod_Thanks", "Thank you for playing!", None),
    ("$MonMod_Yes", "Yes!", "Oui !"),
]
eng = ["; MonMod — interface strings", "; SkyUI MCM"]
fra = ["; MonMod — textes d'interface"]
for i, (k, en, fr) in enumerate(keys):
    if i == 5:
        eng.append("")
    if i == 12:
        eng.append("; sound")
        eng.append("")
    eng.append(f"{k}\t{en}")
    if fr is not None:
        fra.append(f"{k}\t{fr}")
fra.append("$MonMod_OldKey\tAncienne clé")

def write(path, lines):
    text = "\r\n".join(lines) + "\r\n"
    with open(path, "wb") as f:
        f.write(b"\xff\xfe" + text.encode("utf-16-le"))

write(os.path.join(TR, "MonMod_english.txt"), eng)
write(os.path.join(TR, "MonMod_french.txt"), fra)
print(TR, len(keys), "keys")
