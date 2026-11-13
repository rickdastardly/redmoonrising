"""Match dungeons/src/ded.txt complexes to CCP dungeon records (fsd/dungeons.jsonl)
and layouts (fsd/fsd_layouts.json), writing fsd/ded_match.json.

A layout's last room is the entrance outside the complex. The rest line up
with ours when every room but the last has a gate further than 0 km on; a
gateless room in the middle (a side room off the path) may be dropped if
that leaves exactly our rooms. "rooms" in the output: which layout rooms
are ours, in order."""
import json, re
ALIAS = {"Angel Cartel Prisoner Retention": "Cartel Prisoner Retention"}
rows = [json.loads(l) for l in open('fsd/dungeons.jsonl', encoding='utf-8')]
layouts = {d['dungeonID']: d for d in json.load(open('fsd/fsd_layouts.json'))}

def rating(r):
    m = re.search(r'\((\d+) (?:of|out of) 10\)', (r.get('description') or {}).get('en', ''))
    return int(m.group(1)) if m else None

def is_entrance(r):
    """The room outside the complex: just its gate, a kilometre or two off."""
    gates = r.get('gates', [])
    return len(r.get('objects', [])) <= 1 and len(gates) == 1 and gates[0].get('km', 0) <= 2.5


def align(lay, n):
    """Which layout rooms are ours, and how far to trust them: 'strict'
    (gates and structures) when every room but the last has a gate on and
    the last has none; 'objects' (structures only) for an exact layout with
    the right number of rooms whose gates are not usable."""
    idx = [i for i, r in enumerate(lay['rooms']) if not is_entrance(r)]
    if len(idx) != n or lay['rooms'][idx[-1]].get('gates'):
        return None, None
    gated = lambda r: bool(r.get('gates')) and r['gates'][0].get('km', 0) > 0
    if all(gated(lay['rooms'][i]) for i in idx[:-1]):
        return idx, 'strict'
    if lay.get('exact'):
        return idx, 'objects'
    return None, None


ded, cur = [], None
for line in open('src/ded.txt', encoding='utf-8'):
    s = line.strip()
    if s.startswith('= '):
        i, n, f, r = [p.strip() for p in s[2:].split('|')]
        cur = {'id': i, 'name': n, 'rating': int(r), 'rooms': 0}; ded.append(cur)
    elif s.startswith('> ') and cur:
        cur['rooms'] += 1
out = {}
for d in ded:
    name = ALIAS.get(d['name'], d['name'])
    cands = [r for r in rows if r['name']['en'] == name and r['_key'] in layouts]
    best = None
    for r in cands:
        lay = layouts[r['_key']]
        keep, how = align(lay, d['rooms'])
        if keep is None:
            continue
        score = (how == 'strict', lay.get('exact', False), r['archetypeID'] == 24, rating(r) == d['rating'])
        if best is None or score > best[0]:
            best = (score, r['_key'], keep, how, lay.get('exact', False))
    if best:
        _, key, keep, how, exact = best
        lay = layouts[key]
        out[d['id']] = {'dungeonID': key, 'rooms': keep, 'how': how, 'exact': exact,
                        'gates': [[g['km'] for g in lay['rooms'][i].get('gates', [])] for i in keep]}
    print(f"{d['id']:20} ours={d['rooms']} candidates={[r['_key'] for r in cands]} -> {out.get(d['id'], {}).get('dungeonID')} {out.get(d['id'], {}).get('how')} exact={out.get(d['id'], {}).get('exact')} {out.get(d['id'], {}).get('gates')}")
json.dump(out, open('fsd/ded_match.json', 'w'), indent=1)
print(len(out), 'matched of', len(ded))
