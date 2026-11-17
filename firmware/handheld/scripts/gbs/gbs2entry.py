"""Make a cartridge music entry from a GBS rip (prototype).

gbs2entry.py <gbs folder> [rom]   -> prints a report; as a module, convert() returns the entry.

A GBS rip holds the game's sound engine banks (mostly unchanged ROM banks, renumbered), a small
ripper stub (init/play routines, usually in bank 0 at the load address) and playlists with
track names. On the cartridge the engine is in its original banks and the stub's addresses
hold game code, so the ripper's code and data (the bytes that differ from the cartridge, or
everything from the load address in bank 0 when there's no ROM to compare with) are moved to
work RAM with their absolute references patched, and their bank switches go through a table
from rip banks to ROM banks.

Addresses here are offsets in the rip's image ("lin"): bank 0 at 0-$3FFF, bank n at n*$4000.
"""
import os, re, sys

sys.path.insert(0, os.path.dirname(__file__))
from sm83emu import Emu
from gbsdiff import load_gbs, image
from banks import bankmap

ABS16 = {0x01, 0x11, 0x21, 0x31, 0x08, 0xC2, 0xC3, 0xC4, 0xCA, 0xCC, 0xCD, 0xD2, 0xD4, 0xDA, 0xDC, 0xEA, 0xFA}
JR = {0x18, 0x20, 0x28, 0x30, 0x38}
IMM8 = {0x06, 0x0E, 0x16, 0x1E, 0x26, 0x2E, 0x36, 0x3E, 0xC6, 0xCE, 0xD6, 0xDE, 0xE6, 0xEE, 0xF6, 0xFE,
        0xE0, 0xF0, 0xE8, 0xF8, 0x10}
NO_FALLTHROUGH = {0xC3, 0xC9, 0xD9, 0x18, 0xE9}


def ins_len(op):
    if op == 0xCB:
        return 2
    if op in ABS16:
        return 3
    if op in JR or op in IMM8:
        return 2
    return 1


def lin(bank, addr):
    return addr if addr < 0x4000 else bank * 0x4000 + addr - 0x4000


class TraceEmu(Emu):
    """Records what runs and is read, bank switches, and which constants in the ripper's code
    (`ld r,n` with n from `tainted(L)`) end up written to the bank register ("taint" follows
    values through registers, memory, the stack and arithmetic)."""

    def __init__(self, img, source=lambda L: False):
        super().__init__(img)
        self.trace_exec = False
        self.fetching = False
        self.ins = {}       # lin -> (bytes, bank at execution)
        self.reads = set()  # lin of data reads from ROM
        self.bankw = {}     # lin of the instruction -> set of values written to $2000-$3FFF
        self.cur = None
        self.source = source
        self.rt = {}        # register -> frozenset of source constants
        self.mt = {}        # memory address -> frozenset
        self.wt = frozenset()
        self.bank_sources = set()
        self.bank_untracked = set()   # bank switches with a value from no known source
        self.jphl = set()   # jp hl: (lin of the jp, lin of the target, sources of h)

    def fetch(self):
        self.fetching = True
        v = super().fetch()
        self.fetching = False
        return v

    def rd(self, a):
        a &= 0xFFFF
        if not self.fetching and a < 0x8000:
            self.reads.add(lin(self.bank, a))
        return super().rd(a)

    def _addr_taint(self, a):
        a &= 0xFFFF
        if a < 0x8000:   # data in the ripper's code is a source; elsewhere ('d', L) marks original data
            L = lin(self.bank, a)
            return frozenset({L}) if self.source(L) else frozenset({('d', L)})
        return self.mt.get(a, frozenset())

    def step(self):
        pc = self.pc
        if pc < 0x8000:
            L = lin(self.bank, pc)
            if L not in self.ins:
                op = self.rom[L] if L < len(self.rom) else 0
                self.ins[L] = (bytes(self.rom[L:L + ins_len(op)]), self.bank)
            self.cur = L
        else:
            self.cur = None
        # --- taint: work out what this instruction moves, before running it ---
        R = 'bcdehlma'
        rt, E = self.rt, frozenset()
        g = lambda r: rt.get(r, E)
        op = self.rd(pc)
        x, y, z = op >> 6, (op >> 3) & 7, op & 7
        hl = self.hl
        if op == 0xE9:
            self.jphl.add((self.cur, lin(self.bank, hl) if hl < 0x8000 else None, g('h')))
        after = None
        self.wt = E
        if x == 1 and op != 0x76:
            src = self._addr_taint(hl) if z == 6 else g(R[z])
            if y == 6:
                self.wt = src
            else:
                after = (R[y], src)
        elif x == 0 and z == 6:                       # ld r,n
            imm = self._addr_taint(pc + 1)
            if y == 6:
                self.wt = imm
            else:
                after = (R[y], imm)
        elif op in (0x02, 0x12, 0x22, 0x32, 0xEA, 0xE0, 0xE2):
            self.wt = g('a')
        elif op in (0x0A, 0x1A, 0x2A, 0x3A):
            a = (self.bc, self.de, hl, hl)[(op >> 4) & 3]
            after = ('a', self._addr_taint(a))
        elif op == 0xFA:
            after = ('a', self._addr_taint(self.rd16(pc + 1)))
        elif op == 0xF0:
            after = ('a', self._addr_taint(0xFF00 + self.rd(pc + 1)))
        elif op == 0xF2:
            after = ('a', self._addr_taint(0xFF00 + self.c))
        elif x == 2 or (x == 3 and z == 6):           # ALU: a keeps its sources and the operand's
            src = self._addr_taint(hl) if (x == 2 and z == 6) else (g(R[z]) if x == 2 else E)
            if x == 2 and z == 7 and y in (2, 5):     # sub a / xor a: zero
                after = ('a', E)
            elif y != 7:
                after = ('a', g('a') | src)
        elif x == 3 and z == 5 and y % 2 == 0:        # push
            p = y >> 1
            self.wt = g('a') if p == 3 else g('bdh'[p]) | g('cel'[p])
        elif x == 3 and z == 1 and y % 2 == 0:        # pop
            p = y >> 1
            t = self._addr_taint(self.sp) | self._addr_taint(self.sp + 1)
            after = [('a', t)] if p == 3 else [('bdh'[p], t), ('cel'[p], t)]
        elif x == 0 and z == 1 and y % 2 == 0:        # ld rr,nn
            p = y >> 1
            if p < 3:
                after = [('bdh'[p], self._addr_taint(pc + 2)), ('cel'[p], self._addr_taint(pc + 1))]
        super().step()
        self.min_sp = min(getattr(self, 'min_sp', 0xFFFF), self.sp)
        if after:
            for r, t in (after if isinstance(after, list) else [after]):
                if t:
                    rt[r] = t
                else:
                    rt.pop(r, None)

    def wr(self, a, v):
        a &= 0xFFFF
        if 0x2000 <= a < 0x4000:
            if self.cur is not None:
                self.bankw.setdefault(self.cur, set()).add(v & 0xFF)
            self.bank_sources |= {(src, self.cur) for src in self.wt}
            if not self.wt and v & 0xFF:   # bank 0 is the same in both numberings
                self.bank_untracked.add(self.cur)
        if a >= 0x8000:
            if self.wt:
                self.mt[a] = self.wt
            else:
                self.mt.pop(a, None)
        super().wr(a, v)


def read_m3u(folder, songs):
    """[(song index, title)] from the rip's playlists; songs they don't list are numbered."""
    out = {}
    for f in sorted(os.listdir(folder)):
        if not f.lower().endswith('.m3u'):
            continue
        for line in open(os.path.join(folder, f), encoding='latin1'):
            if '::GBS,' not in line:
                continue
            parts = line.strip().split('::GBS,', 1)[1].split(',')
            idx = parts[0].strip()
            idx = int(idx[1:], 16) if idx.startswith('$') else int(idx)
            rest = ','.join(parts[1:])
            title = re.split(r'(?<!\\),', rest)[0].replace('\\,', ',') if rest else ''
            title = title.split(' - ')[0].strip()
            out.setdefault(idx, title or f'Song {idx + 1:02d}')
    for i in range(songs):
        out.setdefault(i, f'Song {i + 1:02d}')
    return sorted(out.items())


def clean_name(t):
    t = t.replace('’', "'").replace('\xe9', 'e').replace('\xe8', 'e')
    return re.sub(r'\s+', ' ', t).strip()


def stub_region(g):
    """Where the ripper's code is taken to be without a ROM to compare with."""
    if g['load'] >= 0x4000:
        return lambda L: g['load'] <= L < g['load'] + 0x100
    return lambda L: g['load'] <= L < 0x4000


def trace(g, songs, plays):
    img = image(g)
    agg = dict(ins={}, reads=set(), bankw={}, ram=set(), sram=set(), svbk=set(), play_bank=set(),
               bank_sources=set())
    for song in songs:
        e = TraceEmu(img, stub_region(g))
        e.rst_base = g['load']
        e.pad = 0
        e.sp = g['sp']
        e.wr(0xFF26, 0x80)
        e.a = song
        e.call(g['init'])
        for _ in range(plays):
            agg['play_bank'].add(e.bank)
            e.call(g['play'])
        for k, v in e.ins.items():
            agg['ins'].setdefault(k, v)
        agg['reads'] |= e.reads
        for k, v in e.bankw.items():
            agg['bankw'].setdefault(k, set()).update(v)
        sp_lo = e.min_sp - 2   # the stack's actual extent
        for (reg, a) in list(e.ram_writes) + list(e.ram_reads):
            if reg == 'sram':
                if not (sp_lo <= a < g['sp'] + 2):   # the rip's stack can be in cartridge RAM
                    agg['sram'].add(a)
            elif not (sp_lo <= a < g['sp'] + 2):
                agg['ram'].add((reg, a))
        agg['svbk'] |= e.svbk_writes
        agg['bank_sources'] |= e.bank_sources
        agg.setdefault('rst', set()).update(e.rst_used)
        agg.setdefault('jphl', set()).update(e.jphl)
        agg.setdefault('bank_untracked', set()).update(e.bank_untracked)
    return agg


def free_window(used, size, lo=0xC000, hi=0xD000, align=None):
    """align: the address's low byte must be this."""
    addr = lo
    while True:
        if align is not None:
            addr += (align - addr) % 256
        if addr + size > hi:
            return None
        clash = [u for u in range(addr - 16, addr + size + 16) if u in used]
        if not clash:
            return addr
        addr = max(clash) + 17


def convert(folder, rom=None, plays=120):
    gp = [os.path.join(folder, f) for f in os.listdir(folder) if f.lower().endswith('.gbs')]
    g = load_gbs(gp[0])
    img = image(g)
    tracks = read_m3u(folder, g['songs'])
    t = trace(g, [i for i, _ in tracks], plays)
    problems, notes = [], []
    bm = {b: rb for b, rb, s, l in bankmap(g, rom)} if rom else None
    if not rom:
        notes.append('no ROM: only bank 0 from the load address is treated as the ripper\'s')

    def romlin(L):
        if L < 0x4000:
            return L
        b = L // 0x4000
        return bm.get(b, b) * 0x4000 + L % 0x4000

    # the ripper's bytes that run or are read
    touched = set(t['reads'])
    for L, (b, bank) in t['ins'].items():
        touched |= set(range(L, L + len(b)))
    if rom:
        ripper = {L for L in touched if L < len(img) and (romlin(L) >= len(rom) or img[L] != rom[romlin(L)])}
    else:
        ripper = {L for L in touched if g['load'] <= L < 0x4000}
    for L, (b, bank) in t['ins'].items():   # whole instructions
        if any(x in ripper for x in range(L, L + len(b))):
            ripper |= set(range(L, L + len(b)))

    clusters = []
    for L in sorted(ripper):
        if clusters and L - clusters[-1][1] <= 64 and L // 0x4000 == clusters[-1][0] // 0x4000:
            clusters[-1][1] = L + 1
        else:
            clusters.append([L, L + 1])

    def cluster_of(L):
        for i, (lo, hi) in enumerate(clusters):
            if lo <= L < hi:
                return i
        return None

    # control entering a cluster by falling through from original code can't be moved
    by_end = {L + len(b): L for L, (b, bank) in t['ins'].items()}
    tails = []
    for i, (lo, hi) in enumerate(clusters):
        prev = by_end.get(lo)
        if prev is not None and cluster_of(prev) is None and t['ins'][prev][0][0] not in NO_FALLTHROUGH:
            problems.append('inline patch at %X (original code runs into it)' % lo)
        # leaving by falling through: jump back to the original code after the moved block
        last = by_end.get(hi)
        tail = None
        if last is not None and cluster_of(last) == i and t['ins'][last][0][0] not in NO_FALLTHROUGH and hi in t['ins']:
            cpu = hi if hi < 0x4000 else 0x4000 + hi % 0x4000
            tail = bytes([0xC3, cpu & 0xFF, cpu >> 8])
        tails.append(tail or b'')

    blob = bytearray()
    cl_off = []
    for (lo, hi), tail in zip(clusters, tails):
        cl_off.append(len(blob))
        blob += img[lo:hi] + tail

    # RAM for the moved code, the bank table, the player's loop and its stack
    ram_used = {a for reg, a in t['ram'] if reg == 'w0'}
    if t['svbk'] - {0, 1}:
        notes.append('uses WRAM banks %s' % sorted({v & 7 for v in t['svbk']}))
    data_bank = next((b for b in (7, 6, 5, 4, 3, 2) if b not in {v & 7 for v in t['svbk']}), None)
    if data_bank is None:
        problems.append('no free WRAM bank for the track table')
    ripper_bankw = {L: v for L, v in t['bankw'].items() if cluster_of(L) is not None}
    maxbank = max([max(v) for v in ripper_bankw.values()] + [1])
    LOOP = 0x240
    need = len(blob) + 24 + maxbank + 1 + 0x56 + LOOP
    where = free_window(ram_used, need)
    if where is None:
        problems.append('no free work RAM window of %d bytes' % need)
        return g, tracks, t, None, problems, notes, clusters

    def reloc(L):
        i = cluster_of(L)
        return where + cl_off[i] + L - clusters[i][0]
    xlat_addr = where + len(blob)
    table_addr = xlat_addr + 24
    stack = table_addr + maxbank + 1 + 0x50
    stub_addr = stack + 6

    def cpu_lin(nn, bank):
        return lin(bank, nn) if nn < 0x8000 else None

    for L, (ins, bank) in sorted(t['ins'].items()):
        i = cluster_of(L)
        if i is None:
            continue
        off = reloc(L) - where
        op = ins[0]
        if op in ABS16:
            nn = ins[1] | ins[2] << 8
            tl = cpu_lin(nn, bank)
            if tl is not None and cluster_of(tl) is not None:
                r = reloc(tl)
                blob[off + 1], blob[off + 2] = r & 0xFF, r >> 8
            if op == 0xEA and 0x2000 <= nn < 0x4000:
                blob[off], blob[off + 1], blob[off + 2] = 0xCD, xlat_addr & 0xFF, xlat_addr >> 8
        elif op in JR:
            d = ins[1] - 256 if ins[1] > 127 else ins[1]
            if cluster_of(L + 2 + d) != i:
                problems.append('jr out of a moved block at %X' % L)
        elif op in (0xC7, 0xCF, 0xD7, 0xDF, 0xE7, 0xEF, 0xF7, 0xFF):
            notes.append('rst in moved code at %X' % L)
        elif op == 0xE9:
            notes.append('jp hl in moved code at %X' % L)
    for L in ripper_bankw:
        if t['ins'][L][0][0] != 0xEA:
            problems.append('bank switch at %X isn\'t ld [nn],a (%s)' % (L, t['ins'][L][0].hex()))
    other_bankw = {L: v for L, v in t['bankw'].items() if L not in ripper_bankw}
    if rom and other_bankw:
        # original code switching banks: fine if it writes ROM bank numbers (its own tables)
        odd = [L for L, v in other_bankw.items() if any(x in bm and bm[x] != x for x in v)]
        if odd:
            notes.append('original code writes rip bank numbers? at %s' % ' '.join('%X' % L for L in odd[:4]))

    tb = bytes((bm.get(i, i) if bm else i) for i in range(maxbank + 1))
    xl = bytes([0xE5, 0xF5, 0x21, table_addr & 0xFF, table_addr >> 8,
                0x85, 0x6F, 0x30, 0x01, 0x24, 0x7E, 0xEA, 0x00, 0x20, 0xF1, 0xE1, 0xC9]).ljust(24, b'\0')

    def target(nn, bank):
        tl = cpu_lin(nn, bank)
        return reloc(tl) if tl is not None and cluster_of(tl) is not None else nn
    init = target(g['init'], 1)
    play_banks = t['play_bank']
    play = target(g['play'], min(play_banks) if len(play_banks) == 1 else 1)
    if len(play_banks) > 1 and g['play'] >= 0x4000 and cluster_of(cpu_lin(g['play'], 1)) is not None:
        problems.append('play is in a switched bank that changes')

    sram = bool(t['sram'])
    if sram:
        notes.append('uses cartridge RAM %04X-%04X' % (min(t['sram']), max(t['sram'])))
    timer = (g['tma'], g['tac']) if g['tac'] & 4 else None
    bank1 = bm.get(1, 1) if bm else 1
    init_glue = [0x3E, 0x77, 0xE0, 0x24, 0x3E, 0xF3, 0xE0, 0x25]   # NR50, NR51 as the boot ROM leaves them          # NR50, NR51
    if sram:
        # enable cartridge RAM and clear the part the engine uses (a rip starts with it zeroed;
        # on the cartridge it holds whatever was left there). Nothing outside it is touched.
        lo_, n_ = min(t['sram']), max(t['sram']) + 1 - min(t['sram'])
        init_glue += [0x3E, 0x0A, 0xEA, 0x00, 0x00,
                      0x21, lo_ & 0xFF, lo_ >> 8, 0x01, n_ & 0xFF, n_ >> 8,
                      0xAF, 0x22, 0x0B, 0x78, 0xB1, 0x20, 0xF9]
    start_glue = [0xF5, 0x3E, bank1, 0xEA, 0x00, 0x20, 0xF1, 0xCD, init & 0xFF, init >> 8]
    frame_glue = [0xCD, play & 0xFF, play >> 8]
    if 0x4000 <= play < 0x8000 and len(play_banks) == 1:
        # the engine may leave another bank selected; the game selects the engine's bank first
        pb = min(play_banks)
        frame_glue = [0x3E, bm.get(pb, pb) if bm else pb, 0xEA, 0x00, 0x20] + frame_glue
    entry = dict(
        init=lambda pc, b=bytes(init_glue): b,
        start=lambda pc, b=bytes(start_glue): b,
        frame=lambda pc, b=bytes(frame_glue): b,
        first=0,
        tracks=[(i, 0, clean_name(n)) for i, n in tracks],
        stub=stub_addr, stack=stack, vars=stub_addr - 6, data_bank=data_bank or 7,
        blobs=[(where, bytes(blob) + xl + tb)],
    )
    if timer:
        entry['timer'] = timer
    if blob:
        notes.append('moved %d bytes in %d blocks' % (len(blob), len(clusters)))
    return g, tracks, t, entry, problems, notes, clusters


if __name__ == '__main__':
    folder = sys.argv[1]
    rom = open(sys.argv[2], 'rb').read() if len(sys.argv) > 2 else None
    g, tracks, t, entry, problems, notes, cl = convert(folder, rom)
    print('%s: %d tracks; problems %s; notes %s' % (g['title'], len(tracks), problems, notes))
