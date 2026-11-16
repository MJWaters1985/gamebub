import os,sys
from gbsdiff import *
def bankmap(g, r):
    img=image(g); res=[]
    nb=(len(img)+0x3FFF)//0x4000
    for b in range(nb):
        lo=max(b*0x4000, g['load']); hi=min((b+1)*0x4000, len(img))
        chunk=img[lo:hi]
        if b==0:
            same=sum(1 for i in range(lo,hi) if i<len(r) and img[i]==r[i]); res.append((0,0,same,hi-lo)); continue
        off=lo-b*0x4000; best=(None,-1)
        for rb in range(len(r)//0x4000):
            rc=r[rb*0x4000+off:rb*0x4000+off+len(chunk)]
            s=sum(1 for x,y in zip(chunk,rc) if x==y)
            if s>best[1]: best=(rb,s)
        res.append((b,best[0],best[1],len(chunk)))
    return res
if __name__=='__main__':
    X=S+'/gbs-dl/x'
    for n in sorted(os.listdir(X)):
        gd=os.path.join(X,n); gp=[os.path.join(gd,f) for f in os.listdir(gd) if f.lower().endswith('.gbs')]
        if not gp: continue
        g=load_gbs(gp[0]); rom=find_rom(n)
        if not rom: print(n[:40],'no ROM'); continue
        r=open(rom,'rb').read()
        print('%-40s'%n[:40], ' '.join('%d->%s %d%%'%(b,('%X'%rb) if rb is not None else '?',100*s//max(1,l)) for b,rb,s,l in bankmap(g,r)))
