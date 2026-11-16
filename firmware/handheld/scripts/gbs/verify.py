"""Check a GBS-derived entry against its rip: the sound register writes the entry's code makes
on the cartridge ROM must match what the rip makes, frame by frame.

verify(g, entry, rom, songs, frames) -> list of (song, frames matched, total, first difference)
"""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from sm83emu import Emu, Halt
from gbsdiff import image


class SoundEmu(Emu):
    def __init__(self, mem):
        super().__init__(mem)
        self.trace_exec = False
        self.log = []
        self.written = set()
        self.inputs = set()    # RAM read before it was written

    def rd(self, a):
        a &= 0xFFFF
        if (0xC000 <= a < 0xE000 or 0xFF80 <= a < 0xFFFF) and a not in self.written:
            self.inputs.add(a)
        if a == 0xFF26:
            return (self.hi[0x126] & 0x80) | 0x70
        return super().rd(a)

    def wr(self, a, v):
        a &= 0xFFFF
        self.written.add(a)
        if 0xFF10 <= a < 0xFF40 and a != 0xFF26:
            self.log.append((a, v & 0xFF))
        super().wr(a, v)


def reference(g, song, frames, emu=None):
    e = SoundEmu(image(g))
    e.sp = g['sp']
    e.wr(0xFF26, 0x80)
    e.a = song
    e.call(g['init'])
    out = []
    for _ in range(frames):
        e.log = []
        e.call(g['play'])
        out.append(e.log)
    if emu is not None:
        emu.append(e)
    return out


HARNESS = 0xDF00   # glue routines (work RAM bank 1 top; the trace keeps clear of the stack area)


def candidate(entry, rom, song, frames, preset=()):
    e = SoundEmu(rom)
    import random
    rnd = random.Random(1)
    e.sram = bytearray(rnd.randrange(256) for _ in range(len(e.sram)))   # cartridge RAM isn't clear
    for b in range(2, 8):
        e.wram[b] = bytearray(rnd.randrange(256) for _ in range(0x1000))
    for addr, blob in entry.get('blobs', ()):
        for i, b in enumerate(blob):
            e.wr(addr + i, b)
    for addr, v in preset:
        e.wr(addr, v)
    code = {}
    at = entry['stub']
    for name in ('init', 'start', 'frame'):
        b = entry[name](at) + b'\xC9'
        code[name] = at
        for i, x in enumerate(b):
            e.wr(at + i, x)
        at += len(b)
    if entry.get('init_bytes_at'):
        pass
    e.sp = entry['stack']
    e.wr(0xFF26, 0x80)
    e.call(code['init'])
    e.a = song
    e.c = 0
    e.call(code['start'])
    out = []
    for _ in range(frames):
        e.log = []
        e.call(code['frame'])
        out.append(e.log)
    return out


def verify(g, entry, rom, songs, frames=120, preset=()):
    res = []
    for s in songs:
        ref = reference(g, s, frames)
        try:
            got = candidate(entry, rom, s, frames, preset)
        except (RuntimeError, Halt) as ex:
            res.append((s, 0, frames, 'hang: %s' % ex))
            continue
        ok = 0
        diff = None
        for i, (r, c) in enumerate(zip(ref, got)):
            if r == c:
                ok += 1
            elif diff is None:
                diff = 'frame %d: rip %s / cart %s' % (i, r[:4], c[:4])
        res.append((s, ok, frames, diff))
    return res


def repair(g, entry, rom, bm, song, frames=60):
    """Look for RAM the rip's engine reads before writing (left zero in the rip, set by the game
    on the cartridge) whose preset to a bank offset makes the cartridge match the rip."""
    em = []
    ref = reference(g, song, frames, em)
    inputs = sorted(a for a in em[0].inputs if not (g['sp'] - 0x100 <= a <= g['sp']))
    offsets = sorted({rb - b for b, rb in bm.items() if b > 0 and rb != b}) if bm else []
    def score(preset):
        try:
            got = candidate(entry, rom, song, frames, preset)
        except (RuntimeError, Halt):
            return -1
        return sum(1 for r, c in zip(ref, got) if r == c)
    best = (score(()), ())
    for k in offsets:
        for a in inputs:
            p = best[1] + ((a, k & 0xFF),)
            s = score(p)
            if s > best[0]:
                best = (s, p)
            if best[0] == frames:
                return best
    return best


def repair_immediates(g, entry, rom, bm, songs, frames=40):
    """Constants in the moved code that equal a renumbered rip bank (e.g. `ld a,1` passed to
    the engine as its bank) are tried as the ROM bank; changes are kept when more frames match."""
    if not bm:
        return entry, []
    addr, blob = entry['blobs'][0]
    blob = bytearray(blob)
    refs = {s: reference(g, s, frames) for s in songs}

    def score(b):
        e2 = dict(entry, blobs=[(addr, bytes(b))])
        total = 0
        for s in songs:
            try:
                got = candidate(e2, rom, s, frames)
            except (RuntimeError, Halt):
                continue
            total += sum(1 for r, c in zip(refs[s], got) if r == c)
        return total
    best = score(blob)
    changes = []
    for i in range(len(blob) - 1):
        if blob[i] in (0x3E, 0x06, 0x0E, 0x16, 0x1E, 0x26, 0x2E) and blob[i + 1] in bm and bm[blob[i + 1]] != blob[i + 1]:
            trial = bytearray(blob)
            trial[i + 1] = bm[blob[i + 1]]
            s = score(trial)
            if s > best:
                best, blob = s, trial
                changes.append('%04X: %02X->%02X' % (addr + i, entry['blobs'][0][1][i + 1], trial[i + 1]))
    return dict(entry, blobs=[(addr, bytes(blob))]), changes
