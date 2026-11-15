use arrayvec::{ArrayString, ArrayVec};
use std::path::PathBuf;

use crate::device::drivers::fpga;

#[allow(unused)]
pub struct CoreInfo {
    pub id: ArrayString<32>,
    pub name: ArrayString<32>,
    pub author: ArrayString<32>,
    pub files: ArrayVec<CoreFile, 8>,
    pub bitstream: PathBuf,
}

#[allow(unused)]
pub struct CoreFile {
    pub id: u16,
    pub label: ArrayString<16>,

    /// List of file extensions (optional).
    pub extensions: ArrayVec<ArrayString<8>, 4>,

    /// If true, the core will still run if the file is not loaded.
    pub optional: bool,
    /// If true, file is treated as read-only, won't be saved at core end.
    pub read_only: bool,
    /// If true, the file path is selected by the user (filtered by extensions).
    pub user_selected: bool,
    /// If true, dependent on the file with ID 0 (and the path is determined based on that path + this extension).
    pub dependent_on_0: bool,
    /// If true, if the file is not loaded, the region will still be initialized with 0xFFs.
    pub initialize: bool,

    /// The address to load the file to.
    pub address: u32,
    /// Maximum size of the file.
    pub max_size: u32,
    /// Exact size of the file.
    pub exact_size: u32,
    /// Maximum read/write speed when loading/saving the file (in KB/s)
    pub max_transfer_speed: u32,

    /// Word size during transfer
    /// TODO: remove this, make all transfers 32-bit
    pub transfer_word_size: fpga::FpgaSpiWordSize,
}

impl CoreInfo {
    pub fn get_settings_path(&self) -> PathBuf {
        let mut p = PathBuf::from(super::DIR_SETTINGS);
        p.push(self.id);
        p.add_extension("json");
        p
    }
}

/// Get full information for a built-in core.
pub fn get_core(id: &str) -> Result<CoreInfo, String> {
    match id {
        "Game-Bub.GB" => Ok(crate::bitstream::gameboy::Gameboy::get_core_info()),
        "Game-Bub.GBA" => Ok(crate::bitstream::gba::Gba::get_core_info()),
        "Game-Bub.SNES" => Ok(crate::bitstream::snes::Snes::get_core_info()),
        "Game-Bub.NES" => Ok(crate::bitstream::nes::Nes::get_core_info()),
        "Game-Bub.Genesis" => Ok(crate::bitstream::genesis::Genesis::get_core_info()),
        _ => Err(format!("Unknown core {id}")),
    }
}
