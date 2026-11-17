"""Package a GBS rip for cartridge music, matched to the cartridge on the device.

Without a dump of the cartridge, the ripper's code is taken to be bank 0 from the load address
(and the first $100 bytes when the rip loads above $4000); it is moved to work RAM. Everything
else the rip runs or reads must be the same on the cartridge: the package carries CRCs of it
and a short probe per rip bank, so the device can find each rip bank among the cartridge's
banks, check the code, fill in the rip-bank -> ROM-bank table and refuse a cartridge that
doesn't match (another revision or region, or a rip with other patches).

Package (.cmr), little endian:
  "CMR4", u16 file offset of the bank table, u8 table size,
  u8 count + u16 file offsets of constants holding rip bank numbers (ld a,N before a call into
     the engine) to translate through the table,
  u8 count + probes: u8 rip bank, u16 offset in the bank, u8 length, bytes,
  u16 count + checks: u8 rip bank (0 = bank 0), u16 offset in the bank, u16 length, u32 CRC32,
  u16 count + bank numbers in the engine's data: u8 rip bank, u16 offset, u8 the rip's value
     (on the cartridge: the same, or renumbered by the ripper to the ROM bank that rip bank is in),
  u16 count + bytes in the checked ranges that may be bank numbers the ripper renumbered (their
     value is a rip bank): u8 rip bank, u16 offset, u8 the rip's value. Where the cartridge has
     another value there, the CRC is taken with the rip's, and once the table is filled in the
     cartridge's must be the ROM bank of that rip bank,
  u16 count + calls the ripper redirected to its copy of a bank 0 routine (the address in an
     instruction of the engine's own code points into the moved code): u8 rip bank, u16 offset
     of the address, u16 the rip's address, u16 length, u32 CRC32 of the ripper's copy. Unless the
     cartridge has the same address there, its routine at that address (through up to three jp)
     must have that CRC,
  u8 flags (1: uses cartridge RAM), 2304-byte player file.

package.py <rip folder> <out.cmr>      make one package
"""
import os, sys, struct, zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
from gbs2entry import trace, read_m3u, clean_name, free_window, ABS16, JR, NO_FALLTHROUGH, lin
from gbsdiff import load_gbs, image
import gen_music_player as gm

DATA_CHECK_CAP = 0x2000   # bytes of music data checked (code is always checked in full)
MARKS_CAP = 2000          # possible bank numbers listed (device memory)


def make(folder, plays=30, small=False, _t=None):
    """small: move only the first $100 bytes from the load address (where rippers put their
    init and play code) and check the rest of bank 0 against the cartridge, for rips whose bank 0
    from the load address doesn't fit in work RAM."""
    gp = [os.path.join(folder, f) for f in os.listdir(folder) if f.lower().endswith('.gbs')]
    g = load_gbs(gp[0])
    img = image(g)
    tracks = read_m3u(folder, g['songs'])
    t = _t or trace(g, [i for i, _ in tracks], plays)
    problems = []

    code_bytes = set()
    for L, (b, bank) in t['ins'].items():
        code_bytes |= set(range(L, L + len(b)))
    touched = code_bytes | t['reads']
    ripper = {L for L in touched if g['load'] <= L < (min(0x4000, g['load'] + 0x100) if small else 0x4000)}
    if g['load'] >= 0x4000:
        ripper |= {L for L in touched if g['load'] <= L < g['load'] + 0x100}
    elif 0x4000 <= g['init'] < 0x4100:
        # the ripper's code runs on past the end of bank 0 (its init at the start of bank 1; play
        # there alone can be the engine's own entry point)
        ripper |= {L for L in touched if 0x4000 <= L < 0x4100}
    # data read from the moved code's area that runs on past it (a table or compressed data the
    # ripper's code points at) moves with it, or the moved pointer would run off the end
    for L in sorted(ripper):
        n = L + 1
        while n in t['reads'] and n not in ripper and n - L < 0x800:
            ripper.add(n)
            n += 1

    def whole_instructions():
        for L, (b, bank) in t['ins'].items():
            if any(x in ripper for x in range(L, L + len(b))):
                ripper.update(range(L, L + len(b)))
    whole_instructions()
    # relative jumps in the ripper's code must stay inside what is moved: take in their targets
    for _ in range(4):
        grew = False
        for L, (b, bank) in t['ins'].items():
            if L in ripper and b[0] in JR:
                d = b[1] - 256 if b[1] > 127 else b[1]
                tgt = L + 2 + d
                lo, hi = min(L, tgt), max(L, tgt)
                if tgt not in ripper and hi - lo < 0x200 and lo // 0x4000 == hi // 0x4000:
                    ripper.update(range(lo, hi + 1))
                    grew = True
        whole_instructions()
        if not grew:
            break

    # GBS rips have their rst handlers at load + n; on the cartridge they are at $00n0. They stay
    # there (checked against the cartridge's bank 0) rather than being moved.
    vec = set()
    todo = [g['load'] + n for n in t.get('rst', ())]
    while todo:   # the handlers: what runs on from each vector used, within the vector area
        L = todo.pop()
        if L in vec or L not in t['ins'] or not g['load'] <= L < g['load'] + 0x40:
            continue
        b = t['ins'][L][0]
        vec.update(range(L, L + len(b)))
        if b[0] not in NO_FALLTHROUGH:
            todo.append(L + len(b))
        if b[0] in JR:
            todo.append(L + 2 + (b[1] - 256 if b[1] > 127 else b[1]))
    ripper -= vec

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
    by_end = {L + len(b): L for L, (b, bank) in t['ins'].items()}
    tails = []
    for i, (lo, hi) in enumerate(clusters):
        prev = by_end.get(lo)
        if prev is not None and cluster_of(prev) is None and t['ins'][prev][0][0] not in NO_FALLTHROUGH:
            problems.append('original code runs into the moved code at %X' % lo)
        last = by_end.get(hi)
        tail = b''
        if last is not None and cluster_of(last) == i and t['ins'][last][0][0] not in NO_FALLTHROUGH and hi in t['ins']:
            cpu = hi if hi < 0x4000 else 0x4000 + hi % 0x4000
            tail = bytes([0xC3, cpu & 0xFF, cpu >> 8])
        tails.append(tail)
    blob = bytearray()
    cl_off = []
    for (lo, hi), tail in zip(clusters, tails):
        cl_off.append(len(blob))
        blob += img[lo:hi] + tail

    # rip bank numbers in play: written by the moved code, the bank play runs in, bank 1, and
    # constants passed to the engine
    ripper_bankw = {L: v for L, v in t['bankw'].items() if cluster_of(L) is not None}
    play_banks = t['play_bank']
    # constants in the ripper's code that reach the bank register (followed through the trace)
    # rip bank numbers: every constant or data byte in the moved code that reaches the bank
    # register (however it gets there) is rewritten to the ROM bank on the device
    imm_L = sorted({src for src, writer in t['bank_sources'] if not isinstance(src, tuple) and cluster_of(src) is not None})
    # bank numbers read from the engine's own data must be original on the cartridge: always checked
    must_check = {src[1] for src, writer in t['bank_sources'] if isinstance(src, tuple)}
    for L in t.get('bank_untracked', ()):
        if L is not None and cluster_of(L) is not None:
            problems.append('the moved code at %X switches banks with a value from elsewhere' % L)
    used_banks = {v for s in ripper_bankw.values() for v in s} | (set(play_banks) if len(play_banks) == 1 else set()) | {1}
    used_banks |= {img[L] for L in imm_L}
    used_banks |= {L // 0x4000 for L in touched if L >= 0x4000}
    maxbank = max(used_banks)

    ram_used = {a for reg, a in t['ram'] if reg == 'w0'}
    svbk = {v & 7 for v in t['svbk']}
    data_bank = next((b for b in (7, 6, 5, 4, 3, 2) if b not in svbk), None)
    if data_bank is None:
        problems.append('no free WRAM bank for the track table')
    LOOP = 0x260
    need = len(blob) + 24 + maxbank + 1 + 0x56 + LOOP
    # jp hl into the moved code with h from an `ld h,n` there (an address built from a high byte
    # and a table of low bytes): the code moves by whole pages and those bytes are patched
    hi_fix, align = set(), None
    for L, tgt, hs in t.get('jphl', ()):
        if L is None or tgt is None or cluster_of(L) is None or cluster_of(tgt) is None:
            continue
        for src in hs:
            if isinstance(src, int) and cluster_of(src) is not None and img[src - 1] == 0x26:
                i = cluster_of(tgt)
                a = (clusters[i][0] - cl_off[i]) % 256
                if align is not None and a != align:
                    problems.append('jp hl into moved code needs two alignments')
                align = a
                hi_fix.add((src, tgt))
    where = free_window(ram_used, need, align=align)
    if where is None:
        if not small and g['load'] < 0x4000 - 0x100:
            return make(folder, plays, True, t)
        return None, g, tracks, ['no free work RAM window of %d bytes' % need], None

    def reloc(L):
        i = cluster_of(L)
        return where + cl_off[i] + L - clusters[i][0]
    xlat_addr = where + len(blob)
    table_addr = xlat_addr + 24
    stack = table_addr + maxbank + 1 + 0x50
    stub_addr = stack + 6
    for L, (ins, bank) in sorted(t['ins'].items()):
        i = cluster_of(L)
        if i is None:
            continue
        off = reloc(L) - where
        op = ins[0]
        if op in ABS16:
            nn = ins[1] | ins[2] << 8
            tl = lin(bank, nn) if nn < 0x8000 else None
            if tl is not None and cluster_of(tl) is not None:
                r = reloc(tl)
                blob[off + 1], blob[off + 2] = r & 0xFF, r >> 8
        elif op in JR:
            d = ins[1] - 256 if ins[1] > 127 else ins[1]
            if cluster_of(L + 2 + d) != i:
                problems.append('jr out of moved code at %X' % L)
    for src, tgt in hi_fix:
        o = reloc(src) - where
        blob[o] = (img[src] + ((reloc(tgt) - tgt) >> 8)) & 0xFF
    xl = bytes([0xE5, 0xF5, 0x21, table_addr & 0xFF, table_addr >> 8,
                0x85, 0x6F, 0x30, 0x01, 0x24, 0x7E, 0xEA, 0x00, 0x20, 0xF1, 0xE1, 0xC9]).ljust(24, b'\0')
    table = bytes(range(maxbank + 1))   # filled in on the device
    raw = bytes(blob) + xl + table
    imm_raw = [reloc(L) - where for L in imm_L]

    def target(nn, bank):
        tl = lin(bank, nn) if nn < 0x8000 else None
        return reloc(tl) if tl is not None and cluster_of(tl) is not None else nn
    init = target(g['init'], 1)
    pb = min(play_banks) if play_banks else 1
    play = target(g['play'], pb)
    fixed_play_bank = len(play_banks) == 1
    sram = sorted(t['sram'])
    init_glue = [0x3E, 0x77, 0xE0, 0x24, 0x3E, 0xF3, 0xE0, 0x25]   # NR50, NR51 as the boot ROM leaves them
    if sram:
        lo_, n_ = sram[0], sram[-1] + 1 - sram[0]
        init_glue += [0x3E, 0x0A, 0xEA, 0x00, 0x00, 0x21, lo_ & 0xFF, lo_ >> 8, 0x01, n_ & 0xFF, n_ >> 8,
                      0xAF, 0x22, 0x0B, 0x78, 0xB1, 0x20, 0xF9]
    t1 = table_addr + 1
    start_glue = [0xF5, 0xFA, t1 & 0xFF, t1 >> 8, 0xEA, 0x00, 0x20, 0xF1, 0xCD, init & 0xFF, init >> 8]
    frame_glue = [0xCD, play & 0xFF, play >> 8]
    if 0x4000 <= play < 0x8000 and fixed_play_bank:
        # the engine may leave another bank selected; the game selects the engine's bank first
        tp = table_addr + pb
        frame_glue = [0xFA, tp & 0xFF, tp >> 8, 0xEA, 0x00, 0x20] + frame_glue
    entry = dict(
        init=lambda pc, b=bytes(init_glue): b, start=lambda pc, b=bytes(start_glue): b,
        frame=lambda pc, b=bytes(frame_glue): b, first=0,
        tracks=[(i, 0, clean_name(n)) for i, n in tracks],
        stub=stub_addr, stack=stack, vars=stub_addr - 6, data_bank=data_bank or 7,
        # the moved code is packed; only the bank table stays as it is, for the device to fill in,
        # and the player translates the bank-number constants through it at boot
        blobs=[(where, bytes(blob) + xl)],
        raw_blobs=[(table_addr, table)],
    )
    if imm_raw:
        entry['xlat'] = (table_addr, [where + o for o in imm_raw])
    if g['tac'] & 4:
        entry['timer'] = (g['tma'], g['tac'])
    if problems:
        return None, g, tracks, problems, None
    # too big for the player: shorter names, then numbers only
    full = entry['tracks']
    for shorten in (None, 12, 0):
        if shorten is not None:
            entry['tracks'] = [(i, p1, n[:shorten]) for i, p1, n in full]
        try:
            player, size = gm.generate_game(entry, g['title'])
            break
        except AssertionError as ex:
            err = str(ex).split(': ')[-1]
    else:
        if not small and g['load'] < 0x4000 - 0x100:
            return make(folder, plays, True, t)
        return None, g, tracks, [err], None
    table_at = entry['raw_file_offsets'][0]

    # checks: everything run or read outside the moved code, in runs
    def runs(addrs):
        out = []
        for L in sorted(addrs):
            if out and L == out[-1][1] and L // 0x4000 == out[-1][0] // 0x4000:
                out[-1][1] = L + 1
            else:
                out.append([L, L + 1])
        return out
    # below the load address and past the end of the rip, the rip has nothing to compare
    in_rip = {L for L in touched if g['load'] <= L < len(img)}
    # the engine's own code calling (or pointing at) the ripper's copy of a routine
    redirects, redirect_bytes = [], set()
    for L, (ins, bank) in sorted(t['ins'].items()):
        if cluster_of(L) is not None or ins[0] not in ABS16 or L < g['load']:
            continue
        nn = ins[1] | ins[2] << 8
        tl = lin(bank, nn) if nn < 0x8000 else None
        if tl is None or cluster_of(tl) is None or tl >= 0x4000:
            continue
        n, x = 0, tl
        while x in t['ins'] and n < 64:   # the routine up to its first ret or jump
            b = t['ins'][x][0]
            if b[0] in ABS16 and (b[1] | b[2] << 8) < 0x4000 and cluster_of(b[1] | b[2] << 8) is not None:
                break   # ... or a call to another of the ripper's copies (which the cart has elsewhere)
            n += len(b)
            x += len(b)
            if b[0] in NO_FALLTHROUGH:
                break
        if n:
            redirects.append((L + 1, tl, n))
            redirect_bytes |= {L + 1, L + 2}
    bank_bytes = sorted(L for L in must_check - ripper - vec if L >= g['load'])
    code_runs = runs((code_bytes & in_rip) - ripper - vec - set(bank_bytes) - redirect_bytes)
    data_runs = runs((t['reads'] & in_rip) - code_bytes - ripper - vec - set(bank_bytes))
    checks, budget = [], DATA_CHECK_CAP   # (rip address range, where it is on the cartridge)
    for lo, hi in runs(vec):
        checks.append((lo, hi, lo - g['load']))
    for lo, hi in code_runs:
        checks.append((lo, hi))
    for lo, hi in sorted(data_runs, key=lambda r: r[0]):
        if budget <= 0:
            break
        hi = min(hi, lo + budget)
        checks.append((lo, hi))
        budget -= hi - lo
    probes = {}
    def window(lo, hi):
        # the longest stretch (up to 24 bytes) with no byte that could be a renumbered bank number
        best, start = (0, lo), lo
        for x in range(lo, min(hi, len(img)) + 1):
            if x == min(hi, len(img)) or 1 <= img[x] <= maxbank:
                if x - start > best[0]:
                    best = (x - start, start)
                start = x + 1
        n, at = best
        return at, min(n, 24)
    for b in sorted({lo // 0x4000 for lo, hi in code_runs + data_runs if lo >= 0x4000}):
        runs_b = [r for r in code_runs if r[0] // 0x4000 == b] + [r for r in data_runs if r[0] // 0x4000 == b]
        at, n = max((window(lo, hi) for lo, hi in runs_b), key=lambda w: w[1])
        if n < 8:   # nothing clean enough: the longest run as it is
            at, hi = max(runs_b, key=lambda r: r[1] - r[0])
            n = min(hi, len(img), at + 24) - at
        if n >= 4:
            probes[b] = (at % 0x4000, bytes(img[at:at + n]))
    read_banks = {L // 0x4000 for L in in_rip if L >= 0x4000}   # selected but never read: no matter
    for b in range(1, maxbank + 1):
        if b not in probes and b in used_banks and b in read_banks:
            problems.append('rip bank %d is used but has nothing to find it by' % b)
    if problems:
        return None, g, tracks, problems, None

    # bytes that may be renumbered bank numbers (not exercised by the trace): see the format
    marks = []
    for lo, hi, *at in checks:
        at = at[0] if at else lo
        for i in range(hi - lo):
            if 1 <= img[lo + i] <= maxbank:
                marks.append(((at + i) // 0x4000, (at + i) % 0x4000, img[lo + i]))
    marks = sorted(set(marks))[:MARKS_CAP]

    pkg = bytearray(b'CMR4')
    pkg += struct.pack('<HB', table_at, maxbank + 1)
    pkg += struct.pack('<B', 0)   # constants: translated by the player itself
    pkg += struct.pack('<B', len(probes))
    for b, (off, data) in sorted(probes.items()):
        pkg += struct.pack('<BHB', b, off, len(data)) + data
    pkg += struct.pack('<H', len(checks))
    for lo, hi, *at in checks:
        at = at[0] if at else lo
        pkg += struct.pack('<BHHI', at // 0x4000, at % 0x4000, hi - lo, zlib.crc32(img[lo:hi]))
    pkg += struct.pack('<H', len(bank_bytes))
    for L in bank_bytes:
        pkg += struct.pack('<BHB', L // 0x4000, L % 0x4000, img[L])
    pkg += struct.pack('<H', len(marks))
    for b, o, v in marks:
        pkg += struct.pack('<BHB', b, o, v)
    pkg += struct.pack('<H', len(redirects))
    for L, tl, n in redirects:
        pkg += struct.pack('<BHHHI', L // 0x4000, L % 0x4000, tl, n, zlib.crc32(img[tl:tl + n]))
    pkg += struct.pack('<B', 1 if sram else 0)
    pkg += player
    return bytes(pkg), g, tracks, [], entry


def entry_for(entry, player, P):
    """The entry as the matched player runs it (for checking it in the emulator): the bank table
    from the player file, the constants translated through it."""
    table_addr = entry['raw_blobs'][0][0]
    table = player[P['table_at']:P['table_at'] + P['tsize']]
    blobs = []
    for addr, b in entry['blobs']:
        b = bytearray(b)
        for a in entry.get('xlat', (0, []))[1]:
            if addr <= a < addr + len(b) and b[a - addr] < len(table):
                b[a - addr] = table[b[a - addr]]
        blobs.append((addr, bytes(b)))
    e2 = dict(entry, blobs=blobs + [(table_addr, bytes(table))])
    e2.pop('raw_blobs')
    return e2


def parse(pkg):
    assert pkg[:4] == b'CMR4'
    p = 4
    table_at, tsize = struct.unpack_from('<HB', pkg, p); p += 3
    n = pkg[p]; p += 1
    imms = list(struct.unpack_from('<%dH' % n, pkg, p)); p += 2 * n
    n = pkg[p]; p += 1
    probes = []
    for _ in range(n):
        b, off, ln = struct.unpack_from('<BHB', pkg, p); p += 4
        probes.append((b, off, pkg[p:p + ln])); p += ln
    n, = struct.unpack_from('<H', pkg, p); p += 2
    checks = []
    for _ in range(n):
        checks.append(struct.unpack_from('<BHHI', pkg, p)); p += 9
    n, = struct.unpack_from('<H', pkg, p); p += 2
    bank_bytes = []
    for _ in range(n):
        bank_bytes.append(struct.unpack_from('<BHB', pkg, p)); p += 4
    n, = struct.unpack_from('<H', pkg, p); p += 2
    marks = {}
    for _ in range(n):
        b, o, v = struct.unpack_from('<BHB', pkg, p); p += 4
        marks[(b, o)] = v
    n, = struct.unpack_from('<H', pkg, p); p += 2
    redirects = []
    for _ in range(n):
        redirects.append(struct.unpack_from('<BHHHI', pkg, p)); p += 11
    flags = pkg[p]; p += 1
    return dict(redirects=redirects, marks=marks, bank_bytes=bank_bytes, table_at=table_at, tsize=tsize, imms=imms, probes=probes, checks=checks, flags=flags,
                player=bytes(pkg[p:p + 0x900]))


def match(pkg, rom, mbc1=False):
    """What the device does: find the rip's banks in the cartridge, check them, fill in the
    table. Returns (player file, None) or (None, reason)."""
    P = parse(pkg)
    nbanks = max(2, len(rom) // 0x4000)
    table = list(range(P['tsize']))
    marks = P['marks']
    renumbered = []   # (cartridge byte, rip value) where a possible bank number differs

    def crc_of(rb, cb, o, ln, out):
        data = bytearray(rom[cb * 0x4000 + o:cb * 0x4000 + o + ln])
        for i in range(len(data)):
            v = marks.get((rb, o + i))
            if v is not None and data[i] != v:
                out.append((data[i], v))
                data[i] = v
        return zlib.crc32(data)

    for b, off, data in P['probes']:
        found = [cb for cb in range(1, nbanks) if rom[cb * 0x4000 + off:cb * 0x4000 + off + len(data)] == data]
        ok = None
        for cb in found:
            got = []
            if all(crc_of(b, cb, o, ln, got) == crc for rb, o, ln, crc in P['checks'] if rb == b):
                ok = cb
                renumbered += got
                break
        if ok is None:
            return None, 'rip bank %d not found on the cartridge' % b
        if mbc1 and ok >= 0x20:
            return None, 'needs MBC1 bank %d' % ok
        table[b] = ok
    for rb, o, ln, crc in P['checks']:
        if rb == 0 and crc_of(0, 0, o, ln, renumbered) != crc:
            return None, 'bank 0 code differs'
    for rb, o, tl, n, crc in P['redirects']:
        cb = table[rb] if rb else 0
        x = rom[cb * 0x4000 + o] | rom[cb * 0x4000 + o + 1] << 8
        if x == tl:
            continue   # not redirected: the cartridge's own routine is there
        for _ in range(3):   # through a jump table
            if x + 3 <= 0x4000 and rom[x] == 0xC3 and zlib.crc32(rom[x:x + n]) != crc:
                x = rom[x + 1] | rom[x + 2] << 8
        if x + n > 0x4000 or zlib.crc32(rom[x:x + n]) != crc:
            return None, 'a routine the rip calls differs'
    for c, v in renumbered:
        if v >= len(table) or c != table[v]:
            return None, 'bank numbers differ'
    for rb, o, v in P['bank_bytes']:
        cb = table[rb] if rb else 0
        c = rom[cb * 0x4000 + o]
        if c != v and (v >= len(table) or c != table[v]):
            return None, 'bank numbers in the engine data differ'
    player = bytearray(P['player'])
    for i, v in enumerate(table):
        player[P['table_at'] + i] = v
    for o in P['imms']:
        player[o] = table[player[o]] if player[o] < len(table) else player[o]
    return bytes(player), None


if __name__ == '__main__':
    pkg, g, tracks, problems, entry = make(sys.argv[1])
    if pkg is None:
        print('not packaged:', problems)
        sys.exit(1)
    open(sys.argv[2], 'wb').write(pkg)
    print('%s: %d tracks, %d bytes' % (g['title'], len(tracks), len(pkg)))
