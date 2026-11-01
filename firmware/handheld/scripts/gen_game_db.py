"""Generate res/gamedb.bin, the game database used to identify physical cartridges.

The data comes from FlashGBX's databases (config/db_DMG.json and config/db_AGB.json,
https://github.com/lesserkuma/FlashGBX, GPL-3.0), which are based on No-Intro. Entries
are keyed by the SHA-1 of the cartridge header (the first 0x180 bytes of the ROM).

Usage: python3 scripts/gen_game_db.py path/to/FlashGBX/config res/gamedb.bin

Format (little endian):
    header:  b"GBDB", u32 version, then per platform (DMG, AGB): u32 offset, u32 count;
             then u32 strings offset
    record:  8-byte key (start of the header SHA-1), u32 ROM CRC32, u32 ROM size,
             u32 name offset, u16 name length, u8 save type (AGB, FlashGBX numbering),
             u8 reserved  (24 bytes; records are sorted by key)
    strings: UTF-8 names, e.g. "Pokemon - Emerald Version (USA, Europe)"
"""

import json
import struct
import sys
from pathlib import Path

VERSION = 1
RECORD = struct.Struct("<8sIIIHBB")


def main() -> None:
    config = Path(sys.argv[1])
    out = Path(sys.argv[2])

    strings = bytearray()
    names = {}

    def name_ref(name: str) -> tuple[int, int]:
        if name not in names:
            data = name.encode("utf-8")
            names[name] = (len(strings), len(data))
            strings.extend(data)
        return names[name]

    tables = []
    for platform in ("DMG", "AGB"):
        db = json.loads((config / f"db_{platform}.json").read_text(encoding="utf-8"))
        records = {}
        for sha1, entry in db.items():
            key = bytes.fromhex(sha1)[:8]
            if key in records:
                raise SystemExit(f"{platform}: key collision for {sha1}")
            name = entry["gn"] + (" " + entry["ne"] if entry.get("ne") else "")
            offset, length = name_ref(name)
            records[key] = RECORD.pack(
                key, entry.get("rc", 0), entry.get("rs", 0), offset, length, entry.get("st", 0), 0
            )
        tables.append(b"".join(records[k] for k in sorted(records)))

    header_size = 4 + 4 + 2 * 8 + 4
    offset = header_size
    header = bytearray(b"GBDB") + struct.pack("<I", VERSION)
    for table in tables:
        header += struct.pack("<II", offset, len(table) // RECORD.size)
        offset += len(table)
    header += struct.pack("<I", offset)

    out.write_bytes(bytes(header) + b"".join(tables) + bytes(strings))
    print(f"{out}: {out.stat().st_size} bytes, "
          f"{sum(len(t) for t in tables) // RECORD.size} games")


if __name__ == "__main__":
    main()
