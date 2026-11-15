//! Driver for the Genesis / Mega Drive FPGA module (a port of the MiSTer Genesis core).
//!
//! The ROM file (plain binary, as No-Intro `.md` files) is written to the core's SDRAM, then
//! the battery save (if any) into its save RAM. The region and the save size come from the
//! cartridge header.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

use esp_idf_svc::hal::units::Hertz;

use crate::{
    core::{CoreFile, CoreHandler, CoreInfo},
    device::{drivers::fpga, Device},
};

use super::{util::color_correction, Bitstream};

/// Genesis master clock (53.693 MHz NTSC; the FPGA's clock runs 0.03% fast).
const SYSTEM_CLOCK_RATE: Hertz = Hertz(53_710_937);

const REG_REGION: u32 = 0x0000_0000;
const REG_DOWNLOAD: u32 = 0x0000_0004;
const REG_SIX_BUTTON: u32 = 0x0000_0008;
const REG_ROM_SIZE: u32 = 0x0000_000C;
const COLOR_CORRECTION_BASE: u32 = 0x5000_0000;

/// REG_REGION values (the console's region switches)
const REGION_JAPAN: u32 = 0;
const REGION_USA: u32 = 1;
const REGION_EUROPE: u32 = 2;

const FILE_ROM: u16 = 0;
const FILE_SAVE: u16 = 1;

/// Largest ROM the core maps (the 68000's cartridge space).
const MAX_ROM_SIZE: u32 = 10 * 1024 * 1024;
/// The core's save RAM: one byte for each 68000 word of the cartridge's save area.
const MAX_SAVE_SIZE: u32 = 64 * 1024;

/// What the firmware needs from the cartridge header (at 0x100).
#[derive(Clone, Copy, Debug, Default)]
struct Cartridge {
    /// REG_REGION value
    region: u32,
    /// Save file size in bytes (0 if none)
    save_size: u32,
}

/// Read the region and save RAM size from the header, as the MiSTer core's top level does.
fn parse_header(header: &[u8; 0x200]) -> Cartridge {
    // Region codes at 0x1F0-0x1F2: letters (J, U, E), or a hex digit whose bits are
    // Japan (0), USA (2) and Europe (3).
    let (mut japan, mut usa, mut europe) = (false, false, false);
    for (i, &c) in header[0x1F0..0x1F3].iter().enumerate() {
        match c {
            b'J' => japan = true,
            b'U' => usa = true,
            b'E' => europe = true,
            b'0'..=b'9' | b'A'..=b'F' if i == 0 => {
                let bits = (c as char).to_digit(16).unwrap();
                japan |= bits & 1 != 0;
                usa |= bits & 4 != 0;
                europe |= bits & 8 != 0;
            }
            _ => {}
        }
    }
    let region = if usa {
        REGION_USA
    } else if europe {
        REGION_EUROPE
    } else if japan {
        REGION_JAPAN
    } else {
        REGION_USA
    };

    // Save RAM: "RA" at 0x1B0, then its start and end addresses. The core keeps one byte per
    // 68000 word of the range.
    let be32 = |offset: usize| u32::from_be_bytes(header[offset..offset + 4].try_into().unwrap());
    let save_size = if &header[0x1B0..0x1B2] == b"RA" {
        let (start, end) = (be32(0x1B4), be32(0x1B8));
        if end > start { ((end - start) / 2 + 1).min(MAX_SAVE_SIZE) } else { 0 }
    } else {
        0
    };
    Cartridge { region, save_size }
}

pub struct Genesis {
    cartridge: Option<Cartridge>,
}

impl Genesis {
    pub fn new() -> Self {
        Genesis { cartridge: None }
    }

    pub fn get_core_info() -> CoreInfo {
        CoreInfo {
            id: "Game-Bub.Genesis".try_into().unwrap(),
            name: "Genesis".try_into().unwrap(),
            author: "Game Bub".try_into().unwrap(),
            files: [
                CoreFile {
                    id: FILE_ROM,
                    label: "ROM".try_into().unwrap(),
                    extensions: [
                        ".md".try_into().unwrap(),
                        ".gen".try_into().unwrap(),
                        ".bin".try_into().unwrap(),
                    ]
                    .into_iter()
                    .collect(),

                    optional: false,
                    read_only: true,
                    user_selected: true,
                    dependent_on_0: false,
                    initialize: false,

                    address: 0x3000_0000, // Loader (file in order, 32 bits at a time)
                    max_size: MAX_ROM_SIZE,
                    exact_size: 0,
                    max_transfer_speed: 5_000, // 5 MB/s (the slowest SPI clock)
                    transfer_word_size: fpga::FpgaSpiWordSize::Bits32,
                },
                CoreFile {
                    id: FILE_SAVE,
                    label: "Save".try_into().unwrap(),
                    extensions: [".sav".try_into().unwrap()].into_iter().collect(),

                    optional: true,
                    read_only: false,
                    user_selected: false,
                    dependent_on_0: true,
                    // The core fills the save RAM with 0xFF when it loads a game.
                    initialize: false,

                    address: 0x4000_0000, // Save RAM
                    max_size: MAX_SAVE_SIZE,
                    exact_size: 0,
                    max_transfer_speed: 5_000, // 5 MB/s
                    transfer_word_size: fpga::FpgaSpiWordSize::Bits32,
                },
            ]
            .into_iter()
            .collect(),
            bitstream: crate::util::get_system_file_path("genesis.bit.hs"),
        }
    }
}

impl Bitstream for Genesis {
    fn on_vblank_irq(&mut self) {}
}

impl CoreHandler for Genesis {
    fn as_legacy_bitstream(&mut self) -> &mut dyn Bitstream {
        self
    }

    fn on_after_program(&mut self) {
        Device::lock().fpga.set_system_clock_rate(SYSTEM_CLOCK_RATE);
    }

    fn get_file_path_override(&mut self, _id: u16) -> Option<std::path::PathBuf> {
        None
    }

    fn on_before_file_load(&mut self, id: u16, file: &mut File) -> Result<(), String> {
        if id == FILE_ROM {
            let size = file.metadata().map_err(|_| "I/O")?.len() as u32;
            let mut header = [0u8; 0x200];
            file.read_exact(&mut header).map_err(|_| "This ROM file is too small.")?;
            file.seek(SeekFrom::Start(0)).map_err(|_| "I/O")?;
            if &header[0x100..0x104] != b"SEGA" && &header[0x101..0x105] != b"SEGA" {
                // Interleaved (.smd) dumps and unusual cartridges don't have it at 0x100.
                log::warn!("No SEGA signature at 0x100");
            }
            let cartridge = parse_header(&header);
            log::info!("Genesis cartridge: {:?}, {size} bytes", cartridge);
            self.cartridge = Some(cartridge);

            let mut device = Device::lock();
            for (register, value) in [
                (REG_REGION, cartridge.region),
                (REG_SIX_BUTTON, 0),
                (REG_ROM_SIZE, size),
                (REG_DOWNLOAD, 1),
            ] {
                device.fpga.write_u32(register, value).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    fn on_after_file_load(&mut self, id: u16) {
        if id == FILE_ROM {
            if let Err(e) = Device::lock().fpga.write_u32(REG_DOWNLOAD, 0) {
                log::warn!("Couldn't end the download: {e}");
            }
        }
    }

    fn on_before_run(&mut self) -> Result<(), String> {
        let mut device = Device::lock();
        device
            .fpga
            .disable_interrupt(fpga::Irq::ModuleVblank)
            .map_err(|e| e.to_string())?;
        color_correction::presets::GENESIS
            .with_display_color()
            .configure(&mut device, COLOR_CORRECTION_BASE)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn on_focus_changed(&mut self, _has_focus: bool) {}

    fn get_file_size(&mut self, id: u16) -> Option<u32> {
        assert!(id == FILE_SAVE);
        Some(self.cartridge.map_or(0, |c| c.save_size))
    }

    fn on_after_file_save(&mut self, _id: u16, _file: &mut File) -> Result<(), String> {
        Ok(())
    }
}
