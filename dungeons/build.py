"""Turns the dungeon text files (dungeons/src/*.txt) into the JSON the server
loads (dungeons/*.json), checking every ship name against the game's NPC
types.

    python dungeons/build.py [types.tsv]

types.tsv: typeID, groupID, typeName[, groupName, categoryID] per line (the
game's NPC entity types; dungeons/types.tsv). With none, names are not
checked.

Source files: ded*.txt hold DED complexes (kind "ded", rated 1-10);
missions-*.txt hold agent missions (kind "mission", rated by level).

    = id | name | faction | rating-or-level
    ! objective                                 (missions)
    > room | open|locked | km to the gate (0: last room)
      count names/alternatives @ class          a group of the current wave
      * count name @ class                      an overseer, or the ship whose death triggers a wave
    + count names @ class                       (DED) a wave that comes once the room is clear
    ^ trigger                                   a wave: cleared | killed A/B | attacked A/B | approach

Classes: f frigate, d destroyer, c cruiser, bc battlecruiser, bs battleship,
s a sentry or other fixed defence, ? whatever the game says. A count may be a
range (2-3). A name the game lacks: a ship falls back to its faction's
deadspace ships of its class (an overseer keeps the name as its label); a
sentry, or a ship of unknown class, is left out.
"""

import json
import os
import re
import sys

CLASSES = {"f": "frigate", "d": "destroyer", "c": "cruiser", "bc": "battlecruiser", "bs": "battleship", "s": "sentry"}
HERE = os.path.dirname(os.path.abspath(__file__))
# Names the guides use for things the game calls by faction.
STASIS = {"angel": "Angel Stasis Tower", "blood": "Bloodraider Stasis Tower", "guristas": "Guristas Stasis Tower",
          "sansha": "Sansha Stasis Tower", "serpentis": "Serpentis Stasis Tower", "amarr": "Amarr Stasis Tower",
          "caldari": "Caldari Stasis Tower", "gallente": "Gallente Stasis Tower", "minmatar": "Minmatar Stasis Tower"}


# Guides' names for faction batteries: the game calls Blood Raider ones "Bloodraider".
BATTERY_PREFIX = {"Blood ": "Bloodraider ", "Blood Raider ": "Bloodraider "}


def local_name(name, faction):
    if name in ("Starbase Stasis Tower", "Stasis Tower"):
        return STASIS.get(faction, "Amarr Stasis Tower")
    if name.endswith("Battery"):
        for guide, game in BATTERY_PREFIX.items():
            if name.startswith(guide):
                return game + name[len(guide):]
    return name


def key(name):
    """How names are matched: the game's spacing and capitals vary
    ("Gistum  Centurion", "Corelum Guardian Chief SafeGuard")."""
    return " ".join(name.split()).lower()


def resolve(name, types):
    """The game's own spelling of a name, or None."""
    if types is None:
        return name
    if name in types:
        return name
    return types.get("\0keys", {}).get(key(name))


def load_types(path):
    """name -> (groupName, categoryID); None with no file."""
    if not path or not os.path.exists(path):
        return None
    types = {}
    for line in open(path, encoding="utf-8", errors="replace"):
        parts = line.rstrip("\n").split("\t")
        if len(parts) >= 3:
            types[parts[2]] = (parts[3] if len(parts) > 3 else "", parts[4] if len(parts) > 4 else "")
    keys = {}
    for name in types:
        keys.setdefault(key(name), name)
    types["\0keys"] = keys
    return types


def class_of_group(group):
    g = group.lower()
    for word, cls in (("battlecruiser", "battlecruiser"), ("battleship", "battleship"), ("destroyer", "destroyer"),
                      ("cruiser", "cruiser"), ("frigate", "frigate"), ("sentry", "sentry"), ("tower", "sentry"),
                      ("battery", "sentry"), ("bunker", "sentry")):
        if word in g:
            return cls
    return None


def count_of(text):
    """'3' -> 3; '2-3' -> 3 (the upper bound, rounded from the mean)."""
    m = re.fullmatch(r"(\d+)(?:\s*[-/]\s*(\d+))?", text.strip())
    if not m:
        return None
    lo = int(m.group(1))
    hi = int(m.group(2)) if m.group(2) else lo
    return max(1, round((lo + hi) / 2 + 0.01))


def group(text, types, missing, faction=""):
    """One group line; None if it has to be left out."""
    boss = text.startswith("* ")
    if boss:
        text = text[2:].strip()
    body, _, cls = text.rpartition("@")
    cls = cls.strip().lower()
    count_text, _, names = body.strip().partition(" ")
    count = count_of(count_text)
    if count is None:
        return None
    wanted = [local_name(n.strip(), faction) for n in names.split("/") if n.strip()]
    found = []
    for n in wanted:
        real = resolve(n, types)
        if real is None:
            missing.add(n)
        elif real not in found:
            found.append(real)
    if cls in CLASSES:
        cls = CLASSES[cls]
    else:
        cls = None
    if cls is None and found and types is not None:
        cls = class_of_group(types[found[0]][0])
        if cls is None:
            # A ship the game files under a general group ("Mission Drone",
            # "Rogue Drone"): transports and industrials, which often carry
            # the mission's item, are hauler-sized; anything else is small.
            low = found[0].lower()
            cls = "cruiser" if any(w in low for w in ("transport", "industrial", "hauler")) else "frigate"
    if cls is None:
        # A name the game lacks, with no class given: nothing to fall back on.
        return None
    if cls == "sentry" and not found:
        return None
    g = {"count": count, "class": "frigate" if cls == "sentry" else cls}
    if found:
        g["names"] = found
    if cls == "sentry":
        g["fixed"] = True
    if boss:
        g["boss"] = True
        if not found:
            g["label"] = wanted[0] if wanted else "Overseer"
    return g, wanted


def trigger(text, room_ships, types=None, faction=""):
    """'^ ...' -> (trigger, names), the names as the game spells them."""
    words = text.strip()
    low = words.lower()
    if low.startswith("killed") or low.startswith("attacked"):
        names = [n.strip() for n in words.split(" ", 1)[1].split("/")] if " " in words else []
        names = [n for n in names if n in room_ships]
        names = [resolve(local_name(n, faction), types) or n for n in names]
        return ("killed", names) if names else ("cleared", [])
    if low.startswith("approach"):
        return ("arrive", [])
    return ("cleared", [])


def parse(path, kind, types, missing):
    dungeons, d, room, ships = [], None, None, set()
    for raw in open(path, encoding="utf-8"):
        line = raw.rstrip()
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if stripped.startswith("= "):
            ident, name, faction, rating = [p.strip() for p in stripped[2:].split("|")]
            d = {"id": ident, "name": name, "kind": kind, "faction": faction, "rating": int(rating), "rooms": []}
            dungeons.append(d)
            room = None
        elif stripped.startswith("! "):
            d["objective"] = stripped[2:].strip()
        elif stripped.startswith("> "):
            parts = [p.strip() for p in stripped[2:].split("|")]
            name = parts[0]
            gate = parts[1] if len(parts) > 1 else "open"
            try:
                distance = float(re.sub(r"[^0-9.].*$", "", parts[2])) if len(parts) > 2 and parts[2] else 0.0
            except ValueError:
                distance = 0.0
            room = {"name": name, "gateLocked": gate == "locked", "gateDistance": distance * 1000.0,
                    "waves": [{"trigger": "arrive", "ships": []}]}
            ships = set()
            d["rooms"].append(room)
        elif stripped.startswith("^ "):
            trig, on = trigger(stripped[2:], ships, types, d["faction"])
            wave = {"trigger": trig, "ships": []}
            if on:
                wave["on"] = on
            room["waves"].append(wave)
        elif stripped.startswith("+ "):
            made = group(stripped[2:], types, missing, d["faction"])
            room["waves"].append({"trigger": "cleared", "ships": [made[0]] if made else []})
            if made:
                ships.update(made[1])
        else:
            made = group(stripped, types, missing, d["faction"])
            if made:
                room["waves"][-1]["ships"].append(made[0])
                ships.update(made[1])
    # Empty waves come to nothing: drop them, and rooms left with none at all
    # keep an empty arrival wave so the room still exists (a gate pocket).
    for d in dungeons:
        for room in d["rooms"]:
            room["waves"] = [w for w in room["waves"] if w["ships"]] or [{"trigger": "arrive", "ships": []}]
            if room["waves"][0]["trigger"] != "arrive":
                room["waves"][0]["trigger"] = "arrive"
                room["waves"][0].pop("on", None)
        last = len(d["rooms"]) - 1
        for k, room in enumerate(d["rooms"]):
            if k < last and room["gateDistance"] <= 0:
                room["gateDistance"] = 20000.0
            if k == last:
                room["gateDistance"] = 0.0
    return [d for d in dungeons if any(w["ships"] for r in d["rooms"] for w in r["waves"])]


# ---- DED layouts: CCP's own rooms (fsd/), for the complexes that match --------------------------

ASTEROID_GROUPS = set(range(450, 470))   # ore: no size of their own in the client data
CLOUD_GROUP = 227
OBJECTS_PER_ROOM = 80


def load_layouts():
    """dungeon id -> (CCP layout, its rooms that are ours, how far to trust
    them), from fsd/ded_match.json (fsd/match_ded.py); {} without the files."""
    base = os.path.join(HERE, "fsd")
    try:
        match = json.load(open(os.path.join(base, "ded_match.json"), encoding="utf-8"))
        layouts = {d["dungeonID"]: d for d in json.load(open(os.path.join(base, "fsd_layouts.json"), encoding="utf-8"))}
        kinds = {}
        for line in open(os.path.join(base, "object_types.tsv"), encoding="utf-8"):
            p = line.rstrip("\n").split("\t")
            if len(p) >= 4:
                kinds[int(p[0])] = (int(p[1]), p[2], float(p[3] or 0))
    except OSError:
        return {}, {}
    out = {}
    for ident, info in match.items():
        lay = layouts.get(info.get("dungeonID"))
        if lay and "rooms" in info:
            out[ident] = (lay, info["rooms"], info.get("how"), info.get("exact", False))
    return out, kinds


def apply_layout(d, match, kinds):
    """Gives a complex CCP's gate positions (when its gates line up) and room
    objects (when the layout is exact)."""
    lay, keep, how, exact = match
    if len(keep) != len(d["rooms"]):
        return False
    rooms = [lay["rooms"][i] for i in keep]
    for room, src in zip(d["rooms"], rooms):
        if how == "strict" and src.get("gates"):
            room["gatePos"] = src["gates"][0]["pos"]
        if not exact:
            continue
        objects = []
        for o in src.get("objects", []):
            kind = kinds.get(o["type"])
            if not kind:
                continue
            group, name, radius = kind
            if group in ASTEROID_GROUPS or name == "Beacon":
                continue
            if group == CLOUD_GROUP and radius <= 1:
                radius = 1000.0
            if radius <= 1:
                continue
            objects.append({"typeId": o["type"], "name": o.get("name") or name, "pos": o["pos"], "radius": radius})
        objects.sort(key=lambda o: -o["radius"])
        room["objects"] = objects[:OBJECTS_PER_ROOM]
    d["layout"] = lay["dungeonID"]
    return True


def main():
    types = load_types(sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "types.tsv"))
    src = os.path.join(HERE, "src")
    for name in sorted(os.listdir(src)):
        if not name.endswith(".txt"):
            continue
        kind = "mission" if name.startswith("missions") else "ded"
        missing = set()
        dungeons = parse(os.path.join(src, name), kind, types, missing)
        if kind == "ded":
            layouts, kinds = load_layouts()
            laid = [d["id"] for d in dungeons if d["id"] in layouts and apply_layout(d, layouts[d["id"]], kinds)]
            if laid:
                print(f"  CCP layouts (gates and structures) for {len(laid)}: {', '.join(laid)}")
        out = os.path.join(HERE, name[:-4] + ".json")
        with open(out, "w", encoding="utf-8") as f:
            json.dump(dungeons, f, indent=1, ensure_ascii=False)
            f.write("\n")
        print(f"{name}: {len(dungeons)} {kind}(s) -> {os.path.basename(out)}")
        if missing:
            print(f"  {len(missing)} name(s) not in the game: " + "; ".join(sorted(missing)[:60]))


if __name__ == "__main__":
    main()
