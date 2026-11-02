#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Synthetic Skyrim LE plugin with dialogue (lot 4 fixture).

Writes tests/fixtures/skyrim_dialogue.esp: a quest with aliases, stages and
an objective, a scene with two phases, four topics (custom, custom, hello,
scene) with seven INFO replies, three NPCs, two voice types, a faction, and
the conditions the speaker resolution (C-02) reads. Uncompressed, non
localized, header of 24 bytes, master Skyrim.esm (index 00); the plugin's
own forms use index 01.
"""
import struct, os

OUT = os.path.join(os.path.dirname(__file__), "..", "tests", "fixtures", "skyrim_dialogue.esp")

def sub(sig, data):
    return sig.encode() + struct.pack("<H", len(data)) + data

def z(s):
    return s.encode("utf-8") + b"\0"

def u32(v): return struct.pack("<I", v)
def u16(v): return struct.pack("<H", v)
def f32(v): return struct.pack("<f", v)

def rec(sig, formid, subs, flags=0):
    body = b"".join(subs)
    return sig.encode() + struct.pack("<IIIHH", len(body), flags, formid, 0, 0) + struct.pack("<HH", 43, 0) + body

def grup(label, gtype, body):
    return b"GRUP" + struct.pack("<I", 24 + len(body)) + label + struct.pack("<iHHHH", gtype, 0, 0, 0, 0) + body

def top(sig, body):
    return grup(sig.encode(), 0, body)

def ctda(func, p1=0, p2=0, op=0, flags=0, value=1.0, run_on=0, ref=0, p3=-1):
    t = (op << 5) | flags
    return sub("CTDA", bytes([t, 0, 0, 0]) + f32(value) + u16(func) + u16(0) + u32(p1) + u32(p2) + u32(run_on) + u32(ref) + struct.pack("<i", p3))

def trdt(emotion, value, number, note=None):
    return sub("TRDT", u32(emotion) + u32(value) + struct.pack("<i", 0) + bytes([number, 0, 0, 0]) + u32(0) + bytes([0, 0, 0, 0]))

# ---- form ids (index 01 = this plugin)
NPC_RALOF, NPC_LOKIR, NPC_GERDUR = 0x01000A01, 0x01000A02, 0x01000A03
NPC_LYDIA_MASTER = 0x000A2C8E  # in Skyrim.esm, unresolved without the master
VT_MALENORD, VT_FEMALENORD = 0x01000B01, 0x01000B02
FACT_GUARDS = 0x01000C01
Q_MQ101 = 0x01000101
T_RALOF1, T_RALOF2, T_GERDURHELLO, T_SCENE, T_SCENE2 = 0x01000201, 0x01000202, 0x01000203, 0x01000204, 0x01000205
I1, I2, I3, I4, I5, I6, I7 = 0x01000301, 0x01000302, 0x01000303, 0x01000304, 0x01000305, 0x01000306, 0x01000307
SC_INTRO = 0x01000401
GLOB_X = 0x01000D01
WEAP_AXE, MESG_HELGEN = 0x01000E01, 0x01000F01
NPC_HADVAR_OVERRIDE = 0x0002BF9D  # an override of a Skyrim.esm record (C-03 « maître »)

F_GETISID, F_GETSTAGE, F_GETISSEX, F_GETISALIASREF, F_GETISVOICETYPE, F_GETINFACTION, F_HASKEYWORD = 72, 58, 70, 566, 426, 71, 560

parts = []
# TES4
hedr = sub("HEDR", f32(1.7) + u32(30) + u32(0x01000E00))
parts.append(rec("TES4", 0, [hedr, sub("CNAM", z("XIMOD")), sub("SNAM", z("Lot 4 dialogue fixture")), sub("MAST", z("Skyrim.esm")), sub("DATA", bytes(8)), sub("INTV", u32(1))]))

# GLOB
parts.append(top("GLOB", rec("GLOB", GLOB_X, [sub("EDID", z("MQ101Progress")), sub("FNAM", b"s"), sub("FLTV", f32(0.0))])))
# VTYP
parts.append(top("VTYP", rec("VTYP", VT_MALENORD, [sub("EDID", z("MaleNord")), sub("DNAM", bytes([0]))]) + rec("VTYP", VT_FEMALENORD, [sub("EDID", z("FemaleNord")), sub("DNAM", bytes([1]))])))
# FACT
parts.append(top("FACT", rec("FACT", FACT_GUARDS, [sub("EDID", z("GuardFaction")), sub("FULL", z("Whiterun Guards")), sub("DATA", u32(0))])))
# NPC_
def npc(fid, edid, full, female, voice):
    acbs = u32(0x1 if female else 0) + u16(0) * 10  # flags + 20 bytes
    return rec("NPC_", fid, [sub("EDID", z(edid)), sub("FULL", z(full)), sub("ACBS", acbs), sub("VTCK", u32(voice)), sub("RNAM", u32(0x00013746))])
parts.append(top("NPC_", npc(NPC_RALOF, "Ralof", "Ralof", False, VT_MALENORD) + npc(NPC_LOKIR, "Lokir", "Lokir", False, VT_MALENORD) + npc(NPC_GERDUR, "Gerdur", "Gerdur", True, VT_FEMALENORD) + npc(NPC_HADVAR_OVERRIDE, "Hadvar", "Hadvar", False, VT_MALENORD)))
# WEAP with a model, MESG owned by the quest (C-03)
parts.append(top("WEAP", rec("WEAP", WEAP_AXE, [sub("EDID", z("RalofAxe")), sub("FULL", z("Ralof's Axe")), sub("MODL", z("Weapons\\Iron\\IronWarAxe.nif")), sub("DATA", u32(10) + f32(1.0) + u16(8))])))
parts.append(top("MESG", rec("MESG", MESG_HELGEN, [sub("EDID", z("MQ101HelgenMsg")), sub("DESC", z("Helgen burns.")), sub("FULL", z("Helgen")), sub("QNAM", u32(Q_MQ101)), sub("DNAM", u32(1))])))
# QUST
qust = [sub("EDID", z("MQ101")), sub("FULL", z("Unbound")), sub("DNAM", u16(0x11) + bytes([50, 0]) + struct.pack("<i", 0) + u32(0)),
        sub("NEXT", b""),
        sub("INDX", u16(10) + bytes([0, 0])), sub("CNAM", z("I woke up in a cart with Ralof.")),
        sub("INDX", u16(20) + bytes([0, 0])), sub("CNAM", z("I escaped Helgen.")), sub("CNAM", z("Ralof led me to Riverwood.")),
        sub("QOBJ", u16(10)), sub("FNAM", u32(0)), sub("NNAM", z("Follow Ralof")),
        sub("ANAM", u32(3)),
        sub("ALST", u32(0)), sub("ALID", z("Ralof")), sub("FNAM", u32(0)), sub("ALUA", u32(NPC_RALOF)), sub("ALED", b""),
        sub("ALST", u32(1)), sub("ALID", z("Lokir")), sub("FNAM", u32(0)), sub("ALUA", u32(NPC_LOKIR)), sub("ALED", b""),
        sub("ALST", u32(2)), sub("ALID", z("Companion")), sub("FNAM", u32(0)), sub("ALED", b"")]
parts.append(top("QUST", rec("QUST", Q_MQ101, qust)))
# DIAL + INFO
def dial(fid, edid, full, quest, category, subtype, snam, infos):
    subs = [sub("EDID", z(edid))]
    if full:
        subs.append(sub("FULL", z(full)))
    subs += [sub("PNAM", f32(50.0))]
    if quest:
        subs.append(sub("QNAM", u32(quest)))
    subs += [sub("DATA", bytes([0, category]) + u16(subtype)), sub("SNAM", snam.encode()), sub("TIFC", u32(len(infos)))]
    body = b"".join(infos)
    return rec("DIAL", fid, subs) + grup(u32(fid), 7, body)

def info(fid, subs):
    return rec("INFO", fid, [sub("ENAM", u16(0) + u16(0))] + subs)

i1 = info(I1, [sub("CNAM", bytes([0])), sub("TCLT", u32(T_RALOF2)),
               trdt(0, 50, 1), sub("NAM1", z("We're being taken to Helgen.")), sub("NAM2", z("said quietly, resigned")),
               trdt(3, 40, 2), sub("NAM1", z("Don't worry, I'm sure it's a mistake.")),
               ctda(F_GETISID, NPC_RALOF), ctda(F_GETSTAGE, Q_MQ101, op=3, value=20.0),
               sub("ANAM", u32(NPC_RALOF))])
i2 = info(I2, [sub("PNAM", u32(I1)), trdt(1, 60, 1), sub("NAM1", z("Shut up back there!")),
               ctda(F_GETISID, NPC_LOKIR, flags=0x01), ctda(F_GETISSEX, 1, flags=0x01, run_on=1)])
i3 = info(I3, [trdt(0, 0, 1), sub("NAM1", z("Hey, you. You're finally awake.")), sub("RNAM", z("Hello?"))])
i4 = info(I4, [trdt(0, 0, 1), sub("NAM1", z("Helgen. The headsman is waiting.")), ctda(F_GETISALIASREF, 0, flags=0x02), ctda(F_HASKEYWORD, 0x0001F3A0, run_on=1)])
i5 = info(I5, [trdt(5, 30, 1), sub("NAM1", z("Welcome to Riverwood.")), sub("ANAM", u32(NPC_GERDUR))])
i6 = info(I6, [trdt(0, 0, 1), sub("NAM1", z("You were trying to cross the border, right?"))])
i7 = info(I7, [trdt(1, 70, 1), sub("NAM1", z("Damn you Stormcloaks. Skyrim was fine until you came along.")), ctda(F_GETISID, NPC_LYDIA_MASTER)])
dials = dial(T_RALOF1, "MQ101RalofTopic01", "What's going on?", Q_MQ101, 0, 0, "CUST", [i1, i2, i3]) \
      + dial(T_RALOF2, "MQ101RalofTopic02", "Where are we going?", Q_MQ101, 0, 0, "CUST", [i4]) \
      + dial(T_GERDURHELLO, "DialogueGerdurHello", "", None, 7, 82, "HELO", [i5]) \
      + dial(T_SCENE, "MQ101IntroSceneTopic01", "", Q_MQ101, 2, 0, "SCEN", [i6]) \
      + dial(T_SCENE2, "MQ101IntroSceneTopic02", "", Q_MQ101, 2, 0, "SCEN", [i7])
parts.append(top("DIAL", dials))
# SCEN
scen = [sub("EDID", z("MQ101IntroScene")), sub("FNAM", u32(0)),
        sub("HNAM", b""), sub("NAM0", z("Ralof talks")), sub("NEXT", b""), sub("NEXT", b""), sub("WNAM", u32(100)), sub("HNAM", b""),
        sub("HNAM", b""), sub("NAM0", z("Lokir answers")), sub("NEXT", b""), sub("NEXT", b""), sub("WNAM", u32(100)), sub("HNAM", b""),
        sub("ALID", u32(0)), sub("LNAM", u32(0)), sub("DNAM", u32(0)),
        sub("ALID", u32(1)), sub("LNAM", u32(0)), sub("DNAM", u32(0)),
        sub("ANAM", u16(0)), sub("NAM0", z("Ralof line")), sub("ALID", u32(0)), sub("LNAM", u32(0)), sub("INAM", u32(1)), sub("FNAM", u32(0)), sub("SNAM", u32(0)), sub("ENAM", u32(0)), sub("DATA", u32(T_SCENE)), sub("ANAM", b""),
        sub("ANAM", u16(0)), sub("NAM0", z("Lokir line")), sub("ALID", u32(1)), sub("LNAM", u32(0)), sub("INAM", u32(2)), sub("FNAM", u32(0)), sub("SNAM", u32(1)), sub("ENAM", u32(1)), sub("DATA", u32(T_SCENE2)), sub("ANAM", b""),
        sub("PNAM", u32(Q_MQ101)), sub("INAM", u32(2))]
parts.append(top("SCEN", rec("SCEN", SC_INTRO, scen)))

data = b"".join(parts)
os.makedirs(os.path.dirname(OUT), exist_ok=True)
with open(OUT, "wb") as f:
    f.write(data)
print(OUT, len(data), "bytes")
