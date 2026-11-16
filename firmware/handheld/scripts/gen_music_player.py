"""
Generate a cartridge music player for the Game Boy core, in place of the Game
Boy Color boot ROM (gameboy.bios-cgb.bin: 2304 bytes, mapped at $0000-$00FF and
$0200-$08FF until $FF50 is written).

The player drives a game's own sound driver on the physical cartridge: it
clears RAM, calls the driver's init routine once, then calls its per-frame
routine on every frame and starts tracks by writing the driver's track-request
variable. Left/Right choose the previous/next track, A restarts it.

Because the boot ROM hides cartridge addresses $0200-$08FF, the player copies
its main loop to work RAM, switches the boot ROM off and runs from there.

Prototype: The Legend of Zelda: Link's Awakening DX (USA, Europe) (Rev 2),
whose driver addresses come from the zladx/LADX-Disassembly project and were
checked against that revision.

Usage: python3 scripts/gen_music_player.py out.bin
"""

import sys

# Link's Awakening DX (Rev 2)
GAME = dict(
    init_bank=0x1F,       # SoundSystemInit: bank $1F
    init_addr=0x4000,
    step_addr=0x089F,     # PlayAudioStep (bank 0; switches banks itself)
    track_var=0xD368,     # wMusicTrackToPlay
    frame_counter=0xFF,   # hFrameCounter is $FFE7 (low byte below)
    frame_counter_lo=0xE7,
    first_track=0x01,
    last_track=0x4F,
    start_track=0x01,     # Title screen
)

STUB_RAM = 0xC800         # where the main loop runs (game RAM the driver doesn't use)
VAR_TRACK = 0xC7F0        # current track
VAR_JOY = 0xC7F1          # previous joypad state (1 = pressed)


class Asm:
    """A tiny two-pass assembler for the few SM83 instructions the player uses."""

    def __init__(self, origin):
        self.origin = origin
        self.code = bytearray()
        self.labels = {}
        self.fixups = []  # (offset, label, kind)

    @property
    def pc(self):
        return self.origin + len(self.code)

    def label(self, name):
        self.labels[name] = self.pc

    def b(self, *bs):
        self.code += bytes(bs)

    def w(self, v):
        self.code += bytes([v & 0xFF, (v >> 8) & 0xFF])

    def ref16(self, op, target):
        self.b(op)
        if isinstance(target, str):
            self.fixups.append((len(self.code), target, "abs"))
            self.w(0)
        else:
            self.w(target)

    def jr(self, op, target):
        self.b(op)
        self.fixups.append((len(self.code), target, "rel"))
        self.b(0)

    def resolve(self):
        for off, name, kind in self.fixups:
            addr = self.labels[name]
            if kind == "abs":
                self.code[off:off + 2] = bytes([addr & 0xFF, addr >> 8])
            else:
                rel = addr - (self.origin + off + 1)
                assert -128 <= rel <= 127, f"jr to {name} out of range"
                self.code[off] = rel & 0xFF
        return bytes(self.code)


def main_loop(g):
    """The player loop, run from work RAM after the boot ROM is switched off."""
    a = Asm(STUB_RAM)
    a.b(0x3E, 0x01); a.b(0xE0, 0x50)                 # ld a,1 ; ldh [$FF50],a (boot ROM off)
    a.b(0x3E, 0x80); a.b(0xE0, 0x26)                 # ld a,$80 ; ldh [NR52],a (sound on)
    a.b(0x3E, g["init_bank"]); a.ref16(0xEA, 0x2100) # ld a,bank ; ld [$2100],a
    a.ref16(0xCD, g["init_addr"])                    # call SoundSystemInit
    a.b(0x3E, g["start_track"]); a.ref16(0xEA, VAR_TRACK)
    a.ref16(0xCD, "play")

    a.label("frame")
    a.label("wait_vbl")                              # wait for line 144
    a.b(0xF0, 0x44); a.b(0xFE, 144); a.jr(0x20, "wait_vbl")
    a.ref16(0x21, 0xFF00 | g["frame_counter_lo"]); a.b(0x34)   # ld hl,hFrameCounter ; inc [hl]
    a.ref16(0xCD, g["step_addr"])                    # call PlayAudioStep

    # Joypad: d-pad and buttons, 1 = pressed: b = {start,select,B,A,down,up,left,right}
    a.b(0x3E, 0x20); a.b(0xE0, 0x00)                 # select d-pad
    a.b(0xF0, 0x00); a.b(0xF0, 0x00)                 # read (twice, to settle)
    a.b(0x2F); a.b(0xE6, 0x0F); a.b(0x47)            # cpl ; and $0F ; ld b,a
    a.b(0x3E, 0x10); a.b(0xE0, 0x00)                 # select buttons
    a.b(0xF0, 0x00); a.b(0xF0, 0x00)
    a.b(0x2F); a.b(0xE6, 0x0F)                       # cpl ; and $0F
    a.b(0xCB, 0x37); a.b(0xB0); a.b(0x47)            # swap a ; or b ; ld b,a
    a.b(0x3E, 0x30); a.b(0xE0, 0x00)                 # deselect
    # newly pressed = b & ~previous
    a.ref16(0xFA, VAR_JOY); a.b(0x2F); a.b(0xA0); a.b(0x4F)   # ld a,[prev] ; cpl ; and b ; ld c,a
    a.b(0x78); a.ref16(0xEA, VAR_JOY)                # ld a,b ; ld [prev],a
    # Right: next track
    a.b(0x79); a.b(0xE6, 0x01); a.jr(0x28, "not_right")
    a.ref16(0xFA, VAR_TRACK); a.b(0xFE, g["last_track"]); a.jr(0x20, "inc")
    a.b(0x3E, g["first_track"] - 1)
    a.label("inc"); a.b(0x3C); a.ref16(0xEA, VAR_TRACK); a.ref16(0xCD, "play")
    a.label("not_right")
    # Left: previous track
    a.b(0x79); a.b(0xE6, 0x02); a.jr(0x28, "not_left")
    a.ref16(0xFA, VAR_TRACK); a.b(0xFE, g["first_track"]); a.jr(0x20, "dec")
    a.b(0x3E, g["last_track"] + 1)
    a.label("dec"); a.b(0x3D); a.ref16(0xEA, VAR_TRACK); a.ref16(0xCD, "play")
    a.label("not_left")
    # A: restart the track
    a.b(0x79); a.b(0xE6, 0x10); a.jr(0x28, "not_a")
    a.ref16(0xCD, "play")
    a.label("not_a")

    a.label("wait_end")                              # wait until line 144 is over
    a.b(0xF0, 0x44); a.b(0xFE, 144); a.jr(0x28, "wait_end")
    a.ref16(0xC3, "frame")

    a.label("play")                                  # request the current track
    a.ref16(0xFA, VAR_TRACK); a.ref16(0xEA, g["track_var"]); a.b(0xC9)
    return a.resolve()


def boot(stub):
    """Boot code at $0200: clear RAM, copy the loop to work RAM, start the LCD, jump."""
    a = Asm(0x0200)
    a.b(0xF3)                                        # di
    a.ref16(0x31, 0xDFFF)                            # ld sp,$DFFF
    a.ref16(0x21, 0xC000); a.ref16(0x01, 0x2000)     # clear $C000-$DFFF
    a.label("clr"); a.b(0xAF, 0x22, 0x0B, 0x78, 0xB1); a.jr(0x20, "clr")
    a.ref16(0x21, 0xFF80); a.b(0x06, 0x7F)           # clear HRAM $FF80-$FFFE
    a.label("clrh"); a.b(0xAF, 0x22, 0x05); a.jr(0x20, "clrh")
    a.ref16(0x21, "stub"); a.ref16(0x11, STUB_RAM); a.ref16(0x01, len(stub))
    a.label("cpy"); a.b(0x2A, 0x12, 0x13, 0x0B, 0x78, 0xB1); a.jr(0x20, "cpy")
    a.b(0x3E, 0x80); a.b(0xE0, 0x40)                 # LCD on (for line timing)
    a.ref16(0xC3, STUB_RAM)
    a.label("stub")
    code = a.resolve()
    return code + stub


def main():
    stub = main_loop(GAME)
    rom = bytearray(0x900)
    rom[0:3] = bytes([0xC3, 0x00, 0x02])             # jp $0200
    code = boot(stub)
    assert 0x200 + len(code) <= 0x900
    rom[0x200:0x200 + len(code)] = code
    open(sys.argv[1], "wb").write(rom)
    print(f"{sys.argv[1]}: loop {len(stub)} bytes at ${STUB_RAM:04X}, boot code {len(code) - len(stub)} bytes")


if __name__ == "__main__":
    main()
