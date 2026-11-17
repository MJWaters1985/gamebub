# Cartridge music from GBS rips

Turns GBS music rips into cartridge music players (`../gen_music_player.py`) that drive the
sound engine on the cartridge itself.

## Packages for the device (no ROM needed)

- `package.py`: traces a rip in an SM83 emulator (`sm83emu.py`, `gbs2entry.py`), moves the
  ripper's code to work RAM (bank 0 from the load address, or only its first $100 bytes when
  that doesn't fit; the start of bank 1 when init or play is there), and writes a package
  (`.cmr`, format in the file's docstring): the player, a short probe per rip bank, CRCs of
  everything else the rip runs or reads, the rst handlers it uses (checked against the
  cartridge's own at $0000) and the bank numbers in the engine's data. The device
  (`src/cart_tool/rips.rs`) finds each rip bank among the cartridge's banks, checks the rest,
  fills in the player's rip-bank -> ROM-bank table and refuses a cartridge that doesn't match.
- `package_all.py`: every rip in `gbs-dl/x`, in parallel: `<out>/*.cmr`, `<out>/index.txt`
  (normalized title -> package; copy both to `/system/cart-music/rips/` on the SD card) and
  `gbs/package_all.json` (why the others weren't packaged).
- `testpkg.py [name]`: packages rips that have a ROM, matches them as the device does and
  compares the sound register writes with the rip's.
- `rom_report.py <package dir> <out.tsv>`: the same for every ROM in `CART_MUSIC_ROMS`, by
  the ROM's No-Intro name as the device looks it up: plays / some songs differ / refused (why) /
  rip not packaged (why) / no rip.

## Players made with a ROM

- `gbs2entry.py`: maps the rip's banks to the ROM's (`banks.py`) and moves the bytes that
  differ from the ROM.
- `verify.py`: runs an entry's code on the ROM and compares every sound register write with
  the rip's, frame by frame (cartridge RAM and the Color's extra work RAM start as garbage).
- `pipeline.py`: convert, verify, repair and generate players for every rip with a ROM.
- `playeremu.py`: runs a whole generated player over a ROM, for debugging.

Set `CART_MUSIC_WORK` (rips in `gbs-dl/x/<set>/`; results go in `gbs/`) and
`CART_MUSIC_ROMS` (folders with ROM dumps, `:`-separated).

Known limits: rips whose banks the ripper assembled from pieces of several ROM banks
(Donkey Kong Land) can't be matched to a cartridge; Kirby's Pinball Land hangs in the game's
init on the FPGA core but not in the emulator.
