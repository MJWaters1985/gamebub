//! Driver for the NES FPGA module (a port of the MiSTer NES core).
//!
//! The core reads the iNES header itself: the ROM file is streamed to it whole, then the
//! battery save (if any) is loaded into the cartridge RAM.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    time::{Duration, Instant},
};

use esp_idf_svc::hal::units::Hertz;

use crate::{
    core::{CoreFile, CoreHandler, CoreInfo},
    device::{drivers::fpga, Device},
};

use super::{util::color_correction, Bitstream};

/// NES master clock (21.477 MHz NTSC; the FPGA's clock runs 0.03% fast).
const SYSTEM_CLOCK_RATE: Hertz = Hertz(21_484_375);

const REG_DOWNLOAD_TYPE: u32 = 0x0000_0000;
const REG_DOWNLOAD: u32 = 0x0000_0004;
const REG_LOADER_STATUS: u32 = 0x0000_0008;
const REG_SYS_TYPE: u32 = 0x0000_000C;
const COLOR_CORRECTION_BASE: u32 = 0x5000_0000;

/// REG_LOADER_STATUS bits
const LOADER_BUSY: u32 = 1 << 0;
const LOADER_FAIL: u32 = 1 << 1;

/// REG_DOWNLOAD_TYPE values
const DOWNLOAD_NES: u32 = 0;

const FILE_ROM: u16 = 0;
const FILE_SAVE: u16 = 1;

/// Largest battery RAM the core keeps (and the save window's size).
const MAX_SAVE_SIZE: u32 = 128 * 1024;
/// How long the core may take to clear its RAM after loading a game.
const LOADER_TIMEOUT: Duration = Duration::from_secs(3);

/// What the firmware needs from the iNES header (the core reads the rest).
#[derive(Clone, Copy, Debug, Default)]
struct Cartridge {
    #[allow(dead_code)] // logged
    mapper: u16,
    /// Battery-backed save size in bytes (0 if none).
    save_size: u32,
    /// 0 = NTSC, 1 = PAL, 2 = Dendy (the core's sys_type).
    sys_type: u8,
}

fn parse_header(header: &[u8; 16]) -> Option<Cartridge> {
    if &header[0..4] != b"NES\x1a" {
        return None;
    }
    let nes2 = header[7] & 0x0C == 0x08;
    let mut mapper = (header[6] >> 4) as u16 | (header[7] & 0xF0) as u16;
    let battery = header[6] & 0x02 != 0;
    let (save_size, sys_type) = if nes2 {
        mapper |= ((header[8] & 0x0F) as u16) << 8;
        let shift = header[10] >> 4;
        let size = if shift == 0 { 0 } else { 64u32 << shift };
        let sys_type = match header[12] & 0x03 {
            1 => 1, // PAL
            3 => 2, // Dendy
            _ => 0, // NTSC, or multi-region
        };
        (size, sys_type)
    } else {
        (if battery { 8 * 1024 } else { 0 }, 0)
    };
    Some(Cartridge {
        mapper,
        save_size: if battery { save_size.min(MAX_SAVE_SIZE) } else { 0 },
        sys_type,
    })
}

pub struct Nes {
    cartridge: Option<Cartridge>,
}

impl Nes {
    pub fn new() -> Self {
        Nes { cartridge: None }
    }

    pub fn get_core_info() -> CoreInfo {
        CoreInfo {
            id: "Game-Bub.NES".try_into().unwrap(),
            name: "NES".try_into().unwrap(),
            author: "Game Bub".try_into().unwrap(),
            files: [
                CoreFile {
                    id: FILE_ROM,
                    label: "ROM".try_into().unwrap(),
                    extensions: [".nes".try_into().unwrap()].into_iter().collect(),

                    optional: false,
                    read_only: true,
                    user_selected: true,
                    dependent_on_0: false,
                    initialize: false,

                    address: 0x3000_0000, // Loader (byte stream)
                    max_size: 8 * 1024 * 1024,
                    exact_size: 0,
                    // The core takes a byte every 4 cycles (5.4 MB/s), so this must
                    // stay at the slowest SPI clock.
                    max_transfer_speed: 5_000, // 5 MB/s
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
                    // The core clears the save RAM when it loads a game.
                    initialize: false,

                    address: 0x4000_0000, // Cartridge RAM
                    max_size: MAX_SAVE_SIZE,
                    exact_size: 0,
                    max_transfer_speed: 5_000, // 5 MB/s (the slowest SPI clock)
                    transfer_word_size: fpga::FpgaSpiWordSize::Bits32,
                },
            ]
            .into_iter()
            .collect(),
            bitstream: crate::util::get_system_file_path("nes.bit.hs"),
        }
    }

    /// Wait for the core to finish loading the game (it clears RAM afterwards).
    fn wait_for_loader(&self) -> Result<(), String> {
        let start = Instant::now();
        loop {
            let status = Device::lock()
                .fpga
                .read_u32(REG_LOADER_STATUS)
                .map_err(|e| e.to_string())?;
            if status & LOADER_FAIL != 0 {
                return Err("This ROM file isn't supported.".to_string());
            }
            if status & LOADER_BUSY == 0 {
                return Ok(());
            }
            if start.elapsed() > LOADER_TIMEOUT {
                return Err("The game didn't finish loading.".to_string());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Bitstream for Nes {
    fn on_vblank_irq(&mut self) {}
}

impl CoreHandler for Nes {
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
            let mut header = [0u8; 16];
            file.read_exact(&mut header).map_err(|_| "I/O")?;
            file.seek(SeekFrom::Start(0)).map_err(|_| "I/O")?;
            let cartridge = parse_header(&header).ok_or("Not an iNES (.nes) file.")?;
            log::info!("NES cartridge: {:?}", cartridge);
            self.cartridge = Some(cartridge);

            let mut device = Device::lock();
            for (register, value) in [
                (REG_DOWNLOAD_TYPE, DOWNLOAD_NES),
                (REG_SYS_TYPE, cartridge.sys_type as u32),
                (REG_DOWNLOAD, 1),
            ] {
                device.fpga.write_u32(register, value).map_err(|e| e.to_string())?;
            }
        } else {
            // The save goes into RAM the loader clears, so wait for it.
            self.wait_for_loader()?;
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
        self.wait_for_loader()?;
        let mut device = Device::lock();
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
        Some(self.cartridge.map_or(0, |c| c.save_size))
    }

    fn on_after_file_save(&mut self, _id: u16, _file: &mut File) -> Result<(), String> {
        Ok(())
    }
}
