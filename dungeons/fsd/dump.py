import struct, sys
f=open('/tmp/fsd/Dungeons.fsd','rb').read()
def line(o):
    v=struct.unpack_from('<Q',f,o)[0]
    lo,hi=struct.unpack_from('<II',f,o)
    a,b=struct.unpack_from('<ff',f,o)
    s=''
    if 64<=v<len(f) and v%4==0:
        L=struct.unpack_from('<Q',f,v)[0]
        if 0<L<80 and all(32<=c<127 for c in f[v+8:v+8+L]): s='STR '+f[v+8:v+8+L].decode()
        elif all(32<=c<127 for c in f[v:v+6]): s='TXT '+repr(f[v:v+30])
        else: s='-> '+str(struct.unpack_from('<4Q',f,v))
    return f"{hex(o)}: {v:>20} | {lo:>10} {hi:>10} | {a:>10.4g} {b:>10.4g} {s}"
def dump(o,n):
    for i in range(n): print(line(o+8*i))
if __name__=='__main__':
    dump(int(sys.argv[1],0), int(sys.argv[2]))
