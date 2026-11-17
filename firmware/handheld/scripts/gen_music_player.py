"""
Generate a cartridge music player for the Game Boy core, in place of the Game
Boy Color boot ROM (gameboy.bios-cgb.bin: 2304 bytes, mapped at $0000-$00FF and
$0200-$08FF until $FF50 is written).

The player drives a game's own sound driver on the physical cartridge. Each
game has an entry (scripts/cart_music_games.py) with small pieces of glue code:
`init` (once), `start` (start a track: A and C hold the track's parameters) and
`frame` (once per frame), plus its tracks and their names. The player shows the
track number and name; Left/Right choose the previous/next track, Up/Down jump
10, A restarts the track.

Memory: the boot ROM hides cartridge addresses $0200-$08FF, so the player's
loop runs from work RAM bank 0 (at the entry's `stub` address, $C800 unless the
game's driver uses it), with the stack below it. The font and track table go in
the Color's WRAM bank 7 ($D000), which drivers written for the DMG or for bank 1
don't touch; the player switches to bank 7 only to read them.

Usage: python3 scripts/gen_music_player.py <game id> out.bin
       python3 scripts/gen_music_player.py --all <dir>   (every game, named for the SD card)
"""

import sys

from cart_music_games import GAMES
from sm83 import Asm

# 5x7 font, one string per character (7 rows of 5 columns, '#' = pixel).
GLYPHS = {
    " ": "...................................",
    "0": ".###.#...##..###.#.###..##...#.###.",
    "1": "..#...##....#....#....#....#...###.",
    "2": ".###.#...#....#..##..#...#....#####",
    "3": ".###.#...#....#..##.....##...#.###.",
    "4": "...#...##..#.#.#..#.#####...#....#.",
    "5": "######....####.....#....##...#.###.",
    "6": "..##..#...#....####.#...##...#.###.",
    "7": "#####....#...#...#...#....#....#...",
    "8": ".###.#...##...#.###.#...##...#.###.",
    "9": ".###.#...##...#.####....#...#..##..",
    "A": ".###.#...##...#######...##...##...#",
    "B": "####.#...##...#####.#...##...#####.",
    "C": ".###.#...##....#....#....#...#.###.",
    "D": "####.#...##...##...##...##...#####.",
    "E": "######....#....####.#....#....#####",
    "F": "######....#....####.#....#....#....",
    "G": ".###.#...##....#.####...##...#.###.",
    "H": "#...##...##...#######...##...##...#",
    "I": ".###...#....#....#....#....#...###.",
    "J": "..###...#....#....#....#.#..#..##..",
    "K": "#...##..#.#.#..##...#.#..#..#.#...#",
    "L": "#....#....#....#....#....#....#####",
    "M": "#...###.###.#.##...##...##...##...#",
    "N": "#...###..##.#.##..###...##...##...#",
    "O": ".###.#...##...##...##...##...#.###.",
    "P": "####.#...##...#####.#....#....#....",
    "Q": ".###.#...##...##...##.#.##..#..##.#",
    "R": "####.#...##...#####.#.#..#..#.#...#",
    "S": ".###.#...##.....###.....##...#.###.",
    "T": "#####..#....#....#....#....#....#..",
    "U": "#...##...##...##...##...##...#.###.",
    "V": "#...##...##...##...##...#.#.#...#..",
    "W": "#...##...##...##.#.##.#.##.#.#.#.#.",
    "X": "#...##...#.#.#...#...#.#.#...##...#",
    "Y": "#...##...#.#.#...#....#....#....#..",
    "Z": "#####....#...#...#...#...#....#####",
    "-": "...............#####...............",
    "'": "..#....#....#......................",
    ".": "..............................##...",
    ":": ".......##...##........##...##......",
    "!": "..#....#....#....#....#.........#..",
    "?": ".###.#...#....#...#....#.........#.",
    "&": ".##..#..#.#.#...#...#.#.##..#..##.#",
    "/": "....#...#....#...#...#....#....#....",
    "(": "...#...#...#....#....#.....#.....#.",
    ")": ".#.....#.....#....#....#...#...#...",
    ",": ".........................#...#..#..",
}
FONT_CHARS = "".join(GLYPHS)


def font_tiles(chars=FONT_CHARS):
    data = bytearray()
    for ch in chars:
        g = GLYPHS[ch]
        g = g.ljust(35, ".")[:35]
        for y in range(7):
            bits = 0
            for x in range(5):
                if g[y * 5 + x] == "#":
                    bits |= 0x40 >> x  # one column of left margin
            data.append(bits)
    return bytes(data)  # 7 bytes per glyph


def font_for(names):
    """The glyphs a game needs: space and digits first (the track number is drawn as glyph
    1 + digit), then "/" and the characters its track names use, in FONT_CHARS order."""
    used = {c for n in names for c in n.upper()[:20] if c in GLYPHS}
    return " 0123456789/" + "".join(c for c in FONT_CHARS if c in used and c not in " 0123456789/")


def encode_name(name, chars=FONT_CHARS):
    out = bytearray()
    for ch in name.upper()[:20]:
        i = chars.find(ch)
        out.append(i if i >= 0 else 0)
    return bytes([len(out)]) + bytes(out)


WRAM7 = 0xD000


def build(game):
    """Optional entry fields, for games whose RAM use needs them (e.g. entries made from GBS
    rips, scripts/gbs2entry.py): `stub` (the loop's address, in $C000-$CFFF), `vars` (3 bytes:
    track, previous joypad, redraw flag; default stub - 6), `data_bank` (the work RAM bank for
    the font and track table, default 7), `blobs` ([(address, bytes)] copied to $C000-$CFFF at
    boot), `timer` ((TMA, TAC) from a GBS header: run `frame` on the hardware timer instead of
    once a frame; TAC bit 7 selects double-speed mode)."""
    stub_addr = game.get("stub", 0xC800)
    var = game.get("vars", stub_addr - 6)
    VAR_TRACK, VAR_JOY, VAR_REDRAW = var, var + 1, var + 2
    data_bank = game.get("data_bank", 7)
    timer = game.get("timer")
    tracks = game["tracks"]          # list of (p0, p1, name)
    chars = font_for([n for _, _, n in tracks])
    game["font_chars"] = chars
    linear = all(p0 == i + game.get("first", 1) and p1 == 0 for i, (p0, p1, _) in enumerate(tracks))
    count = len(tracks)

    # Bank 7 data: count, flags, then per track [p0 p1 if not linear] and the name (length + glyphs).
    # Entries follow each other; the player finds the Nth by walking the name lengths.
    table = bytearray([count, 1 if linear else 0, game.get("first", 1)])
    for p0, p1, name in tracks:
        if not linear:
            table += bytes([p0, p1])
        table += encode_name(name, chars)

    # --- The loop in work RAM ---
    a = Asm(stub_addr)
    a.b(0x3E, 0x01); a.b(0xE0, 0x50)                 # boot ROM off
    a.b(0x3E, 0x80); a.b(0xE0, 0x26)                 # sound on
    if timer and timer[1] & 0x80:                    # double speed (KEY1 + stop)
        a.b(0x3E, 0x30, 0xE0, 0x00, 0x3E, 0x01, 0xE0, 0x4D, 0x10, 0x00)
    a.code += game["init"](stub_addr)                # glue: init
    if timer:                                        # the hardware timer paces `frame`
        a.b(0x3E, timer[0], 0xE0, 0x06, 0x3E, (timer[1] & 3) | 4, 0xE0, 0x07)
    a.b(0xAF); a.ref16(0xEA, VAR_TRACK)
    a.ref16(0xCD, "start")

    a.label("frame")
    # find the entry to draw before the vertical blank: walking a long table takes longer than it
    a.ref16(0xFA, VAR_REDRAW); a.b(0xB7); a.ref16(0xC4, "name_ptr")   # call nz,name_ptr -> hl
    a.label("wait_vbl")
    if timer:
        a.ref16(0xCD, "tick")
    a.b(0xF0, 0x44); a.b(0xFE, 144); a.jr(0x20, "wait_vbl")
    a.ref16(0xFA, VAR_REDRAW); a.b(0xB7); a.jr(0x28, "no_redraw")
    a.ref16(0xCD, "draw")
    a.label("no_redraw")
    if not timer:
        a.ref16(0xCD, "frame_glue")
    # joypad -> b = {start,select,B,A,down,up,left,right}, 1 = pressed
    a.b(0x3E, 0x20); a.b(0xE0, 0x00); a.b(0xF0, 0x00); a.b(0xF0, 0x00)
    a.b(0x2F, 0xE6, 0x0F, 0x47)
    a.b(0x3E, 0x10); a.b(0xE0, 0x00); a.b(0xF0, 0x00); a.b(0xF0, 0x00)
    a.b(0x2F, 0xE6, 0x0F, 0xCB, 0x37, 0xB0, 0x47)
    a.b(0x3E, 0x30); a.b(0xE0, 0x00)
    a.ref16(0xFA, VAR_JOY); a.b(0x2F, 0xA0, 0x4F); a.b(0x78); a.ref16(0xEA, VAR_JOY)
    for mask, delta in ((0x01, 1), (0x02, -1), (0x04, -10), (0x08, 10)):  # right, left, up, down
        skip = f"skip{mask}"
        a.b(0x79, 0xE6, mask); a.jr(0x28, skip)
        a.ref16(0xFA, VAR_TRACK); a.b(0xC6, delta & 0xFF)  # add a,delta
        # wrap into 0..count-1: values >= count (including negatives as 2xx) wrap
        if delta > 0:
            a.b(0xFE, count); a.jr(0x38, f"ok{mask}")      # cp count ; jr c
            a.b(0xD6, count)                               # sub count
            a.b(0xFE, count); a.jr(0x38, f"ok{mask}")
            a.b(0xAF)                                      # still too big: 0
        else:
            a.b(0xFE, count); a.jr(0x38, f"ok{mask}")
            a.b(0xC6, count)                               # add count (from negative)
            a.b(0xFE, count); a.jr(0x38, f"ok{mask}")
            a.b(0x3E, count - 1)
        a.label(f"ok{mask}")
        a.ref16(0xEA, VAR_TRACK); a.ref16(0xCD, "start")
        a.ref16(0xC3, "wait_end")       # one action per frame (start clobbers c, the pressed mask)
        a.label(skip)
    a.b(0x79, 0xE6, 0x10); a.jr(0x28, "skipA"); a.ref16(0xCD, "start"); a.label("skipA")
    a.label("wait_end")
    if timer:
        a.ref16(0xCD, "tick")
    a.b(0xF0, 0x44); a.b(0xFE, 144); a.jr(0x28, "wait_end")
    a.ref16(0xC3, "frame")

    if timer:
        # tick: run `frame` when the timer has overflowed (IF bit 2; interrupts stay off)
        a.label("tick")
        a.b(0xF0, 0x0F, 0xCB, 0x57, 0xC8)            # ldh a,[IF] ; bit 2,a ; ret z
        a.b(0xE6, 0xFB, 0xE0, 0x0F)                  # and ~4 ; ldh [IF],a
        a.ref16(0xC3, "frame_glue")

    # start: look up the track's parameters (bank 7), then glue start(A=p0, C=p1)
    a.label("start")
    a.b(0x3E, 0x01); a.ref16(0xEA, VAR_REDRAW)
    if linear:
        a.ref16(0xFA, VAR_TRACK); a.b(0xC6, game.get("first", 1)); a.b(0x0E, 0x00)
    else:
        a.ref16(0xCD, "name_ptr")                    # hl = entry (p0, p1, name)
        a.b(0x3E, data_bank); a.b(0xE0, 0x70)             # bank 7
        a.b(0x2A, 0x4E, 0x47)                        # ld a,[hl+] (p0) ; ld c,[hl] (p1) ; ld b,a
        a.b(0x3E, 0x01); a.b(0xE0, 0x70); a.b(0x78)  # bank 1 ; ld a,b
    a.code += game["start"](a.pc)
    a.b(0xC9)

    # name_ptr: hl = pointer to the current track's entry (bank 7 table), walking the entries
    a.label("name_ptr")
    a.b(0x3E, data_bank); a.b(0xE0, 0x70)
    a.ref16(0xFA, VAR_TRACK); a.b(0x47)                        # b = track index
    a.ref16(0x21, WRAM7 + 3)
    a.label("walk"); a.b(0x78, 0xB7); a.jr(0x28, "walked")    # b == 0: found
    if not linear:
        a.b(0x23, 0x23)                                        # skip p0, p1
    a.b(0x7E, 0x3C)                                            # a = length + 1
    a.b(0x85, 0x6F, 0x30, 0x01, 0x24)                          # hl += a (add a,l ; ld l,a ; jr nc,+1 ; inc h)
    a.b(0x05); a.jr(0x18, "walk")                              # dec b ; jr walk
    a.label("walked")
    a.b(0x3E, 0x01); a.b(0xE0, 0x70); a.b(0xC9)

    # draw: "NN/MM" on row 6, name on row 8 (columns 0-19), during vertical blank
    a.label("draw")                                            # hl = the entry (from name_ptr)
    a.b(0xE5); a.b(0xAF); a.ref16(0xEA, VAR_REDRAW)
    wide = count > 99                                          # three digits
    a.ref16(0x21, 0x9800 + 6 * 32 + (6 if wide else 7))
    a.ref16(0xFA, VAR_TRACK); a.b(0x3C); a.ref16(0xCD, "put2")   # track number (1-based)
    a.b(0x3E, chars.index("/")); a.b(0x22)
    a.b(0x3E, count); a.ref16(0xCD, "put2")
    a.ref16(0x21, 0x9800 + 8 * 32)                             # clear the name row
    a.b(0x06, 20, 0xAF); a.label("clr_row"); a.b(0x22, 0x05); a.jr(0x20, "clr_row")
    a.b(0xE1)                                                  # pop hl (the entry)
    a.b(0x3E, data_bank); a.b(0xE0, 0x70)
    if not linear:
        a.b(0x23, 0x23)                                        # skip p0, p1
    a.b(0x46, 0x23)                                            # ld b,[hl] (length) ; inc hl
    a.b(0x78); a.b(0xD6, 20); a.b(0x2F, 0x3C)                  # a = 20 - len ...
    a.b(0xCB, 0x3F)                                            # srl a -> left padding
    a.b(0x5F, 0x16, 0x00); a.b(0xE5)                           # de = pad ; push hl
    a.ref16(0x21, 0x9800 + 8 * 32); a.b(0x19); a.b(0x54, 0x5D) # hl = row+pad -> de
    a.b(0xE1)                                                  # pop hl (name)
    a.b(0x78, 0xB7); a.jr(0x28, "name_done")
    a.label("name_cpy"); a.b(0x2A, 0x12, 0x13, 0x05); a.jr(0x20, "name_cpy")
    a.label("name_done")
    a.b(0x3E, 0x01); a.b(0xE0, 0x70); a.b(0xC9)

    # put2: write A (0-99, or 0-255 when wide) as digits at [hl+] (glyph 1 + n = digit n)
    a.label("put2")
    if wide:
        a.b(0x0E, 0xFF)
        a.label("hundreds"); a.b(0x0C, 0xD6, 100); a.jr(0x30, "hundreds")
        a.b(0xC6, 100, 0x47); a.b(0x79, 0x3C, 0x22); a.b(0x78)  # hundreds digit; a = rest
    a.b(0x0E, 0xFF)
    a.label("tens"); a.b(0x0C, 0xD6, 10); a.jr(0x30, "tens")
    a.b(0xC6, 10 + 1); a.b(0x47); a.b(0x79, 0x3C, 0x22); a.b(0x78, 0x22); a.b(0xC9)

    a.label("frame_glue")
    a.code += game["frame"](a.pc)
    a.b(0xC9)
    stub = a.resolve()
    return stub, bytes(table), stub_addr


LOW_AREA = 0x00FC  # bytes free at $0004-$00FF (the first part of the packed data)


def lz_tokens(data):
    """Greedy LZ77 for unlz: token $01-$7F = that many literal bytes follow; $80+n = copy
    n+3 bytes from `distance` back in the output (next byte: distance - 1); $00 = end."""
    out, lit, i = [], bytearray(), 0

    def flush():
        while lit:
            n = min(len(lit), 0x7F)
            out.append(bytes([n]) + lit[:n])
            del lit[:n]
    while i < len(data):
        best_len, best_dist = 0, 0
        for dist in range(1, min(256, i) + 1):
            n = 0
            while i + n < len(data) and n < 0x7F + 3 and data[i - dist + n] == data[i + n]:
                n += 1
            if n > best_len:
                best_len, best_dist = n, dist
        if best_len >= 3:
            flush()
            out.append(bytes([0x80 | (best_len - 3), best_dist - 1]))
            i += best_len
        else:
            lit.append(data[i])
            i += 1
    flush()
    return out


def unlz(data):
    """Python model of the player's unlz, to check the packing."""
    out, i = bytearray(), 0
    while data[i]:
        t = data[i]; i += 1
        if t < 0x80:
            out += data[i:i + t]; i += t
        else:
            dist = data[i] + 1; i += 1
            for _ in range((t & 0x7F) + 3):
                out.append(out[-dist])
    return bytes(out)


def boot(stub, stub_addr, table, font, stack, data_bank=7, blobs=(), raw_blobs=()):
    # Pack table + font + loop; the first tokens go in $0004-$00FF, the rest after the code.
    data = table + font + stub + b"".join(b for _, b in blobs)
    tokens = lz_tokens(data)
    low = bytearray()
    while tokens and len(low) + len(tokens[0]) <= LOW_AREA - 1:
        low += tokens.pop(0)
    low.append(0)
    high = b"".join(tokens) + b"\x00"
    assert unlz(bytes(low[:-1]) + high) == data
    font_at, stub_at = WRAM7 + len(table), WRAM7 + len(table) + len(font)
    blob_at = stub_at + len(stub)

    a = Asm(0x0200)
    a.b(0xF3); a.ref16(0x31, 0xFFFE)                 # stack in HRAM while WRAM is cleared
    # clear all of work RAM (banks 0-7), then HRAM (inline, once the stack is in WRAM)
    a.ref16(0x21, 0xC000); a.ref16(0x01, 0x1000); a.ref16(0xCD, "fill0")
    for bank in range(1, 8):
        a.b(0x3E, bank); a.b(0xE0, 0x70)
        a.ref16(0x21, 0xD000); a.ref16(0x01, 0x1000); a.ref16(0xCD, "fill0")
    a.ref16(0x31, stack)
    a.ref16(0x21, 0xFF80); a.b(0x06, 0x7F); a.label("clrh"); a.b(0xAF, 0x22, 0x05); a.jr(0x20, "clrh")
    # unpack to the data bank: table at $D000, then the font, the loop and any blobs
    a.b(0x3E, data_bank); a.b(0xE0, 0x70)
    a.ref16(0x21, 0x0004); a.ref16(0x11, WRAM7); a.ref16(0xCD, "unlz")
    a.ref16(0x21, "packed"); a.ref16(0xCD, "unlz")   # de continues
    # font: 7 rows of 1bpp -> tiles at $8000 (both bit planes, colour 3), 8th row blank
    a.ref16(0x21, font_at); a.ref16(0x11, 0x8000); a.b(0x06, len(font) // 7)
    a.label("glyph"); a.b(0x0E, 7)
    a.label("grow"); a.b(0x2A, 0x12, 0x13, 0x12, 0x13, 0x0D); a.jr(0x20, "grow")
    a.b(0xAF, 0x12, 0x13, 0x12, 0x13, 0x05); a.jr(0x20, "glyph")
    # the loop to work RAM bank 0
    a.ref16(0x21, stub_at); a.ref16(0x11, stub_addr); a.ref16(0x01, len(stub))
    a.ref16(0xCD, "copy")
    for addr, blob in blobs:
        assert 0xC000 <= addr and addr + len(blob) <= 0xD000, "blobs go in work RAM bank 0"
        a.ref16(0x21, blob_at); a.ref16(0x11, addr); a.ref16(0x01, len(blob)); a.ref16(0xCD, "copy")
        blob_at += len(blob)
    for i, (addr, blob) in enumerate(raw_blobs):   # stored as is, so they can be patched in the file
        assert 0xC000 <= addr and addr + len(blob) <= 0xD000, "blobs go in work RAM bank 0"
        a.ref16(0x21, f"raw{i}"); a.ref16(0x11, addr); a.ref16(0x01, len(blob)); a.ref16(0xCD, "copy")
    a.b(0x3E, 0x01); a.b(0xE0, 0x70)
    # background map: spaces
    a.ref16(0x21, 0x9800); a.ref16(0x01, 0x400); a.ref16(0xCD, "fill0")
    # palette 0: white background, black text
    a.b(0x3E, 0x80); a.b(0xE0, 0x68)
    for byte in (0xFF, 0x7F, 0xFF, 0x7F, 0x00, 0x00, 0x00, 0x00):
        a.b(0x3E, byte); a.b(0xE0, 0x69)
    # LCD on (BG, tiles at $8000), go
    a.b(0x3E, 0x91); a.b(0xE0, 0x40)
    # unmap the player so the cartridge's own $0000-$08FF shows (the rst handlers and any low
    # routines its sound driver uses). Everything from here on runs from RAM; the write itself
    # can't be in the player, so it's done from high RAM: ldh [$FF50],a ; jp stub
    a.ref16(0x21, 0xFF80)
    for byte in (0xE0, 0x50, 0xC3, stub_addr & 0xFF, stub_addr >> 8):
        a.b(0x36, byte, 0x23)                        # ld [hl],n ; inc hl
    a.b(0x3E, 0x01); a.ref16(0xC3, 0xFF80)
    a.label("fill0"); a.b(0xAF, 0x22, 0x0B, 0x78, 0xB1); a.jr(0x20, "fill0"); a.b(0xC9)
    a.label("copy"); a.b(0x2A, 0x12, 0x13, 0x0B, 0x78, 0xB1); a.jr(0x20, "copy"); a.b(0xC9)
    # unlz: unpack hl -> de (format: lz_tokens)
    a.label("unlz"); a.b(0x2A, 0xB7, 0xC8)                       # ld a,[hl+] ; or a ; ret z
    a.b(0xCB, 0x7F); a.jr(0x20, "lz_copy")                       # bit 7,a
    a.b(0x47)
    a.label("lz_lit"); a.b(0x2A, 0x12, 0x13, 0x05); a.jr(0x20, "lz_lit"); a.jr(0x18, "unlz")
    a.label("lz_copy"); a.b(0xE6, 0x7F, 0xC6, 0x03, 0x47)         # b = length
    a.b(0x2A, 0xE5)                                              # a = distance - 1 ; push hl
    a.b(0x2F, 0x83, 0x6F, 0x7A, 0xCE, 0xFF, 0x67)                # hl = de - distance
    a.label("lz_rep"); a.b(0x2A, 0x12, 0x13, 0x05); a.jr(0x20, "lz_rep")
    a.b(0xE1); a.jr(0x18, "unlz")                                # pop hl
    a.label("packed"); a.code += high
    for i, (addr, blob) in enumerate(raw_blobs):
        a.label(f"raw{i}"); a.code += blob
    return a.resolve(), bytes(low)


def generate(game_id):
    return generate_game(GAMES[game_id], game_id)


def generate_game(game, game_id="entry"):
    stub, table, stub_addr = build(game)
    assert stub_addr + len(stub) <= 0xD000, f"{game_id}: the loop doesn't fit in work RAM bank 0"
    font = font_tiles(game["font_chars"])
    blobs = game.get("blobs", ())
    assert len(table) + len(font) + len(stub) + sum(len(b) for _, b in blobs) <= 0x1000
    raw_blobs = game.get("raw_blobs", ())
    code, low = boot(stub, stub_addr, table, font, game.get("stack", stub_addr - 0x10),
                     game.get("data_bank", 7), blobs, raw_blobs)
    # where each raw blob is in the file (they end the code), for patching the file later
    end = 0x200 + len(code)
    for addr, blob in reversed(raw_blobs):
        end -= len(blob)
    offs = []
    for addr, blob in raw_blobs:
        offs.append(end)
        end += len(blob)
    game["raw_file_offsets"] = offs
    assert 0x200 + len(code) <= 0x900, f"{game_id}: player is {len(code)} bytes, {0x700} available"
    rom = bytearray(0x900)
    rom[0:3] = bytes([0xC3, 0x00, 0x02])
    rom[0x200:0x200 + len(code)] = code
    rom[0x0004:0x0004 + len(low)] = low
    return bytes(rom), len(code)


def file_name(game):
    """The player's file name, as the firmware looks for it (cart_music_player_path)."""
    title = "".join(c if c.isalnum() else "_" for c in game["title"]).rstrip("_")
    return f"{title}-{game['version']}-{game['checksum']:04X}.bin"


if __name__ == "__main__":
    if sys.argv[1] == "--all":
        # Every game's player, for /system/cart-music on the SD card.
        import os
        os.makedirs(sys.argv[2], exist_ok=True)
        for game_id, game in GAMES.items():
            rom, size = generate(game_id)
            path = os.path.join(sys.argv[2], file_name(game))
            open(path, "wb").write(rom)
            print(f"{path}: {game['name']}, {size} of {0x700} bytes")
    else:
        rom, size = generate(sys.argv[1])
        open(sys.argv[2], "wb").write(rom)
        print(f"{sys.argv[2]}: {size} of {0x700} bytes")
