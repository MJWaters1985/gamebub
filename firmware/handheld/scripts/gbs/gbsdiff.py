"""GBS rip loading, and matching rips with ROMs (gbsdiff.py prints how each rip differs from its ROM)."""
import struct, glob, os, re, sys
# Work folder (rips in <work>/gbs-dl/x/<set>/, players and results in <work>/gbs/) and the
# folders searched for ROMs to compare with (your own cartridge dumps), from the environment:
#   CART_MUSIC_WORK=<folder>  CART_MUSIC_ROMS=<folder>:<folder>...
S=os.environ.get('CART_MUSIC_WORK', os.path.join(os.path.dirname(os.path.abspath(__file__)), 'work'))
ROMS=[]
for _d in os.environ.get('CART_MUSIC_ROMS', '').split(':'):
    if _d:
        ROMS += sorted(glob.glob(os.path.join(_d, '**', '*.gb*'), recursive=True))

DROP={'version','the','s','special','pikachu','edition'}
def norm(s):
    s=re.sub(r'\[.*?\]|\(.*?\)','',s.lower()); s=s.replace("legend of zelda, the","zelda").replace('&','and')
    return ' '.join(w for w in re.sub(r'[^a-z0-9]+',' ',s).split() if w not in DROP)

def find_rom(gname):
    n=norm(gname); best=None
    for r in ROMS:
        b=os.path.basename(r)
        if 'T-En' in b or '[' in b: continue
        if norm(os.path.splitext(b)[0])==n: return r
    return None

def load_gbs(p):
    d=open(p,'rb').read()
    load,init,play,sp=struct.unpack('<4H',d[6:14])
    return dict(songs=d[4],first=d[5],load=load,init=init,play=play,sp=sp,tma=d[14],tac=d[15],title=d[16:48].rstrip(b'\0').decode('latin1'),data=d[0x70:])

def image(g):
    """GBS address space as {bank:bytes} (bank 0 = $0000-$3FFF)."""
    img=bytes(g['load'])+g['data']
    return img

def diff(g, rom):
    img=image(g); out=[]; i=g['load']
    while i<len(img):
        if i<len(rom) and img[i]==rom[i]: i+=1; continue
        j=i
        while j<len(img) and not (j<len(rom) and img[j]==rom[j] and img[j+1:j+4]==rom[j+1:j+4]): j+=1
        out.append((i,j)); i=j
    return out

if __name__=='__main__':
    X=S+'/gbs-dl/x'
    for gd in sorted(os.path.join(X,n) for n in os.listdir(X)):
        gp=[os.path.join(gd,f) for f in os.listdir(gd) if f.lower().endswith('.gbs')]
        if not gp: continue
        g=load_gbs(gp[0]); name=os.path.basename(gd)
        rom=find_rom(name)
        hdr='%-38s songs %3d load %04X init %04X play %04X sp %04X tac %02X tma %02X'%(name[:38],g['songs'],g['load'],g['init'],g['play'],g['sp'],g['tac'],g['tma'])
        if not rom: print(hdr,' | no ROM'); continue
        r=open(rom,'rb').read()
        d=diff(g,r); tot=sum(b-a for a,b in d)
        print(hdr,' | %s: size %X, %d diff runs, %d bytes: %s'%(os.path.basename(rom)[:20],len(image(g)),len(d),tot,' '.join('%X-%X'%(a,b) for a,b in d[:6])))
