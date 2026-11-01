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
