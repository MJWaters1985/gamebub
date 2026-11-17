"""Small SM83 (Game Boy CPU) emulator for tracing GBS rips and players.

Memory: ROM image with MBC-style bank switching (writes to $2000-$3FFF select the $4000 bank),
VRAM, cart RAM, WRAM with CGB banks (SVBK), OAM/IO/HRAM as plain RAM. Records writes,
executed addresses and bank selections for analysis. No interrupts, no PPU timing (LY is
faked as a counter that advances with cycles).
"""


class Halt(Exception):
    pass


class Emu:
    def __init__(self, rom, mbc_write=None):
        self.rom = rom
        self.bank = 1
        self.vram = bytearray(0x2000)
        self.sram = bytearray(0x8000)
        self.wram = [bytearray(0x1000) for _ in range(8)]
        self.svbk = 1
        self.hi = bytearray(0x200)  # FE00-FFFF
        self.a = self.b = self.c = self.d = self.e = self.h = self.l = 0
        self.f = 0
        self.sp = 0xFFFE
        self.pc = 0
        self.cycles = 0
        self.ram_writes = {}     # (region, addr) -> count
        self.ram_reads = set()
        self.executed = set()    # (bank or 0, addr)
        self.bank_writes = []    # (pc, value)
        self.svbk_writes = set()
        self.trace_exec = True
        self.mbc_write = mbc_write
        self.io_writes = {}
        self.rst_base = 0        # GBS rips: rst n calls load + n
        self.rst_used = set()    # rst vectors called (n)
        self.pad = 0xFF          # past the end of the image (GBS rips drop trailing zeros: 0)

    # --- memory ---
    def rd(self, a):
        a &= 0xFFFF
        if a < 0x4000:
            return self.rom[a] if a < len(self.rom) else self.pad
        if a < 0x8000:
            i = self.bank * 0x4000 + a - 0x4000
            return self.rom[i] if i < len(self.rom) else self.pad
        if a < 0xA000:
            return self.vram[a - 0x8000]
        if a < 0xC000:
            self.ram_reads.add(('sram', a))
            return self.sram[a - 0xA000]
        if a < 0xD000:
            self.ram_reads.add(('w0', a))
            return self.wram[0][a - 0xC000]
        if a < 0xE000:
            self.ram_reads.add(('w%d' % self.svbk, a))
            return self.wram[self.svbk][a - 0xD000]
        if a < 0xFE00:
            return self.rd(a - 0x2000)
        if a == 0xFF44:  # LY: advance with time
            return (self.cycles // 456) % 154
        if a == 0xFF00:
            return 0xCF
        if a == 0xFF26:  # sound status: power bit only, all channels reported off
            return (self.hi[0x126] & 0x80) | 0x70
        return self.hi[a - 0xFE00]

    def wr(self, a, v):
        a &= 0xFFFF
        v &= 0xFF
        if a < 0x8000:
            if 0x2000 <= a < 0x4000:
                self.bank_writes.append((self.pc_ins, v))
                self.bank = v if v else 1
                if self.mbc_write:
                    self.bank = self.mbc_write(a, v, self.bank)
            return
        if a < 0xA000:
            self.vram[a - 0x8000] = v
            return
        if a < 0xC000:
            self.ram_writes[('sram', a)] = self.ram_writes.get(('sram', a), 0) + 1
            self.sram[a - 0xA000] = v
            return
        if a < 0xD000:
            self.ram_writes[('w0', a)] = self.ram_writes.get(('w0', a), 0) + 1
            self.wram[0][a - 0xC000] = v
            return
        if a < 0xE000:
            k = ('w%d' % self.svbk, a)
            self.ram_writes[k] = self.ram_writes.get(k, 0) + 1
            self.wram[self.svbk][a - 0xD000] = v
            return
        if a < 0xFE00:
            return self.wr(a - 0x2000, v)
        if a == 0xFF70:
            self.svbk = (v & 7) or 1
            self.svbk_writes.add(v)
        if 0xFF80 <= a < 0xFFFF:
            self.ram_writes[('hram', a)] = self.ram_writes.get(('hram', a), 0) + 1
        if 0xFF00 <= a < 0xFF80:
            self.io_writes[a] = v
        self.hi[a - 0xFE00] = v

    def rd16(self, a):
        return self.rd(a) | self.rd(a + 1) << 8

    def fetch(self):
        v = self.rd(self.pc)
        self.pc = (self.pc + 1) & 0xFFFF
        return v

    def fetch16(self):
        v = self.fetch()
        return v | self.fetch() << 8

    def push(self, v):
        self.sp = (self.sp - 1) & 0xFFFF
        self.wr(self.sp, v >> 8)
        self.sp = (self.sp - 1) & 0xFFFF
        self.wr(self.sp, v)

    def pop(self):
        v = self.rd(self.sp) | self.rd(self.sp + 1) << 8
        self.sp = (self.sp + 2) & 0xFFFF
        return v

    # --- registers ---
    def get_r(self, i):
        return (self.b, self.c, self.d, self.e, self.h, self.l, None, self.a)[i] if i != 6 else self.rd(self.hl)

    def set_r(self, i, v):
        v &= 0xFF
        if i == 0: self.b = v
        elif i == 1: self.c = v
        elif i == 2: self.d = v
        elif i == 3: self.e = v
        elif i == 4: self.h = v
        elif i == 5: self.l = v
        elif i == 6: self.wr(self.hl, v)
        else: self.a = v

    hl = property(lambda s: s.h << 8 | s.l, lambda s, v: (setattr(s, 'h', v >> 8 & 0xFF), setattr(s, 'l', v & 0xFF)))
    bc = property(lambda s: s.b << 8 | s.c, lambda s, v: (setattr(s, 'b', v >> 8 & 0xFF), setattr(s, 'c', v & 0xFF)))
    de = property(lambda s: s.d << 8 | s.e, lambda s, v: (setattr(s, 'd', v >> 8 & 0xFF), setattr(s, 'e', v & 0xFF)))
    af = property(lambda s: s.a << 8 | s.f, lambda s, v: (setattr(s, 'a', v >> 8 & 0xFF), setattr(s, 'f', v & 0xF0)))

    def get_rp(self, i):
        return (self.bc, self.de, self.hl, self.sp)[i]

    def set_rp(self, i, v):
        v &= 0xFFFF
        if i == 0: self.bc = v
        elif i == 1: self.de = v
        elif i == 2: self.hl = v
        else: self.sp = v

    def flags(self, z, n, h, c):
        self.f = (0x80 if z else 0) | (0x40 if n else 0) | (0x20 if h else 0) | (0x10 if c else 0)

    fz = property(lambda s: bool(s.f & 0x80))
    fc = property(lambda s: bool(s.f & 0x10))

    def cond(self, y):
        return (not self.fz, self.fz, not self.fc, self.fc)[y]

    def alu(self, y, v):
        a = self.a
        if y == 0:  # add
            r = a + v; self.flags((r & 0xFF) == 0, 0, (a & 15) + (v & 15) > 15, r > 255); self.a = r & 0xFF
        elif y == 1:  # adc
            c = 1 if self.fc else 0; r = a + v + c
            self.flags((r & 0xFF) == 0, 0, (a & 15) + (v & 15) + c > 15, r > 255); self.a = r & 0xFF
        elif y == 2:  # sub
            r = a - v; self.flags((r & 0xFF) == 0, 1, (a & 15) < (v & 15), r < 0); self.a = r & 0xFF
        elif y == 3:  # sbc
            c = 1 if self.fc else 0; r = a - v - c
            self.flags((r & 0xFF) == 0, 1, (a & 15) < (v & 15) + c, r < 0); self.a = r & 0xFF
        elif y == 4:
            self.a = a & v; self.flags(self.a == 0, 0, 1, 0)
        elif y == 5:
            self.a = a ^ v; self.flags(self.a == 0, 0, 0, 0)
        elif y == 6:
            self.a = a | v; self.flags(self.a == 0, 0, 0, 0)
        else:
            r = a - v; self.flags((r & 0xFF) == 0, 1, (a & 15) < (v & 15), r < 0)

    def step(self):
        self.pc_ins = self.pc
        if self.trace_exec:
            self.executed.add((self.bank if self.pc >= 0x4000 and self.pc < 0x8000 else 0, self.pc))
        op = self.fetch()
        self.cycles += 4
        x, y, z = op >> 6, (op >> 3) & 7, op & 7
        if x == 1:
            if op == 0x76:
                raise Halt()
            self.set_r(y, self.get_r(z)); return
        if x == 2:
            self.alu(y, self.get_r(z)); return
        if x == 0:
            if z == 0:
                if y == 0: return
                if y == 1:
                    a = self.fetch16(); self.wr(a, self.sp & 0xFF); self.wr(a + 1, self.sp >> 8); return
                if y == 2:
                    self.fetch(); return  # stop
                d = self.fetch(); d = d - 256 if d > 127 else d
                if y == 3 or self.cond(y - 4):
                    self.pc = (self.pc + d) & 0xFFFF
                return
            if z == 1:
                if y % 2 == 0:
                    self.set_rp(y >> 1, self.fetch16())
                else:
                    hl = self.hl; v = self.get_rp(y >> 1); r = hl + v
                    self.f = (self.f & 0x80) | (0x20 if (hl & 0xFFF) + (v & 0xFFF) > 0xFFF else 0) | (0x10 if r > 0xFFFF else 0)
                    self.hl = r & 0xFFFF
                return
            if z == 2:
                p = y >> 1
                if y % 2 == 0:
                    addr = (self.bc, self.de, self.hl, self.hl)[p]; self.wr(addr, self.a)
                else:
                    addr = (self.bc, self.de, self.hl, self.hl)[p]; self.a = self.rd(addr)
                if p == 2: self.hl = (self.hl + 1) & 0xFFFF
                if p == 3: self.hl = (self.hl - 1) & 0xFFFF
                return
            if z == 3:
                self.set_rp(y >> 1, self.get_rp(y >> 1) + (1 if y % 2 == 0 else -1)); return
            if z == 4:
                v = self.get_r(y); r = (v + 1) & 0xFF; self.set_r(y, r)
                self.f = (self.f & 0x10) | (0x80 if r == 0 else 0) | (0x20 if (v & 15) == 15 else 0); return
            if z == 5:
                v = self.get_r(y); r = (v - 1) & 0xFF; self.set_r(y, r)
                self.f = (self.f & 0x10) | 0x40 | (0x80 if r == 0 else 0) | (0x20 if (v & 15) == 0 else 0); return
            if z == 6:
                self.set_r(y, self.fetch()); return
            # z == 7
            a = self.a; c = 1 if self.fc else 0
            if y == 0: self.a = ((a << 1) | (a >> 7)) & 0xFF; self.flags(0, 0, 0, a >> 7)
            elif y == 1: self.a = ((a >> 1) | (a << 7)) & 0xFF; self.flags(0, 0, 0, a & 1)
            elif y == 2: self.a = ((a << 1) | c) & 0xFF; self.flags(0, 0, 0, a >> 7)
            elif y == 3: self.a = ((a >> 1) | (c << 7)) & 0xFF; self.flags(0, 0, 0, a & 1)
            elif y == 4:  # daa
                n = self.f & 0x40; h = self.f & 0x20; cf = self.fc
                if not n:
                    if cf or a > 0x99: a += 0x60; cf = True
                    if h or (a & 0xF) > 9: a += 6
                else:
                    if cf: a -= 0x60
                    if h: a -= 6
                a &= 0xFF; self.a = a; self.f = (0x80 if a == 0 else 0) | n | (0x10 if cf else 0)
            elif y == 5: self.a = a ^ 0xFF; self.f |= 0x60
            elif y == 6: self.f = (self.f & 0x80) | 0x10
            else: self.f = (self.f & 0x80) | (0 if self.fc else 0x10)
            return
        # x == 3
        if z == 0:
            if y < 4:
                if self.cond(y): self.pc = self.pop()
                return
            if y == 4: self.wr(0xFF00 + self.fetch(), self.a); return
            if y == 5:
                d = self.fetch(); d = d - 256 if d > 127 else d; sp = self.sp
                self.flags(0, 0, (sp & 15) + (d & 15) > 15, (sp & 0xFF) + (d & 0xFF) > 0xFF); self.sp = (sp + d) & 0xFFFF; return
            if y == 6: self.a = self.rd(0xFF00 + self.fetch()); return
            d = self.fetch(); d = d - 256 if d > 127 else d; sp = self.sp
            self.flags(0, 0, (sp & 15) + (d & 15) > 15, (sp & 0xFF) + (d & 0xFF) > 0xFF); self.hl = (sp + d) & 0xFFFF; return
        if z == 1:
            if y % 2 == 0:
                v = self.pop(); p = y >> 1
                if p == 3: self.af = v
                else: self.set_rp(p, v)
                return
            if y == 1: self.pc = self.pop(); return
            if y == 3: self.pc = self.pop(); return  # reti
            if y == 5: self.pc = self.hl; return
            if y == 7: self.sp = self.hl; return
        if z == 2:
            if y < 4:
                a = self.fetch16()
                if self.cond(y): self.pc = a
                return
            if y == 4: self.wr(0xFF00 + self.c, self.a); return
            if y == 5: self.wr(self.fetch16(), self.a); return
            if y == 6: self.a = self.rd(0xFF00 + self.c); return
            self.a = self.rd(self.fetch16()); return
        if z == 3:
            if y == 0: self.pc = self.fetch16(); return
            if y == 1: return self.cb()
            return  # di/ei (6,7), invalid others
        if z == 4:
            a = self.fetch16()
            if y < 4 and self.cond(y):
                self.push(self.pc); self.pc = a
            return
        if z == 5:
            if y % 2 == 0:
                p = y >> 1; self.push(self.af if p == 3 else self.get_rp(p)); return
            if y == 1:
                a = self.fetch16(); self.push(self.pc); self.pc = a; return
            return
        if z == 6:
            self.alu(y, self.fetch()); return
        self.rst_used.add(y * 8)
        self.push(self.pc); self.pc = self.rst_base + y * 8

    def cb(self):
        op = self.fetch()
        x, y, z = op >> 6, (op >> 3) & 7, op & 7
        v = self.get_r(z)
        if x == 0:
            c = 1 if self.fc else 0
            if y == 0: r = ((v << 1) | (v >> 7)) & 0xFF; cf = v >> 7
            elif y == 1: r = ((v >> 1) | (v << 7)) & 0xFF; cf = v & 1
            elif y == 2: r = ((v << 1) | c) & 0xFF; cf = v >> 7
            elif y == 3: r = ((v >> 1) | (c << 7)) & 0xFF; cf = v & 1
            elif y == 4: r = (v << 1) & 0xFF; cf = v >> 7
            elif y == 5: r = (v >> 1) | (v & 0x80); cf = v & 1
            elif y == 6: r = ((v << 4) | (v >> 4)) & 0xFF; cf = 0
            else: r = v >> 1; cf = v & 1
            self.set_r(z, r); self.flags(r == 0, 0, 0, cf)
        elif x == 1:
            self.f = (self.f & 0x10) | 0x20 | (0 if v & (1 << y) else 0x80)
        elif x == 2:
            self.set_r(z, v & ~(1 << y))
        else:
            self.set_r(z, v | (1 << y))

    def call(self, addr, limit=2_000_000):
        """Call a routine until it returns to the sentinel. Returns steps used."""
        sentinel = 0xFFF0
        self.push(sentinel)
        self.pc = addr
        n = 0
        while self.pc != sentinel:
            self.step()
            n += 1
            if n > limit:
                raise RuntimeError('routine at %04X did not return (pc %04X bank %X)' % (addr, self.pc, self.bank))
        return n
