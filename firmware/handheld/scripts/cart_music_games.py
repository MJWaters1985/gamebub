"""
Cartridge music entries: one per game revision, identified by the cartridge
header (title, version byte $14C, global checksum $14E-$14F).

Each entry has glue code (functions returning SM83 bytes, given the address the
code is placed at):
- init:  start the sound driver (once)
- start: start a track, with the track's parameters in A (p0) and C (p1)
- frame: run the driver for one frame
and `tracks`: a list of (p0, p1, name).

Addresses come from community disassemblies, checked against dumps of the
user's own cartridges.
"""


def lo(v):
    return v & 0xFF


def hi(v):
    return (v >> 8) & 0xFF


# The Legend of Zelda: Link's Awakening DX (USA, Europe) (Rev 2)
# zladx/LADX-Disassembly: SoundSystemInit (bank $1F, $4000), PlayAudioStep ($089F in
# Rev 2), wMusicTrackToPlay ($D368), hFrameCounter ($FFE7), MUSIC_* track ids.
LADX_NAMES = [
    "Title Screen", "Minigame", "Game Over", "Mabe Village", "Overworld", "Tal Tal Range",
    "Shop", "Raft Ride Rapids", "Mysterious Forest", "Inside Building", "Animal Village",
    "Fairy Fountain", "Title (No Intro)", "BowWow Kidnapped", "Obtain Sword", "Obtain Item",
    "File Select", "Egg Maze", "Kanalet Castle", "Tail Cave", "Bottle Grotto", "Key Cavern",
    "Angler's Tunnel", "After Boss", "Boss", "Title Cutscene", "Obtain Instrument",
    "Intro Wake Up", "Overworld (No Sword)", "Dream Shrine Sleep", "Southern Shrine",
    "Full Moon Cello", "2D Underground", "Owl", "Final Boss", "Dream Shrine Bed",
    "Heart Container", "Cave", "Obtain Power-Up", "Conch Horn", "Sea Lily's Bell",
    "Surf Harp", "Wind Marimba", "Coral Triangle", "Organ of Evening", "Thunder Drum",
    "Marin Sings", "Manbo's Mambo", "Overworld Intro", "Mr. Write's House", "Ulrira",
    "Tarin and the Bees", "Mamu Frog Song", "Monkeys Bridge", "Christine's House",
    "Totaka (Unused)", "Turtle Rock Boss", "Fisherman", "Obtain Item (Unused)",
    "File Select (Totaka)", "Ending", "Moblin Hideout", "Island Disappears",
    "Richard's House", "Ballad (Default)", "Ballad (Bell)", "Ballad (Harp)",
    "Ballad (Marimba)", "Ballad (Triangle)", "Ballad (Organ)", "Ballad of Wind Fish",
    "Ghost House", "Active Power-Up", "Learn the Ballad", "Catfish's Maw",
    "Open Angler's Tunnel", "Marin on the Beach", "Marin Beach Talk", "Marin (Unused)",
]
assert len(LADX_NAMES) == 0x4F

GAMES = {
    "ladx-rev2": dict(
        title="ZELDA", version=2, checksum=0x0135,
        name="Link's Awakening DX",
        init=lambda pc: bytes([0x3E, 0x1F, 0xEA, 0x00, 0x21, 0xCD, 0x00, 0x40]),  # SoundSystemInit
        start=lambda pc: bytes([0xEA, lo(0xD368), hi(0xD368)]),                   # ld [wMusicTrackToPlay],a
        frame=lambda pc: bytes([0x21, 0xE7, 0xFF, 0x34, 0xCD, 0x9F, 0x08]),       # inc [hFrameCounter]; call PlayAudioStep
        first=1,
        tracks=[(i + 1, 0, n) for i, n in enumerate(LADX_NAMES)],
    ),
}
