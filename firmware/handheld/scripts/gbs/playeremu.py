"""Run a generated player (boot image) over a ROM in the SM83 emulator: playeremu.py player.bin rom [steps]"""
import sys
from collections import Counter
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from sm83emu import Emu, Halt

class PlayerEmu(Emu):
    def __init__(s, rom, boot):
        super().__init__(rom); s.boot=boot; s.boot_on=True; s.tima_acc=0; s.snd=0
        s.trace_exec=False
    def rd(s,a):
        a&=0xFFFF
        if s.boot_on and (a<0x100 or 0x200<=a<0x900): return s.boot[a]
        if a==0xFF26: return (s.hi[0x126]&0x80)|0x70
        return super().rd(a)
    def wr(s,a,v):
        a&=0xFFFF
        if a==0xFF50: s.boot_on=False
        if 0xFF10<=a<0xFF40: s.snd+=1
        if a<0x2000 and (v&0xF)==0xA: pass
        super().wr(a,v)
    def step(s):
        c0=s.cycles; super().step(); dc=s.cycles-c0
        tac=s.hi[0x107]
        if tac&4:
            div={0:1024,1:16,2:64,3:256}[tac&3]
            s.tima_acc+=dc
            while s.tima_acc>=div:
                s.tima_acc-=div; t=s.hi[0x105]+1
                if t>0xFF: t=s.hi[0x106]; s.hi[0x10F]|=4
                s.hi[0x105]=t

if __name__=='__main__':
    boot=open(sys.argv[1],'rb').read(); rom=open(sys.argv[2],'rb').read()
    e=PlayerEmu(rom,boot); e.pc=0; n=int(sys.argv[3]) if len(sys.argv)>3 else 3_000_000
    pcs=Counter()
    for i in range(n):
        try: e.step()
        except Halt: print('halt at %04X'%e.pc); break
        if i>n-20000: pcs[(e.bank if 0x4000<=e.pc<0x8000 else 0, e.pc)]+=1
    print('sound writes', e.snd, 'boot_on', e.boot_on, 'cycles', e.cycles)
    print('hot pcs at the end:', ' '.join('%X:%04X'%k for k,_ in pcs.most_common(12)))
