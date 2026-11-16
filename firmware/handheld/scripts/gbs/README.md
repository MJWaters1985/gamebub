# Cartridge music from GBS rips (prototype)

Turns GBS music rips into cartridge music players (`../gen_music_player.py`), checked
against a dump of the cartridge.

- `gbs2entry.py`: traces the rip in an SM83 emulator (`sm83emu.py`), maps the rip's banks
  to the ROM's (`banks.py`), moves the ripper's code and data (the bytes that differ from the
  ROM) to work RAM with its references patched, routes its bank switches through a rip-to-ROM
  bank table, finds free RAM for the player, and takes track names from the rip's playlists.
- `verify.py`: runs the entry's code on the ROM and compares every sound register write with
  the rip's, frame by frame (cartridge RAM starts as garbage, as on a real cartridge); tries
  repairs, such as constants that are rip bank numbers.
- `pipeline.py`: convert, verify, repair and generate players for every rip.
- `playeremu.py`: runs a whole generated player over a ROM, for debugging.

Set `CART_MUSIC_WORK` (rips in `gbs-dl/x/<set>/`; players and results go in `gbs/`) and
`CART_MUSIC_ROMS` (folders with ROM dumps, `:`-separated).

Status (37 rips with ROMs): 22 verified players, 21 play in the FPGA simulator (Kirby's
Pinball Land hangs in the game's init on the core but not in the emulator).
