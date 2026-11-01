# Changes in this fork

This is an unofficial fork of [elipsitz/gamebub](https://github.com/elipsitz/gamebub),
based on the official **1.0.3** release, for the Rev 4 handheld. Issue numbers link to
the official project's issue tracker.

Firmware built from this fork shows its version as
`<official version>.mod-<commit date> (<commit>)`, e.g. `1.0.3.mod-20261101 (3711c01)`,
in **Settings → About**. Builds with uncommitted changes add `-dirty` to the commit.

## Cartridge and game compatibility

### Physical GBA cartridges

- **Slow and large cartridge ROMs.** The cartridge read timing at 3 wait states (the
  setting most games use) gave the cartridge too little time. Some cartridges need a
  longer read strobe (Super Mario Advance 2 hung at a white screen). Others need more
  time between chip select and read strobe; with less, they return all 1s (Pokémon
  Emerald, Lunar Legend and Donkey Kong Country lost the "Nintendo" logo and stopped at
  the BIOS screen). No split of the 3 available cycles works for both kinds of ROM, so
  these reads take one extra cycle, the same timing as 4 wait states. Other wait state
  settings are unchanged. Tested on hardware with Super Mario Advance 2, Pokémon Emerald,
  Pokémon Ruby, Lunar Legend and Donkey Kong Country.
- **Address lines A16–A23** stay driven for the whole cartridge access, like on a GBA.
  Previously they floated for half a cycle at the end of each access.
- **Power-up delay.** The core waits until the cartridge has been powered for 300 ms
  before starting, so the BIOS's first reads don't happen during power-up.

### Game Boy / Game Boy Color core

- **Kirby's Dream Land 2 white screen**
  ([#62](https://github.com/elipsitz/gamebub/issues/62)). When `HALT` ended with
  interrupts disabled, the CPU never sent the next opcode fetch to the cartridge. It read
  the stale `HALT` opcode from the bus and halted forever. The CPU now keeps reading the
  opcode at PC while halted, like the hardware.
- **HALT bug implemented.** `HALT` with IME=0 and an interrupt pending reads the next
  byte twice, as on real hardware. The unused IF bits read as 1, and IE stores all 8
  bits. Blargg's `halt_bug` test now passes.
- **DMG games switching to double speed.** In GB (DMG) mode, the CGB-only registers
  `KEY1`, `SVBK` and `VBK` were still writable. A DMG game writing `KEY1` before `STOP`
  could switch to double speed, and bank writes could corrupt memory. Blargg's
  `cpu_instrs` now passes in DMG mode.

### ROM files and saves

- **Shantae Advance: Risky Revolution** saves correctly
  ([#28](https://github.com/elipsitz/gamebub/issues/28)). It is in the game database as
  SRAM/FRAM, because autodetection found EEPROM library strings.
- **Homebrew save type** is taken from the game code when it follows the homebrew
  convention (codes starting with 1–4)
  ([#68](https://github.com/elipsitz/gamebub/issues/68)).
- **GBA config override files** (`.json` next to the ROM) are found. Previously the
  extension lost its first character.

## New features

- **Wi-Fi** (Settings → Wi-Fi). The radio was unused; Wi-Fi is only turned on for a task and
  then off again.
  - **Network**: scan for networks and enter the password with an on-screen keyboard (D-pad
    to move, A to type, B to delete, X for shift, Y for symbols, Start when done). The network
    is only saved if the connection works. Also test the connection or forget the network.
    The password is stored in the device's settings memory (not encrypted).
  - **Sync Clock on Connect**: set the clock from the internet whenever Wi-Fi connects.
  - **Box Art After Dump**: the Cartridge Manager downloads box art after dumping a ROM.
- **Download Box Art** (Tools): downloads box art for every game in `/roms` that doesn't
  have it, from the [libretro thumbnail library](https://thumbnails.libretro.com) via the
  [images.weserv.nl](https://images.weserv.nl) image proxy (only the game's name is sent),
  saved as 120×120 thumbnails in `/boxart/<system>/`.
- **Run ROM** on the home screen (between Run Cartridge and Cores): pick a system, then a
  game from `/roms/Game Boy`, `/roms/Game Boy Color` or `/roms/Game Boy Advance`, sorted by
  name, with its box art (from `/boxart/<system>/<game>.thumb`). Games start directly with
  the right core, without the file browser. Remembers the last system and game.
- **Cartridge Manager** (Tools → Cartridge Manager): work with physical cartridges on the
  device, without a computer.
  - **Identify**: shows the game's name (from a built-in database of about 6,000 Game Boy,
    Game Boy Color and Game Boy Advance games, based on FlashGBX's No-Intro data), system,
    ROM size, mapper and save type.
  - **Dump ROM to SD card**: saves to `/roms/<system>/<name>.<ext>` (e.g.
    `/roms/Game Boy Advance/Pokemon - Emerald Version (USA, Europe).gba`) and checks the dump
    against the database.
  - **Back up save**: the save becomes the dumped ROM's save (following the Save Files
    setting; the previous one is kept as `.bak`), and a dated copy is kept in
    `/saves/backups/<game>/`.
  - **Restore save to cartridge**: pick the current save or a dated copy. The cartridge's
    save is backed up first, and the written save is read back to verify it.
  - Supports Game Boy cartridges without a mapper or with MBC1, MBC2, MBC3 or MBC5, and GBA
    cartridges with SRAM/FRAM, Flash (64 or 128 KiB) or EEPROM (4 or 64 Kbit). Real-time
    clock data isn't backed up yet.
  - **Cartridge clock**: for cartridges with a real-time clock, the details show the clock,
    and **Set cartridge clock to now** sets it to the local time.
    - GBA (Seiko S-3511, e.g. Pokémon Ruby, Sapphire and Emerald): sets the date and time.
      A clock that lost power (dead or replaced battery) is reset first. Tested with
      Pokémon Emerald.
    - Game Boy MBC3 (e.g. Pokémon Gold, Silver and Crystal): sets the time of day and keeps
      the day counter, since these games keep their own offset from it, so the in-game time
      still needs resetting in the game (in Pokémon Gold / Silver: Down + Select + B on the
      title screen). The save is backed up first, because the clock is written through the
      save memory area. Tested with Pokémon Gold.
    - The clock is read back after setting it. Access follows FlashGBX.
- **Time Zone** (Settings → General). Choose a time zone (or a fixed UTC offset); daylight
  saving time changes automatically. The device clock still keeps UTC; local time is used
  for Date and Time in Settings, the clock of GBA games without a saved clock, save backup
  names, file times on the SD card and the clock sync message.
- **Set Up SD Card** (Tools). Game Bub reads FAT32 (or FAT16) cards, not exFAT.
  - **Create missing folders**: `/roms/<system>`, `/boxart/<system>`, `/saves`,
    `/saves/backups`, `/system` and `/settings`. Existing files are kept.
  - **Check ROMs**: reads every ROM in `/roms` and compares it with the database, to find
    damaged files or an unreliable card. Only reads.
  - **Erase and format card** (hold A for 3 seconds): clears the old partition table
    (including GPT and its backup copy), writes an MBR with one FAT32 partition starting at
    4 MiB (the standard alignment) with 32 KiB clusters, then creates the folders. Custom BIOS files in `/system`
    (`gameboy.bios-dmg.bin`, `gameboy.bios-cgb.bin`, `gba.bios.bin`, `gba.bios-fast.bin`) are
    kept; if one can't be read, nothing is erased. Works on cards Game Bub can't read, such as
    new exFAT cards.
  - If the card couldn't be mounted at startup, the screen shows why.
- **File browser.**
  - Natural, case-insensitive sorting
    ([#65](https://github.com/elipsitz/gamebub/issues/65)).
  - Folders of any size, with paging and the correct first page
    ([#18](https://github.com/elipsitz/gamebub/issues/18)).
  - Case-insensitive file extensions.
  - **Select** marks a favorite ([#44](https://github.com/elipsitz/gamebub/issues/44)).
  - **Start** opens the core's settings before loading a ROM
    ([#55](https://github.com/elipsitz/gamebub/issues/55)).
  - **B** at the top folder goes back.
- **Load ROM** in the in-game menu picks another file for the same core
  ([#29](https://github.com/elipsitz/gamebub/issues/29)).
- **Safer saves.** Saves are written to a temporary file and then swapped in, keeping the
  previous save as a `.bak`, which is used if the main save can't be read.
- New settings:

  | Setting | Where | Choices | Issue |
  |---|---|---|---|
  | Save Files | General | Next to ROM, `/saves` folder (a subfolder per ROM file type, e.g. `/saves/gba`) | [#17](https://github.com/elipsitz/gamebub/issues/17) |
  | Auto Power Off | General | Never, 3, 5, 10 or 30 min, with a warning first | [#37](https://github.com/elipsitz/gamebub/issues/37) |
  | X/Y as Select/Start | General | On, off | [#16](https://github.com/elipsitz/gamebub/issues/16) |
  | Game Color Temperature | General | Default, Warm, Warmer, Cool | [#24](https://github.com/elipsitz/gamebub/issues/24) |
  | Integer Overscale (crop) | Core: GB / GBC | On, off (one scale step larger, cropping top and bottom) | [#30](https://github.com/elipsitz/gamebub/issues/30) |
  | Tilt Sensitivity | Core: GB / GBC and Core: GBA | Lowest, Low, Normal, High | [#63](https://github.com/elipsitz/gamebub/issues/63) |

## Bug fixes

- **Lists that don't wrap around now scroll** to the highlighted item (e.g. the Cartridge
  Manager's actions, Wi-Fi networks, saves to restore). Previously the highlight could move
  out of view.

- **WarioWare: Twisted crash**
  ([#61](https://github.com/elipsitz/gamebub/issues/61)). A transient error reading the
  gyro or accelerometer crashed the device. The sample is now skipped instead.
- **Game Boy Player rumble stopping** (Drill Dozer,
  [#59](https://github.com/elipsitz/gamebub/issues/59)). An occasional bad transfer
  dropped the rumble connection until the game restarted. Rumble now tolerates errors and
  reconnects.
- **Headphones ignored after power cycling**
  ([#57](https://github.com/elipsitz/gamebub/issues/57)). Powering down the DAC no
  longer disables the headphone detection bias, and detection is rechecked after boot.
- **USB drive mode failing after a firmware update**
  ([#39](https://github.com/elipsitz/gamebub/issues/39)). The USB peripheral is reset at
  boot, and setup is retried instead of crashing.
- **Home staying pressed** after opening the power menu with Home + Power
  ([#36](https://github.com/elipsitz/gamebub/issues/36)).
- **Core screen title stuck on "Select ROM"**
  ([#56](https://github.com/elipsitz/gamebub/issues/56)).
- **Last core in the list scrolled out of view**
  ([#66](https://github.com/elipsitz/gamebub/issues/66)).

## Developer tooling

- [Local build guide](docs/building-local.md) for the firmware, FPGA cores, simulators
  and UF2 packaging ([#27](https://github.com/elipsitz/gamebub/issues/27)).
- Verilator simulator:
  - Headless mode with screenshots and scripted input.
  - DMG/CGB selection from the boot ROM.
  - Physical-cartridge and slow-ROM bus models (`SIM_PHYSICAL_CART`, `SIM_SLOW_CART`).
  - Linux build fixes.
- `fpga/scripts/run_test_roms.py` runs test ROM suites against reference screenshots.
- **More free memory** (the ESP32-S3 has no external RAM; the heap was down to ~100 KB in
  ~30 KB pieces): the Wi-Fi driver's code is kept in flash instead of internal RAM (+17 KB),
  and the 16 KB bulk transfer buffer is allocated only while it's used, with smaller
  fallbacks (+16 KB). Longer jobs run on one background thread started at boot. Wi-Fi uses
  plain HTTP with small buffers; box art comes as small JPEGs decoded by the TJpgDec
  decoder in the ESP32-S3 ROM (PNG decoding needs a 32 KB window in one piece), and is
  loaded and shown in ~8 KB strips.
- `firmware/handheld/scripts/gen_game_db.py` builds the Cartridge Manager's game database
  (`res/gamedb.bin`) from FlashGBX's database files.
- **Cartridge read log.** If `/sdcard/cartlog.enable` exists, the GBA core's log of
  physical cartridge header reads is written to `/sdcard/cartlog.bin` when the in-game
  menu opens. `fpga/scripts/parse_cart_log.py` compares it against a ROM dump.

## Credits

The physical-cartridge read timing work and simulation model build on
[ghogan42/gamebub](https://github.com/ghogan42/gamebub). The Cartridge Manager uses the
cartridge protocol, mapper and save memory handling of
[FlashGBX](https://github.com/lesserkuma/FlashGBX) by Lesserkuma, and its game database
(GPL-3.0).
