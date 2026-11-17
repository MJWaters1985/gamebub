"""Play a GBS rip as a Game Boy ROM image: the music boot ROM and the image's bank 0.

The Game Bub rebuilds a rip from the cartridge's ROM (src/cart_tool/music.rs) and runs it in
the Game Boy core with the music boot ROM (res/cart-music-boot.bin, made by this script) in
place of the Color boot ROM, and the mapper set to MBC3 + 32 KB RAM (the image has no header).

    gbs2rom.py boot <out.bin>                       the boot ROM (parameters left blank)
    gbs2rom.py rom <gbs> <out.gbc> <out boot.bin> [song]   a test image and its boot ROM

Image: the rip's image (bank 0 from the load address, bank n at n*$4000); bank 0 below the
load address holds the rst vectors ($00n0: jp load+n, as GBS rips expect), the timer vector
($0050: jp $8000) and `reti` at the other interrupt vectors. Rips that load at $0053 or above
work; the few that load at $0000 don't.

Boot ROM: copies the player code and the parameters to video RAM ($8000; the LCD stays off,
so video RAM can always be read and nothing uses it), then a setup routine to high RAM, and
runs that: boot ROM off, clear work RAM and cartridge RAM, double speed if the rip asks, sound
on, timer (the rip's own, or about 59.73 Hz for rips played on vblank, with the timer reload
dithered between two values), then init with the song.

Player (video RAM): the timer interrupt calls play, then reads the joypad: Left/Right play
the previous/next song, A restarts it (restart = stack reset, sound off and on, init again).

Parameters (at PARAMS in the boot ROM file, copied to $8600): init, play, sp (little endian),
songs, song (0-based), flags (bit 0 double speed, bit 1 the rip's own timer), tma, tac.
"""
import os, struct, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

BOOT_SIZE = 0x900
CODE = 0x200          # player code + parameters in the boot ROM file (mapped at $0200)
VRAM = 0x8000         # where the player runs
VARS = 0x8600         # parameters and variables in video RAM (copied with the code)
PARAMS = CODE + (VARS - VRAM)
P_INIT, P_PLAY, P_SP, P_SONGS, P_SONG, P_FLAGS, P_TMA, P_TAC = 0, 2, 4, 6, 7, 8, 9, 10
V_JOY, V_ACC = 11, 12
TIMER_VECTOR = 0x50
VBLANK_HZ = 4194304 / 70224


class Asm:
    def __init__(self, org):
        self.org = org
        self.a = bytearray()
        self.labels = {}
        self.fix = []

    def here(self):
        return self.org + len(self.a)

    def label(self, n):
        self.labels[n] = self.here()

    def b(self, *x):
        self.a.extend(x)

    def w(self, op, v):
        if isinstance(v, str):
            self.fix.append((len(self.a) + 1, v, 'abs'))
            v = 0
        self.b(op, v & 0xFF, v >> 8)

    def jr(self, op, target):
        self.fix.append((len(self.a) + 1, target, 'rel'))
        self.b(op, 0)

    def done(self):
        for at, n, kind in self.fix:
            t = self.labels[n]
            if kind == 'abs':
                self.a[at:at + 2] = bytes((t & 0xFF, t >> 8))
            else:
                d = t - (self.org + at + 1)
                assert -128 <= d < 128, n
                self.a[at] = d & 0xFF
        return bytes(self.a)


def player():
    """Player code at $8000 (timer interrupt handler, restart, idle loop)."""
    a = Asm(VRAM)
    a.label('isr')
    a.w(0xFA, VARS + P_FLAGS); a.b(0xE6, 0x02)           # ld a,[flags]; and 2
    a.jr(0x20, 'play')                                   # the rip's own timer: leave TMA
    # vblank rate from the timer: per frame 4096 Hz / 59.7275 = 68.58 ticks (x2 at double
    # speed); reload TMA with 256-68 or 256-69 by an accumulator (+0.58 per frame)
    a.w(0xFA, VARS + V_ACC); a.b(0xC6, 148); a.w(0xEA, VARS + V_ACC)
    a.b(0x06, 256 - 68)                                  # ld b,188
    a.jr(0x30, 'nc1'); a.b(0x05); a.label('nc1')         # jr nc; dec b
    a.w(0xFA, VARS + P_FLAGS); a.b(0xE6, 0x01)
    a.jr(0x28, 'single')
    a.b(0x06, 256 - 137)                                 # double speed: 137.15 ticks
    a.label('single')
    a.b(0x78, 0xE0, 0x06)                                # ld a,b; ldh [TMA],a
    a.label('play')
    a.w(0xFA, VARS + P_PLAY); a.b(0x6F)    # ld a,[play]; ld l,a
    a.w(0xFA, VARS + P_PLAY + 1); a.b(0x67)              # ld a,[play+1]; ld h,a
    a.w(0xCD, 'callhl')
    # joypad: b = A (bit 4), Left (bit 1), Right (bit 0)
    a.b(0x3E, 0x20, 0xE0, 0x00, 0xF0, 0x00, 0xF0, 0x00)  # select d-pad, read twice
    a.b(0x2F, 0xE6, 0x03, 0x47)                          # cpl; and 3; ld b,a
    a.b(0x3E, 0x10, 0xE0, 0x00, 0xF0, 0x00, 0xF0, 0x00)  # select buttons
    a.b(0x2F, 0xE6, 0x01, 0xCB, 0x37, 0xB0, 0x47)        # cpl; and 1; swap a; or b; ld b,a
    a.b(0x3E, 0x30, 0xE0, 0x00)
    a.w(0xFA, VARS + V_JOY); a.b(0x2F, 0xA0, 0x4F)       # c = newly pressed
    a.b(0x78); a.w(0xEA, VARS + V_JOY)
    a.b(0x79, 0xB7)                                      # ld a,c; or a
    a.jr(0x20, 'change')
    a.b(0xD9)                                            # reti
    a.label('change')
    a.w(0xFA, VARS + P_SONG)
    a.b(0xCB, 0x41); a.jr(0x28, 'notright')              # bit 0,c
    a.b(0x3C); a.w(0x21, VARS + P_SONGS); a.b(0xBE)      # inc a; cp [songs]
    a.jr(0x38, 'notright'); a.b(0xAF)                    # jr c; xor a
    a.label('notright')
    a.b(0xCB, 0x49); a.jr(0x28, 'notleft')               # bit 1,c
    a.b(0xB7); a.jr(0x20, 'dec')                         # or a; jr nz
    a.w(0xFA, VARS + P_SONGS)
    a.label('dec')
    a.b(0x3D)
    a.label('notleft')
    a.w(0xEA, VARS + P_SONG)
    a.label('restart')                                   # also the setup's entry (IME off)
    a.w(0xFA, VARS + P_SP); a.b(0x6F); a.w(0xFA, VARS + P_SP + 1); a.b(0x67); a.b(0xF9)  # ld sp,hl
    a.w(0x21, 'idle'); a.b(0xE5)                         # init returns to the idle loop
    a.b(0xAF, 0xE0, 0x26, 0x3E, 0x80, 0xE0, 0x26)        # sound off (clears it) and on
    a.b(0x3E, 0xFF, 0xE0, 0x25, 0x3E, 0x77, 0xE0, 0x24)  # NR51, NR50
    a.b(0x3E, 0x01); a.w(0xEA, 0x2000)                   # bank 1
    a.b(0xAF, 0xE0, 0x0F)                                # IF = 0
    a.w(0xFA, VARS + P_INIT); a.b(0x6F); a.w(0xFA, VARS + P_INIT + 1); a.b(0x67)
    a.w(0xFA, VARS + P_SONG)
    a.label('callhl')
    a.b(0xE9)                                            # jp hl (init returns to idle)
    a.label('idle')
    a.b(0xFB, 0x76, 0x00)                                # ei; halt; nop
    a.jr(0x18, 'idle')
    return a, a.done()


def setup(restart):
    """Setup routine, run from high RAM ($FF80) with A = 1."""
    s = Asm(0xFF80)
    s.b(0xE0, 0x50)                                      # boot ROM off
    s.b(0xF3, 0x31, 0xFE, 0xFF)                          # di; ld sp,$FFFE
    s.w(0x21, 0xC000)                                    # clear $C000-$DFFF
    s.label('w'); s.b(0xAF, 0x22, 0x7C, 0xFE, 0xE0); s.jr(0x20, 'w')
    s.b(0x3E, 0x0A); s.w(0xEA, 0x0000)                   # cartridge RAM on, clear it
    s.w(0x21, 0xA000)
    s.label('c'); s.b(0xAF, 0x22, 0x7C, 0xFE, 0xC0); s.jr(0x20, 'c')
    s.w(0xFA, VARS + P_FLAGS); s.b(0xE6, 0x01)
    s.jr(0x28, 'speed')
    s.b(0x3E, 0x01, 0xE0, 0x4D, 0x10, 0x00)              # KEY1 = 1; stop
    s.label('speed')
    # timer: the rip's TMA/TAC, or 4096 Hz with TMA set per frame
    s.w(0xFA, VARS + P_FLAGS); s.b(0xE6, 0x02)
    s.jr(0x28, 'vbl')
    s.w(0xFA, VARS + P_TMA); s.b(0xE0, 0x06)
    s.w(0xFA, VARS + P_TAC); s.b(0xE6, 0x07, 0xF6, 0x04, 0xE0, 0x07)
    s.jr(0x18, 'ie')
    s.label('vbl')
    s.b(0x3E, 0xBC, 0xE0, 0x06, 0x3E, 0x04, 0xE0, 0x07)  # TMA 188, TAC 4096 Hz on
    s.label('ie')
    s.b(0x3E, 0x04, 0xE0, 0xFF)                          # IE = timer
    s.b(0x3E, 0xFF); s.w(0xEA, VARS + V_JOY)             # buttons held: wait for a release
    s.w(0xC3, restart)
    out = s.done()
    assert len(out) < 0x7F, len(out)
    return out


def boot():
    pl, code = player()
    stub = setup(pl.labels['restart'])
    assert VRAM + len(code) <= VARS
    a = Asm(0)
    a.w(0x31, 0xFFFE)
    # LCD off (it is after reset; make sure), then copy the code and parameters ($0200-$080F) to video RAM
    a.b(0xAF, 0xE0, 0x40)
    a.w(0x21, CODE); a.w(0x11, VRAM); a.w(0x01, VARS - VRAM + 0x10)
    a.label('cp'); a.b(0x2A, 0x12, 0x13, 0x0B, 0x78, 0xB1); a.jr(0x20, 'cp')  # ld a,[hl+]; ld [de],a; inc de; dec bc; ld a,b; or c
    a.w(0x21, 0x80); a.b(0x0E, 0x80, 0x06, len(stub))
    a.label('hs'); a.b(0x2A, 0xE2, 0x0C, 0x05); a.jr(0x20, 'hs')
    a.b(0x3E, 0x01)
    a.w(0xC3, 0xFF80)
    head = a.done()
    assert len(head) <= 0x80
    out = bytearray(BOOT_SIZE)
    out[:len(head)] = head
    out[0x80:0x80 + len(stub)] = stub
    out[CODE:CODE + len(code)] = code
    return bytes(out)


def params(g, song):
    flags = (1 if g['tac'] & 0x80 else 0) | (2 if g['tac'] & 4 else 0)
    return struct.pack('<HHHBBBBB', g['init'], g['play'], g['sp'], g['songs'], song, flags,
                       g['tma'], g['tac'])


def bank0(load):
    """Bank 0 below the load address."""
    if load < TIMER_VECTOR + 3:
        raise ValueError('rip loads at $%04X, below $%04X' % (load, TIMER_VECTOR + 3))
    out = bytearray(load)
    for n in range(0, 0x40, 8):
        t = load + n
        out[n:n + 3] = bytes((0xC3, t & 0xFF, t >> 8))
    for v in range(0x40, min(load, 0x68), 8):
        out[v] = 0xD9
    out[TIMER_VECTOR:TIMER_VECTOR + 3] = bytes((0xC3, VRAM & 0xFF, VRAM >> 8))
    return bytes(out)


def build(g, song=0):
    from gbsdiff import image
    img = bytearray(image(g))
    size = 0x8000
    while size < len(img):
        size *= 2
    img += bytes(size - len(img))
    img[:g['load']] = bank0(g['load'])
    bt = bytearray(boot())
    p = params(g, song)
    bt[PARAMS:PARAMS + len(p)] = p
    return bytes(img), bytes(bt), (0x12, (size // 0x8000).bit_length() - 1, 0x03)


if __name__ == '__main__':
    if sys.argv[1] == 'boot':
        open(sys.argv[2], 'wb').write(boot())
        print('parameters at $%03X' % PARAMS)
    else:
        from gbsdiff import load_gbs
        g = load_gbs(sys.argv[2])
        song = int(sys.argv[5]) if len(sys.argv) > 5 else g['first'] - 1
        img, bt, hdr = build(g, song)
        open(sys.argv[3], 'wb').write(img)
        open(sys.argv[4], 'wb').write(bt)
        print('mapper %02X rom size %02X ram size %02X' % hdr)
