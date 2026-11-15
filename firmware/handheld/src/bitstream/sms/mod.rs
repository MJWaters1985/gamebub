//! Driver for the Master System and Game Gear FPGA modules (a port of the MiSTer SMS core,
//! built twice: 256x192 for the Master System, 160x144 for the Game Gear).
//!
//! The ROM file is written to the core's SDRAM, then the battery save (if any) into its
//! 32 KB save RAM. The core detects the cartridge's mapper from the ROM as it loads.

use std::fs::File;

use esp_idf_svc::hal::units::Hertz;

use crate::{
    core::{CoreFile, CoreHandler, CoreInfo},
    device::{drivers::fpga, Device},
};

use super::{util::color_correction, Bitstream};

/// Master clock (53.693 MHz, 15x the Z80's; the FPGA's clock runs 0.03% fast).
const SYSTEM_CLOCK_RATE: Hertz = Hertz(53_710_937);

const REG_PAL: u32 = 0x0000_0000;
const REG_DOWNLOAD: u32 = 0x0000_0004;
const REG_JAPAN: u32 = 0x0000_0008;
/// Read: 1 if the game has written its save RAM since it was loaded
const REG_SAVE_USED: u32 = 0x0000_000C;
const COLOR_CORRECTION_BASE: u32 = 0x5000_0000;

const FILE_ROM: u16 = 0;
const FILE_SAVE: u16 = 1;

const MAX_ROM_SIZE: u32 = 4 * 1024 * 1024;
/// The core's battery RAM (cartridges have 8 KB or 32 KB; the whole RAM is saved).
const SAVE_SIZE: u32 = 32 * 1024;

pub struct Sms {
    game_gear: bool,
}

impl Sms {
    pub fn new(game_gear: bool) -> Self {
        Sms { game_gear }
    }

    pub fn get_core_info(game_gear: bool) -> CoreInfo {
        let (id, name, extension, bitstream) = if game_gear {
            ("Game-Bub.GG", "Game Gear", ".gg", "gg.bit.hs")
        } else {
            ("Game-Bub.SMS", "Master System", ".sms", "sms.bit.hs")
        };
        CoreInfo {
            id: id.try_into().unwrap(),
            name: name.try_into().unwrap(),
            author: "Game Bub".try_into().unwrap(),
            files: [
                CoreFile {
                    id: FILE_ROM,
                    label: "ROM".try_into().unwrap(),
                    extensions: [extension.try_into().unwrap()].into_iter().collect(),

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
                    max_size: SAVE_SIZE,
                    exact_size: 0,
                    max_transfer_speed: 5_000,
                    transfer_word_size: fpga::FpgaSpiWordSize::Bits32,
                },
            ]
            .into_iter()
            .collect(),
            bitstream: crate::util::get_system_file_path(bitstream),
        }
    }
}

impl Bitstream for Sms {
    fn on_vblank_irq(&mut self) {}
}

impl CoreHandler for Sms {
    fn as_legacy_bitstream(&mut self) -> &mut dyn Bitstream {
        self
    }

    fn on_after_program(&mut self) {
        Device::lock().fpga.set_system_clock_rate(SYSTEM_CLOCK_RATE);
    }

    fn get_file_path_override(&mut self, _id: u16) -> Option<std::path::PathBuf> {
        None
    }

    fn on_before_file_load(&mut self, id: u16, _file: &mut File) -> Result<(), String> {
        if id == FILE_ROM {
            log::info!("{} game", if self.game_gear { "Game Gear" } else { "Master System" });
            let mut device = Device::lock();
            for (register, value) in [(REG_PAL, 0), (REG_JAPAN, 0), (REG_DOWNLOAD, 1)] {
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
        color_correction::presets::LINEAR_4BIT
            .with_display_color()
            .configure(&mut device, COLOR_CORRECTION_BASE)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn on_focus_changed(&mut self, _has_focus: bool) {}

    fn get_file_size(&mut self, id: u16) -> Option<u32> {
        assert!(id == FILE_SAVE);
        // Cartridges don't say whether they have battery RAM: save only when the game has
        // written to it (an existing save is kept as it is otherwise).
        let used = Device::lock().fpga.read_u32(REG_SAVE_USED).map_or(true, |v| v & 1 != 0);
        Some(if used { SAVE_SIZE } else { 0 })
    }

    fn on_after_file_save(&mut self, _id: u16, _file: &mut File) -> Result<(), String> {
        Ok(())
    }
}
