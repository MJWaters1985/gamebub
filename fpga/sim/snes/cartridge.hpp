#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

/// An SNES cartridge image, with the mapping configuration read from its header.
class Cartridge {
public:
    explicit Cartridge(std::filesystem::path rom_path);

    /// ROM contents (copier header removed).
    std::vector<uint8_t> rom;

    /// Mapping type: bits 3:0 are 0 = LoROM, 1 = HiROM, 2 = ExHiROM;
    /// bits 7:4 select an enhancement chip.
    uint8_t rom_type = 0;
    /// ROM size as 1 KiB << rom_size.
    uint8_t rom_size = 0;
    /// Battery RAM size as 1 KiB << ram_size, or 0 for none.
    uint8_t ram_size = 0;
    bool pal = false;

    uint32_t rom_mask() const { return (1024u << rom_size) - 1; }
    uint32_t ram_mask() const { return ram_size ? (1024u << ram_size) - 1 : 0; }
    std::string description() const;
};
