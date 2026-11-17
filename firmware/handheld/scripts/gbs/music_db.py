"""Cartridge music data built into the firmware: for each ROM, how to rebuild its GBS rip from
the ROM itself (src/cart_tool/music.rs plays the result; see gbs2rom.py).

music_db.py <out.db>
ROMs come from CART_MUSIC_ROMS, rips from <CART_MUSIC_WORK>/gbs-dl/x (matched by title as
package_all.py does). The output holds bytes from the rips, so it stays out of git
(res/cart-music.db is ignored; the firmware embeds it only when it's there).

Format (little endian):
  "CMDB", u16 version (1), u16 count, then count index entries sorted by key:
    u32 key (CRC-32 of the ROM header $0134-$014F), u32 offset of the entry
  entry: u16 load, init, play, sp; u8 songs, first (1-based), tma, tac; u32 rip length
    (from the load address), u32 CRC-32 of those bytes, u32 ROM size, u32 ops length, ops
  ops: varint (n << 1 | kind), kind 0: n literal bytes follow; kind 1: varint ROM offset,
    copy n bytes from there
"""
import os, struct, sys, zlib, multiprocessing

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gbsdiff import S, ROMS, norm, load_gbs, image
from package_all import X, titles
from gbs2rom import TIMER_VECTOR

K = 8
MIN_LOAD = TIMER_VECTOR + 3


def delta(img, start, rom):
    """Ops that rebuild img[start:] from rom: copies of 8+ bytes, literals otherwise."""
    idx = {}
    for i in range(0, len(rom) - K + 1):
        k = rom[i:i + K]
        lst = idx.get(k)
        if lst is None:
            idx[k] = [i]
        elif len(lst) < 8:
            lst.append(i)
    ops = []
    lit = bytearray()
    i = start
    nxt = None
    while i < len(img):
        best = (0, 0)
        for c in ([nxt] if nxt is not None else []) + idx.get(img[i:i + K], []):
            n = 0
            while i + n < len(img) and c + n < len(rom) and img[i + n] == rom[c + n]:
                n += 1
            if n > best[1]:
                best = (c, n)
        if best[1] >= K:
            if lit:
                ops.append((0, bytes(lit)))
                lit = bytearray()
            ops.append((1, best[0], best[1]))
            i += best[1]
            nxt = best[0] + best[1]
        else:
            lit.append(img[i])
            i += 1
            nxt = nxt + 1 if nxt is not None else None
    if lit:
        ops.append((0, bytes(lit)))
    return ops


def varint(v):
    out = bytearray()
    while True:
        b = v & 0x7F
        v >>= 7
        out.append(b | (0x80 if v else 0))
        if not v:
            return bytes(out)


def encode(ops):
    out = bytearray()
    for o in ops:
        if o[0] == 0:
            out += varint(len(o[1]) << 1) + o[1]
        else:
            out += varint(o[2] << 1 | 1) + varint(o[1])
    return bytes(out)


def apply(ops, rom):
    out = bytearray()
    for o in ops:
        out += o[1] if o[0] == 0 else rom[o[1]:o[1] + o[2]]
    return bytes(out)


def header_key(rom):
    return zlib.crc32(rom[0x134:0x150])


def entry(g, img, rom, ops):
    rip = img[g['load']:]
    assert apply(ops, rom) == rip
    body = encode(ops)
    return struct.pack('<HHHHBBBBIIII', g['load'], g['init'], g['play'], g['sp'], g['songs'],
                       g['first'], g['tma'], g['tac'], len(rip), zlib.crc32(rip), len(rom),
                       len(body)) + body


def rip_sets():
    import json
    status = json.load(open(S + '/gbs/package_all.json'))
    by_title = {}
    for n in status:
        for t in titles(n):
            by_title.setdefault(t, []).append(n)
    return by_title


BY_TITLE = None


def work(path):
    name = os.path.splitext(os.path.basename(path))[0]
    rom = open(path, 'rb').read()
    best = None
    for n in dict.fromkeys(BY_TITLE.get(norm(name), [])):
        gd = os.path.join(X, n)
        gp = [f for f in os.listdir(gd) if f.lower().endswith('.gbs')]
        if not gp:
            continue
        g = load_gbs(os.path.join(gd, gp[0]))
        if g['load'] < MIN_LOAD:
            continue
        img = image(g)
        ops = delta(img, g['load'], rom)
        e = entry(g, img, rom, ops)
        if best is None or len(e) < len(best[1]):
            best = (n, e)
    return name, header_key(rom), best


def init():
    global BY_TITLE
    BY_TITLE = rip_sets()


if __name__ == '__main__':
    out = sys.argv[1]
    roms = sorted(r for r in ROMS if r.lower().endswith(('.gb', '.gbc')))
    entries = {}
    with multiprocessing.Pool(os.cpu_count(), initializer=init) as pool:
        for name, key, best in pool.imap_unordered(work, roms):
            if best is None:
                continue
            if key in entries and entries[key][1] != best[1]:
                print('header clash, keeping the first:', name)
                continue
            entries[key] = (name, best[1])
            print('%-55s %6d bytes  %s' % (name[:55], len(best[1]), best[0][:40]), flush=True)
    keys = sorted(entries)
    head = b'CMDB' + struct.pack('<HH', 1, len(keys))
    offset = len(head) + 8 * len(keys)
    index = bytearray()
    body = bytearray()
    for k in keys:
        index += struct.pack('<II', k, offset + len(body))
        body += entries[k][1]
    data = head + index + body
    open(out, 'wb').write(data)
    print('%d games, %d bytes' % (len(keys), len(data)))
