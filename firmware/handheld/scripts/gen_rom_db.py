"""Generate a database used to identify ROM files (res/snesdb.bin, res/nesdb.bin).

The data comes from libretro-database's No-Intro DATs (metadat/no-intro/,
https://github.com/libretro/libretro-database), whose names are also the names of
libretro's box art. Entries are keyed by the CRC32 of the ROM without any file header
(an SNES copier header, an iNES header).

Usage: python3 scripts/gen_rom_db.py path/to/system.dat out.bin [--ext .unh]

  SNES: "Nintendo - Super Nintendo Entertainment System.dat" -> res/snesdb.bin
  NES:  "Nintendo - Nintendo Entertainment System.dat" --ext .unh -> res/nesdb.bin
        (the DAT lists each game twice; the .unh entry is the ROM without its header)
  Genesis: "Sega - Mega Drive - Genesis.dat" -> res/genesisdb.bin

Format (little endian):
    header:  b"SNDB", u32 version, u32 record count, u32 strings offset
    record:  u32 ROM CRC32, u32 ROM size, u32 name offset, u16 name length,
             u16 reserved  (16 bytes; records are sorted by CRC32)
    strings: UTF-8 No-Intro names (the ROM file name without its extension),
             e.g. "Super Mario World (USA)"
"""

import re
import struct
import sys
from pathlib import Path

VERSION = 1
HEADER = struct.Struct("<4sIII")
RECORD = struct.Struct("<IIIHH")
ROM = re.compile(r'rom \( name "(.*?)" size (\d+) crc ([0-9A-Fa-f]{8})')


def main() -> None:
    dat = Path(sys.argv[1]).read_text(encoding="utf-8")
    out = Path(sys.argv[2])
    ext = sys.argv[4] if len(sys.argv) > 4 and sys.argv[3] == "--ext" else None

    games = {}
    for name, size, crc in ROM.findall(dat):
        if ext is not None and not name.endswith(ext):
            continue
        crc = int(crc, 16)
        if crc in games:
            raise SystemExit(f"CRC collision: {name} and {games[crc][0]}")
        games[crc] = (name.rsplit(".", 1)[0], int(size))

    strings = bytearray()
    records = bytearray()
    for crc in sorted(games):
        name, size = games[crc]
        data = name.encode("utf-8")
        records += RECORD.pack(crc, size, len(strings), len(data), 0)
        strings += data

    strings_offset = HEADER.size + len(records)
    out.write_bytes(HEADER.pack(b"SNDB", VERSION, len(games), strings_offset) + records + strings)
    print(f"{out}: {out.stat().st_size} bytes, {len(games)} games")


if __name__ == "__main__":
    main()
