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

# Pokemon Red / Blue (USA, Europe): pret/pokered. PlayMusic ($23A1: A = song id, C = audio
# bank), and each frame what the game's VBlank handler does: FadeOutAudio ($28CB), then the
# UpdateMusic routine of the bank in wAudioROMBank ($C0EF): Audio1 ($02:$5103), Audio2
# ($08:$5879, without the low-health alarm), Audio3 ($1F:$5177). Song ids from the sound
# header tables of the user's dumps.
POKEMON_RB_TRACKS = [
    (0xBA, 0x02, "Pallet Town"), (0xBD, 0x02, "Pokemon Center"), (0xC0, 0x02, "Gym"),
    (0xC3, 0x02, "Cities 1"), (0xC7, 0x02, "Cities 2"), (0xCA, 0x02, "Celadon City"),
    (0xCD, 0x02, "Cinnabar Island"), (0xD0, 0x02, "Vermilion City"), (0xD4, 0x02, "Lavender Town"),
    (0xD8, 0x02, "S.S. Anne"), (0xDB, 0x02, "Meet Prof. Oak"), (0xDE, 0x02, "Meet Rival"),
    (0xE1, 0x02, "Museum Guy"), (0xE5, 0x02, "Safari Zone"), (0xE8, 0x02, "Pokemon Healed"),
    (0xEB, 0x02, "Route 1"), (0xEF, 0x02, "Route 24"), (0xF3, 0x02, "Route 3"),
    (0xF7, 0x02, "Route 11"), (0xFB, 0x02, "Indigo Plateau"),
    (0xEA, 0x08, "Gym Leader Battle"), (0xED, 0x08, "Trainer Battle"), (0xF0, 0x08, "Wild Battle"),
    (0xF3, 0x08, "Final Battle"), (0xF6, 0x08, "Defeated Trainer"), (0xF9, 0x08, "Defeated Wild"),
    (0xFC, 0x08, "Defeated Gym Leader"),
    (0xC3, 0x1F, "Title Screen"), (0xC7, 0x1F, "Credits"), (0xCA, 0x1F, "Hall of Fame"),
    (0xCD, 0x1F, "Oak's Lab"), (0xD0, 0x1F, "Jigglypuff's Song"), (0xD2, 0x1F, "Bike Riding"),
    (0xD6, 0x1F, "Surfing"), (0xD9, 0x1F, "Game Corner"), (0xDC, 0x1F, "Intro Battle"),
    (0xE0, 0x1F, "Dungeon 1"), (0xE4, 0x1F, "Dungeon 2"), (0xE8, 0x1F, "Dungeon 3"),
    (0xEC, 0x1F, "Pokemon Mansion"), (0xF0, 0x1F, "Pokemon Tower"), (0xF3, 0x1F, "Silph Co."),
    (0xF6, 0x1F, "Meet Evil Trainer"), (0xF9, 0x1F, "Meet Female Trainer"),
    (0xFC, 0x1F, "Meet Male Trainer"),
]

POKEMON_RB = dict(
    init=lambda pc: bytes([0x3E, 0x77, 0xE0, 0x24, 0x3E, 0xFF, 0xE0, 0x25]),  # NR50, NR51
    start=lambda pc: bytes([0xCD, 0xA1, 0x23]),                              # call PlayMusic
    frame=lambda pc: bytes([
        0xCD, 0xCB, 0x28,             # call FadeOutAudio
        0xFA, 0xEF, 0xC0,             # ld a,[wAudioROMBank]
        0xE0, 0xB8,                   # ldh [hLoadedROMBank],a
        0xEA, 0x00, 0x20,             # ld [rROMB],a
        0xFE, 0x02, 0x20, 0x05,       # cp $02 ; jr nz,.audio2
        0xCD, 0x03, 0x51,             # call Audio1_UpdateMusic
        0x18, 0x0C,                   # jr .done
        0xFE, 0x08, 0x20, 0x05,       # .audio2: cp $08 ; jr nz,.audio3
        0xCD, 0x79, 0x58,             # call Audio2_UpdateMusic
        0x18, 0x03,                   # jr .done
        0xCD, 0x77, 0x51,             # .audio3: call Audio3_UpdateMusic
    ]),
    tracks=POKEMON_RB_TRACKS,
)

GAMES["pokemon-red"] = dict(POKEMON_RB, title="POKEMON RED", version=0, checksum=0x91E6, name="Pokemon Red")
GAMES["pokemon-blue"] = dict(POKEMON_RB, title="POKEMON BLUE", version=0, checksum=0x9D0A, name="Pokemon Blue")

# Pokemon Yellow (USA, Europe): pret/pokeyellow (the dump matches its build). One engine for
# every music bank: PlayMusic ($2211: A = song id, C = bank), and each frame FadeOutAudio
# ($27C2) and Audio1_UpdateMusic ($02:$509D), as the VBlank handler does (without the
# low-health alarm). Song id = (header address - $4000) / 3.
POKEMON_Y_TRACKS = [
    (0xBA, 0x02, "Pallet Town"), (0xBD, 0x02, "Pokemon Center"), (0xC0, 0x02, "Gym"),
    (0xC3, 0x02, "Cities 1"), (0xC7, 0x02, "Cities 2"), (0xCA, 0x02, "Celadon City"),
    (0xCD, 0x02, "Cinnabar Island"), (0xD0, 0x02, "Vermilion City"), (0xD4, 0x02, "Lavender Town"),
    (0xD8, 0x02, "S.S. Anne"), (0xDB, 0x02, "Meet Prof. Oak"), (0xDE, 0x02, "Meet Rival"),
    (0xE1, 0x02, "Museum Guy"), (0xE5, 0x02, "Safari Zone"), (0xE8, 0x02, "Pokemon Healed"),
    (0xEB, 0x02, "Route 1"), (0xEF, 0x02, "Route 24"), (0xF3, 0x02, "Route 3"),
    (0xF7, 0x02, "Route 11"), (0xFB, 0x02, "Indigo Plateau"),
    (0xEA, 0x08, "Gym Leader Battle"), (0xED, 0x08, "Trainer Battle"), (0xF0, 0x08, "Wild Battle"),
    (0xF3, 0x08, "Final Battle"), (0xF6, 0x08, "Defeated Trainer"), (0xF9, 0x08, "Defeated Wild"),
    (0xFC, 0x08, "Defeated Gym Leader"),
    (0xC3, 0x1F, "Title Screen"), (0xC7, 0x1F, "Credits"), (0xCA, 0x1F, "Hall of Fame"),
    (0xCD, 0x1F, "Oak's Lab"), (0xD0, 0x1F, "Jigglypuff's Song"), (0xD2, 0x1F, "Bike Riding"),
    (0xD6, 0x1F, "Surfing"), (0xD9, 0x1F, "Game Corner"), (0xDC, 0x1F, "Intro"),
    (0xDF, 0x1F, "Dungeon 1"), (0xE3, 0x1F, "Dungeon 2"), (0xE7, 0x1F, "Dungeon 3"),
    (0xEB, 0x1F, "Pokemon Mansion"), (0xEF, 0x1F, "Pokemon Tower"), (0xF2, 0x1F, "Silph Co."),
    (0xF5, 0x1F, "Meet Evil Trainer"), (0xF8, 0x1F, "Meet Female Trainer"),
    (0xFB, 0x1F, "Meet Male Trainer"),
    (0x99, 0x20, "Surfing Pikachu"), (0x9C, 0x20, "Jessie & James"), (0x9F, 0x20, "Unused Song"),
    (0xA3, 0x20, "GB Printer"),
]

GAMES["pokemon-yellow"] = dict(
    title="POKEMON YELLOW", version=0, checksum=0x047C, name="Pokemon Yellow",
    init=POKEMON_RB["init"],
    start=lambda pc: bytes([0xCD, 0x11, 0x22]),   # call PlayMusic
    frame=lambda pc: bytes([
        0xCD, 0xC2, 0x27,                         # call FadeOutAudio
        0x3E, 0x02, 0xE0, 0xB8, 0xEA, 0x00, 0x20, # bank 2 (hLoadedROMBank, rROMB)
        0xCD, 0x9D, 0x50,                         # call Audio1_UpdateMusic
    ]),
    tracks=POKEMON_Y_TRACKS,
)

# Pokemon Gold / Silver (USA, Europe) and Crystal (USA, Europe) (Rev 1): pret/pokegold and
# pret/pokecrystal (the dumps match their builds). The home bank's InitSound, PlayMusic (DE =
# song id) and UpdateSound (what VBlank calls) switch to the engine's bank themselves.
GSC_NAMES = [
    "Title Screen", "Route 1", "Route 3", "Route 12", "Magnet Train", "Kanto Gym Leader",
    "Kanto Trainer Battle", "Kanto Wild Battle", "Pokemon Center", "Hiker Encounter",
    "Lass Encounter", "Officer Encounter", "Heal", "Lavender Town", "Route 2", "Mt. Moon",
    "Show Me Around", "Game Corner", "Bicycle", "Hall of Fame", "Viridian City", "Celadon City",
    "Trainer Victory", "Wild Victory", "Gym Victory", "Mt. Moon Square", "Gym", "Pallet Town",
    "Pokemon Talk", "Prof. Oak", "Rival Encounter", "Rival After", "Surf", "Evolution",
    "National Park", "Credits", "Azalea Town", "Cherrygrove City", "Kimono Encounter",
    "Union Cave", "Johto Wild Battle", "Johto Trainer Battle", "Route 30", "Ecruteak City",
    "Violet City", "Johto Gym Leader", "Champion Battle", "Rival Battle", "Rocket Battle",
    "Prof. Elm", "Dark Cave", "Route 29", "Route 36", "S.S. Aqua", "Youngster Encounter",
    "Beauty Encounter", "Rocket Encounter", "Pokemaniac Encounter", "Sage Encounter",
    "New Bark Town", "Goldenrod City", "Vermilion City", "Pokemon Channel", "Poke Flute",
    "Tin Tower", "Sprout Tower", "Burned Tower", "Lighthouse", "Lake of Rage", "Indigo Plateau",
    "Route 37", "Rocket Hideout", "Dragon's Den", "Johto Wild (Night)", "Ruins of Alph Radio",
    "Capture", "Route 26", "Mom", "Victory Road", "Pokemon Lullaby", "Pokemon March",
    "Opening", "Opening 2", "Main Menu", "Ruins of Alph", "Rocket Overture", "Dancing Hall",
    "Bug Contest Ranking", "Bug Catching Contest", "Rocket Radio", "Printer", "Post Credits",
]
CRYSTAL_NAMES = GSC_NAMES + [
    "Clair", "Mobile Adapter Menu", "Mobile Adapter", "Buena's Password", "Mystic Man",
    "Crystal Opening", "Battle Tower", "Suicune Battle", "Battle Tower Lobby", "Mobile Center",
]


def gsc(init, play, update, names):
    return dict(
        init=lambda pc: bytes([0xCD, lo(init), hi(init)]),                          # call InitSound
        start=lambda pc: bytes([0x5F, 0x16, 0x00, 0xCD, lo(play), hi(play)]),      # ld e,a ; ld d,0 ; call PlayMusic
        frame=lambda pc: bytes([0xCD, lo(update), hi(update)]),                     # call UpdateSound
        first=1, tracks=[(i + 1, 0, n) for i, n in enumerate(names)],
    )


GAMES["pokemon-gold"] = dict(gsc(0x3D4F, 0x3D98, 0x3D6B, GSC_NAMES),
                             title="POKEMON_GLDAAUE", version=0, checksum=0x682D, name="Pokemon Gold")
GAMES["pokemon-silver"] = dict(gsc(0x3D4F, 0x3D98, 0x3D6B, GSC_NAMES),
                               title="POKEMON_SLVAAXE", version=0, checksum=0x0DAE, name="Pokemon Silver")
GAMES["pokemon-crystal-rev1"] = dict(gsc(0x3B4E, 0x3B97, 0x3B6A, CRYSTAL_NAMES),
                                     title="PM_CRYSTAL", version=1, checksum=0x18D2, name="Pokemon Crystal")
