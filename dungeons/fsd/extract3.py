import json, math, numpy as np, collections
f=open('/tmp/fsd/Dungeons.fsd','rb').read()
d=json.load(open('/tmp/fsd/types.json')); types={int(k):v for k,v in d['types'].items()}; groups={int(k):v for k,v in d['groups'].items()}
heads=sorted(json.load(open('/tmp/fsd/heads_all.json')), key=lambda h:h['off'])
a=np.frombuffer(f[:len(f)//4*4],dtype='<u4'); fl=np.frombuffer(f[:len(f)//4*4],dtype='<f4')
tarr=np.array(sorted(types),dtype='<u4')
cand=np.nonzero(np.isin(a,tarr))[0]; cand=cand[cand>12]
objs={}
for i in cand:
    oid=int(a[i-10]); t=int(a[i]); room=int(a[i-1])
    if not (10000<oid<5_000_000) or not (100<room<1_000_000): continue
    # the three words before the id: 250000 / 5000 markers seen in every record
    if not (a[i-19]==1215570944 or a[i-15]==1167867904 or a[i-16]==1167867904): continue
    x,y,z=float(fl[i-6]),float(fl[i-5]),float(fl[i-4])
    if not all(math.isfinite(v) and abs(v)<2e6 for v in (x,y,z)): continue
    if oid in objs: continue
    objs[oid]=dict(off=int(i*4), obj=oid, room=room, type=t, name=types[t][1], group=groups.get(types[t][0],(0,''))[1], pos=[round(x),round(y),round(z)])
objs=list(objs.values())
byroom=collections.defaultdict(list)
for o in objs: byroom[o['room']].append(o)
print('objects', len(objs), 'rooms', len(byroom))
json.dump(objs, open('/tmp/fsd/objects3.json','w'))
