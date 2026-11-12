#include "cartridge.hpp"

#include <algorithm>
#include <cstdio>

#include "common.hpp"

namespace {

struct HeaderInfo {
    int score = 0;
    uint8_t chip_type = 0;
    uint8_t ram_size = 0;
    bool pal = false;
};

/// Score a possible internal header at `offset` (the address of $FFC0 in that
/// mapping), as in the Analogue Pocket port's rom_parser.sv.
HeaderInfo score_header(const std::vector<uint8_t>& rom, size_t offset, int kind)
{
    HeaderInfo info;
    if (offset + 0x40 > rom.size()) {
        return info;
    }
    auto byte = [&](size_t i) { return rom[offset + i]; };
    uint8_t mapping_mode = byte(0x15);
    uint8_t rom_type = byte(0x16);
    uint8_t rom_size = byte(0x17);
    uint8_t sram_size = byte(0x18);
    uint8_t region = byte(0x19);
    uint8_t dev_id = byte(0x1A);
    uint16_t complement = byte(0x1C) | (byte(0x1D) << 8);
    uint16_t checksum = byte(0x1E) | (byte(0x1F) << 8);
    uint8_t gsu_ram_size = rom[offset - 3]; // $FFBD

    int score = 0;
    if (checksum != 0 && complement != 0 && uint16_t(checksum + complement) == 0xFFFF) score += 4;
    if (dev_id == 0x33) score += 2;
    if (rom_type < 8) score += 1;
    if (rom_size < 16) score += 1;
    if (sram_size < 8) score += 1;
    if (region < 14) score += 1;

    uint8_t chip = 0;
    uint8_t ramsz = sram_size;
    if (mapping_mode == 0x20 && rom_type == 0x03) chip |= 0x84;               // DSP1
    else if (mapping_mode == 0x21 && rom_type == 0x03) chip |= 0x80;          // DSP1B
    else if (mapping_mode == 0x30 && rom_type == 0x05 && dev_id != 0xB2) chip |= 0x80; // DSP1B
    else if (mapping_mode == 0x31 && (rom_type == 0x03 || rom_type == 0x05)) chip |= 0x80; // DSP1B
    else if (mapping_mode == 0x20 && rom_type == 0x05) chip |= 0x90;          // DSP2
    else if (mapping_mode == 0x30 && rom_type == 0x05 && dev_id == 0xB2) chip |= 0xA0; // DSP3
    else if (mapping_mode == 0x30 && rom_type == 0x03) chip |= 0xB0;          // DSP4
    else if (mapping_mode == 0x30 && rom_type == 0xF6) {                      // ST010/ST011
        chip |= 0x88;
        if (rom_size < 10) chip |= 0x20;
        ramsz = 1;
    } else if (mapping_mode == 0x30 && rom_type == 0x25) chip |= 0xC0;        // OBC1

    if (mapping_mode == 0x3A && (rom_type == 0xF5 || rom_type == 0xF9)) {     // SPC7110
        chip |= 0xD0;
        if (rom_type == 0xF9) chip |= 0x08;
    }
    if (mapping_mode == 0x35 && rom_type == 0x55) chip |= 0x08;               // S-RTC
    if (mapping_mode == 0x20 && rom_type == 0xF3) chip |= 0x40;               // CX4
    if (mapping_mode == 0x32 && (rom_type == 0x43 || rom_type == 0x45) && rom_size < 14) chip |= 0x50; // SDD1
    if (mapping_mode == 0x23 && (rom_type == 0x32 || rom_type == 0x34 || rom_type == 0x35)) chip |= 0x60; // SA1
    if (mapping_mode == 0x20 && (rom_type == 0x13 || rom_type == 0x14 || rom_type == 0x15 || rom_type == 0x1A)) {
        // GSU
        ramsz = gsu_ram_size == 0xFF ? 5 : (gsu_ram_size > 6 ? 6 : gsu_ram_size);
        chip |= 0x70;
    }

    bool pal = (region >= 0x02 && region <= 0x0C) || region == 0x11;

    if (kind == 0 && (mapping_mode == 0x20 || mapping_mode == 0x22)) score += 2;
    if (kind == 1 && mapping_mode == 0x21) score += 2;
    if (kind == 2) {
        if (mapping_mode == 0x25 || mapping_mode == 0x35) score += 2;
        if (score != 0) score += 4;
    }

    info.score = score;
    info.chip_type = chip;
    info.ram_size = ramsz;
    info.pal = pal;
    return info;
}

} // namespace

Cartridge::Cartridge(std::filesystem::path rom_path)
{
    rom = read_file(rom_path);
    // Copier header: 512 extra bytes at the start.
    if ((rom.size() & 0x3FF) == 0x200) {
        rom.erase(rom.begin(), rom.begin() + 0x200);
    }

    rom_size = 0;
    while ((1024u << rom_size) < rom.size() && rom_size < 15) {
        rom_size++;
    }

    HeaderInfo lorom = score_header(rom, 0x7FC0, 0);
    HeaderInfo hirom = score_header(rom, 0xFFC0, 1);
    HeaderInfo exhirom = score_header(rom, 0x40FFC0, 2);

    HeaderInfo chosen = lorom;
    rom_type = 0;
    if (lorom.score >= hirom.score && lorom.score >= exhirom.score) {
        chosen = lorom;
        rom_type = 0;
    } else if (hirom.score >= exhirom.score) {
        chosen = hirom;
        rom_type = 1;
    } else if (exhirom.score > 0) {
        chosen = exhirom;
        rom_type = 2;
    }
    rom_type |= chosen.chip_type;
    ram_size = chosen.ram_size > 8 ? 8 : chosen.ram_size;
    pal = chosen.pal;
}

std::string Cartridge::description() const
{
    static const char* maps[] = {"LoROM", "HiROM", "ExHiROM", "?"};
    char text[128];
    std::snprintf(text, sizeof(text), "%s rom_type=0x%02x rom=%zu KiB ram=%d KiB %s",
        maps[std::min(rom_type & 0xF, 3)], rom_type, rom.size() / 1024,
        ram_size ? (1 << ram_size) : 0, pal ? "PAL" : "NTSC");
    return text;
}
