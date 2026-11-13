//! Driver for the SNES FPGA module (a port of the MiSTer SNES core).

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use esp_idf_svc::hal::units::Hertz;

use crate::{
    core::{CoreFile, CoreHandler, CoreInfo},
    device::{drivers::fpga, Device},
};

use super::{util::color_correction, Bitstream};

/// SNES master clock (21.477 MHz NTSC; the FPGA's clock runs 0.03% fast).
const SYSTEM_CLOCK_RATE: Hertz = Hertz(21_484_375);

const REG_ROM_TYPE: u32 = 0x0000_0000;
const REG_ROM_MASK: u32 = 0x0000_0004;
const REG_RAM_MASK: u32 = 0x0000_0008;
const REG_RAM_SIZE: u32 = 0x0000_000C;
const REG_PAL: u32 = 0x0000_0010;
const COLOR_CORRECTION_BASE: u32 = 0x5000_0000;

const FILE_ROM: u16 = 0;
const FILE_SAVE: u16 = 1;

/// Size of a copier (SMC) header.
const COPIER_HEADER_SIZE: u64 = 512;
/// Largest battery RAM size (1 KiB << n) the core supports.
const MAX_RAM_SIZE: u8 = 8;

/// Cartridge mapping, read from the ROM's internal header.
#[derive(Clone, Copy, Debug, Default)]
struct CartridgeConfig {
    /// Bits 3:0: 0 = LoROM, 1 = HiROM, 2 = ExHiROM. Bits 7:4: enhancement chip.
    rom_type: u8,
    /// ROM size as 1 KiB << rom_size.
    rom_size: u8,
    /// Battery RAM size as 1 KiB << ram_size, or 0 for none.
    ram_size: u8,
    pal: bool,
}

impl CartridgeConfig {
    /// The bitstream that has this cartridge's enhancement chip, or the chip's name if
    /// there's none.
    fn bitstream(&self) -> Result<&'static str, &'static str> {
        match self.rom_type >> 4 {
            0x5 => Ok("snes_sdd1.bit.hs"),
            0xD => Err("SPC7110"),
            // None, DSP-1/2/3/4, ST010/011, CX4, SA-1, Super FX, OBC1, S-RTC
            _ => Ok("snes.bit.hs"),
        }
    }

    fn rom_mask(&self) -> u32 {
        (1024u32 << self.rom_size) - 1
    }

    fn ram_bytes(&self) -> u32 {
        if self.ram_size == 0 {
            0
        } else {
            1024u32 << self.ram_size
        }
    }

    fn ram_mask(&self) -> u32 {
        self.ram_bytes().saturating_sub(1)
    }
}

/// Score of a possible internal header, as in the Analogue Pocket port's
/// rom_parser.sv (which follows MiSTer's loader).
#[derive(Clone, Copy, Debug, Default)]
struct HeaderScore {
    score: u32,
    chip_type: u8,
    ram_size: u8,
    pal: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mapping {
    LoRom,
    HiRom,
    ExHiRom,
}

/// Score a header candidate. `data` holds the 0x50 bytes from $FFB0 (in that
/// mapping) onwards, so the header proper ($FFC0) starts at offset 0x10.
fn score_header(data: &[u8; 0x50], mapping: Mapping) -> HeaderScore {
    let header = &data[0x10..];
    let mapping_mode = header[0x15];
    let rom_type = header[0x16];
    let rom_size = header[0x17];
    let sram_size = header[0x18];
    let region = header[0x19];
    let dev_id = header[0x1A];
    let complement = u16::from_le_bytes([header[0x1C], header[0x1D]]);
    let checksum = u16::from_le_bytes([header[0x1E], header[0x1F]]);
    let gsu_ram_size = data[0x0D]; // $FFBD

    let mut score = 0;
    if checksum != 0 && complement != 0 && checksum.wrapping_add(complement) == 0xFFFF {
        score += 4;
    }
    if dev_id == 0x33 {
        score += 2;
    }
    if rom_type < 8 {
        score += 1;
    }
    if rom_size < 16 {
        score += 1;
    }
    if sram_size < 8 {
        score += 1;
    }
    if region < 14 {
        score += 1;
    }

    let mut chip = 0u8;
    let mut ram_size = sram_size;
    match (mapping_mode, rom_type) {
        (0x20, 0x03) => chip |= 0x84,                     // DSP1
        (0x21, 0x03) => chip |= 0x80,                     // DSP1B
        (0x30, 0x05) if dev_id != 0xB2 => chip |= 0x80,   // DSP1B
        (0x31, 0x03 | 0x05) => chip |= 0x80,              // DSP1B
        (0x20, 0x05) => chip |= 0x90,                     // DSP2
        (0x30, 0x05) => chip |= 0xA0,                     // DSP3
        (0x30, 0x03) => chip |= 0xB0,                     // DSP4
        (0x30, 0xF6) => {
            // ST010 / ST011
            chip |= 0x88;
            if rom_size < 10 {
                chip |= 0x20;
            }
            ram_size = 1;
        }
        (0x30, 0x25) => chip |= 0xC0, // OBC1
        _ => {}
    }
    if mapping_mode == 0x3A && (rom_type == 0xF5 || rom_type == 0xF9) {
        // SPC7110 (with RTC)
        chip |= 0xD0;
        if rom_type == 0xF9 {
            chip |= 0x08;
        }
    }
    if mapping_mode == 0x35 && rom_type == 0x55 {
        chip |= 0x08; // S-RTC
    }
    if mapping_mode == 0x20 && rom_type == 0xF3 {
        chip |= 0x40; // CX4
    }
    if mapping_mode == 0x32 && (rom_type == 0x43 || rom_type == 0x45) && rom_size < 14 {
        chip |= 0x50; // SDD1
    }
    if mapping_mode == 0x23 && matches!(rom_type, 0x32 | 0x34 | 0x35) {
        chip |= 0x60; // SA1
    }
    if mapping_mode == 0x20 && matches!(rom_type, 0x13 | 0x14 | 0x15 | 0x1A) {
        // GSU
        ram_size = match gsu_ram_size {
            0xFF => 5,
            n => n.min(6),
        };
        chip |= 0x70;
    }

    let pal = (0x02..=0x0C).contains(&region) || region == 0x11;

    match mapping {
        Mapping::LoRom if mapping_mode == 0x20 || mapping_mode == 0x22 => score += 2,
        Mapping::HiRom if mapping_mode == 0x21 => score += 2,
        Mapping::ExHiRom => {
            if mapping_mode == 0x25 || mapping_mode == 0x35 {
                score += 2;
            }
            if score != 0 {
                score += 4;
            }
        }
        _ => {}
    }

    HeaderScore {
        score,
        chip_type: chip,
        ram_size,
        pal,
    }
}

/// Read a header candidate at ROM offset `address` ($FFB0 in that mapping).
fn read_header(file: &mut File, rom_start: u64, rom_len: u64, address: u64) -> Option<[u8; 0x50]> {
    if address + 0x50 > rom_len {
        return None;
    }
    let mut data = [0u8; 0x50];
    file.seek(SeekFrom::Start(rom_start + address)).ok()?;
    file.read_exact(&mut data).ok()?;
    Some(data)
}

/// Work out the cartridge mapping of a ROM file, and leave the file positioned
/// at the start of the ROM data (after any copier header).
fn detect_cartridge(file: &mut File) -> std::io::Result<CartridgeConfig> {
    let file_len = file.metadata()?.len();
    let rom_start = if file_len % 1024 == COPIER_HEADER_SIZE {
        COPIER_HEADER_SIZE
    } else {
        0
    };
    let rom_len = file_len - rom_start;

    let mut rom_size = 0u8;
    while (1024u64 << rom_size) < rom_len && rom_size < 15 {
        rom_size += 1;
    }

    let mut score = |address: u64, mapping: Mapping| {
        read_header(file, rom_start, rom_len, address)
            .map(|data| score_header(&data, mapping))
            .unwrap_or_default()
    };
    let lorom = score(0x7FB0, Mapping::LoRom);
    let hirom = score(0xFFB0, Mapping::HiRom);
    let exhirom = score(0x40FFB0, Mapping::ExHiRom);

    let (mapping, chosen) = if lorom.score >= hirom.score && lorom.score >= exhirom.score {
        (0, lorom)
    } else if hirom.score >= exhirom.score {
        (1, hirom)
    } else if exhirom.score > 0 {
        (2, exhirom)
    } else {
        (0, lorom)
    };

    file.seek(SeekFrom::Start(rom_start))?;
    Ok(CartridgeConfig {
        rom_type: mapping | chosen.chip_type,
        rom_size,
        ram_size: chosen.ram_size.min(MAX_RAM_SIZE),
        pal: chosen.pal,
    })
}

pub struct Snes {
    cartridge: Option<CartridgeConfig>,
}

impl Snes {
    pub fn new() -> Self {
        Snes { cartridge: None }
    }

    pub fn get_core_info() -> CoreInfo {
        CoreInfo {
            id: "Game-Bub.SNES".try_into().unwrap(),
            name: "Super Nintendo".try_into().unwrap(),
            author: "Game Bub".try_into().unwrap(),
            files: [
                CoreFile {
                    id: FILE_ROM,
                    label: "ROM".try_into().unwrap(),
                    extensions: [".sfc".try_into().unwrap(), ".smc".try_into().unwrap()]
                        .into_iter()
                        .collect(),
                    filename: None,

                    optional: false,
                    read_only: true,
                    user_selected: true,
                    dependent_on_0: false,
                    initialize: false,

                    address: 0x3000_0000, // SDRAM
                    max_size: 16 * 1024 * 1024 + COPIER_HEADER_SIZE as u32,
                    exact_size: 0,
                    // The core writes SDRAM at about 7 MB/s.
                    max_transfer_speed: 5_000, // 5 MB/s
                    transfer_word_size: fpga::FpgaSpiWordSize::Bits32,
                },
                CoreFile {
                    id: FILE_SAVE,
                    label: "Save".try_into().unwrap(),
                    extensions: [".srm".try_into().unwrap()].into_iter().collect(),
                    filename: None,

                    optional: true,
                    read_only: false,
                    user_selected: false,
                    dependent_on_0: true,
                    initialize: true,

                    address: 0x4000_0000, // SRAM
                    max_size: 1024 << MAX_RAM_SIZE,
                    exact_size: 0,
                    max_transfer_speed: 10_000, // 10 MB/s
                    transfer_word_size: fpga::FpgaSpiWordSize::Bits32,
                },
            ]
            .into_iter()
            .collect(),
            bitstream: crate::util::get_system_file_path("snes.bit.hs"),
        }
    }
}

impl Bitstream for Snes {
    fn on_vblank_irq(&mut self) {}
}

impl CoreHandler for Snes {
    fn as_legacy_bitstream(&mut self) -> &mut dyn Bitstream {
        self
    }

    fn get_bitstream_override(&mut self, files: &[(u16, &Path)]) -> Result<Option<PathBuf>, String> {
        let Some((_, rom)) = files.iter().find(|(id, _)| *id == FILE_ROM) else {
            return Ok(None);
        };
        let mut file = File::open(rom).map_err(|e| e.to_string())?;
        let cartridge = detect_cartridge(&mut file).map_err(|e| e.to_string())?;
        match cartridge.bitstream() {
            Ok(name) => Ok(Some(crate::util::get_system_file_path(name))),
            Err(chip) => Err(format!(
                "This game needs the {chip} chip,\nwhich isn't supported yet."
            )),
        }
    }

    fn on_after_program(&mut self) {
        Device::lock().fpga.set_system_clock_rate(SYSTEM_CLOCK_RATE);
    }

    fn get_file_path_override(&mut self, _id: u16) -> Option<PathBuf> {
        None
    }

    fn on_before_file_load(&mut self, id: u16, file: &mut File) -> Result<(), String> {
        if id == FILE_ROM {
            let cartridge = detect_cartridge(file).map_err(|_| "I/O")?;
            log::info!("SNES cartridge: {:?}", cartridge);
            self.cartridge = Some(cartridge);
        }
        Ok(())
    }

    fn on_before_run(&mut self) -> Result<(), String> {
        let mut device = Device::lock();
        let cartridge = self.cartridge.unwrap_or_default();
        let registers = [
            (REG_ROM_TYPE, cartridge.rom_type as u32),
            (REG_ROM_MASK, cartridge.rom_mask()),
            (REG_RAM_MASK, cartridge.ram_mask()),
            (REG_RAM_SIZE, cartridge.ram_size as u32),
            (REG_PAL, cartridge.pal as u32),
        ];
        for (register, value) in registers {
            device
                .fpga
                .write_u32(register, value)
                .map_err(|e| e.to_string())?;
        }
        device
            .fpga
            .disable_interrupt(fpga::Irq::ModuleVblank)
            .map_err(|e| e.to_string())?;

        color_correction::presets::IDENTITY
            .with_display_color()
            .configure(&mut device, COLOR_CORRECTION_BASE)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn on_focus_changed(&mut self, _has_focus: bool) {}

    fn get_file_size(&mut self, id: u16) -> Option<u32> {
        assert!(id == FILE_SAVE);
        Some(self.cartridge.map_or(0, |c| c.ram_bytes()))
    }

    fn on_after_file_save(&mut self, _id: u16, _file: &mut File) -> Result<(), String> {
        Ok(())
    }
}
