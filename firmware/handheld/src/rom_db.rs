//! Identify ROM files by CRC32 and rename them to their No-Intro names, which is what
//! the box art library uses (see scripts/gen_rom_db.py).

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::library::System;

const HEADER_SIZE: usize = 16;
const RECORD_SIZE: usize = 16;
/// Characters FAT doesn't allow in file names.
const INVALID_FILE_NAME_CHARS: &str = "\"*/:<>?\\|";

/// A system's ROM database, and how its ROM files differ from No-Intro dumps.
pub struct RomDatabase {
    data: &'static [u8],
    /// Size of the file header (not part of the No-Intro dump) of a file with this length
    /// and these first bytes.
    header_size: fn(len: u64, start: &[u8]) -> u64,
    /// Save file extension.
    save_extension: &'static str,
    /// System name for messages.
    pub name: &'static str,
}

/// Super Nintendo: some files have a 512-byte copier (SMC) header.
pub static SNES: RomDatabase = RomDatabase {
    data: include_bytes!("../res/snesdb.bin"),
    header_size: |len, _| if len % 1024 == 512 { 512 } else { 0 },
    save_extension: "srm",
    name: "SNES",
};

/// NES: files have a 16-byte iNES header.
pub static NES: RomDatabase = RomDatabase {
    data: include_bytes!("../res/nesdb.bin"),
    header_size: |_, start| if start.starts_with(b"NES\x1a") { 16 } else { 0 },
    save_extension: "sav",
    name: "NES",
};

/// Master System: plain ROM files.
pub static SMS: RomDatabase = RomDatabase {
    data: include_bytes!("../res/smsdb.bin"),
    header_size: |_, _| 0,
    save_extension: "sav",
    name: "Master System",
};

/// Game Gear: plain ROM files.
pub static GG: RomDatabase = RomDatabase {
    data: include_bytes!("../res/ggdb.bin"),
    header_size: |_, _| 0,
    save_extension: "sav",
    name: "Game Gear",
};

/// Genesis / Mega Drive: plain ROM files (interleaved .smd dumps aren't recognized).
pub static GENESIS: RomDatabase = RomDatabase {
    data: include_bytes!("../res/genesisdb.bin"),
    header_size: |_, _| 0,
    save_extension: "sav",
    name: "Genesis",
};

impl RomDatabase {
    fn u32_at(&self, offset: usize) -> u32 {
        u32::from_le_bytes(self.data[offset..offset + 4].try_into().unwrap())
    }

    fn ok(&self) -> bool {
        self.data.len() >= HEADER_SIZE && &self.data[0..4] == b"SNDB"
    }

    fn record_count(&self) -> usize {
        self.u32_at(8) as usize
    }

    fn record_name(&self, index: usize) -> &'static str {
        let data = self.data;
        let record = HEADER_SIZE + index * RECORD_SIZE;
        let offset = self.u32_at(12) as usize + self.u32_at(record + 8) as usize;
        let length = u16::from_le_bytes([data[record + 12], data[record + 13]]) as usize;
        std::str::from_utf8(&data[offset..offset + length]).unwrap_or("")
    }

    /// No-Intro name of the ROM with this CRC32 (without any file header).
    pub fn lookup(&self, crc32: u32) -> Option<&'static str> {
        if !self.ok() {
            return None;
        }
        let (mut lo, mut hi) = (0, self.record_count());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.u32_at(HEADER_SIZE + mid * RECORD_SIZE).cmp(&crc32) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(self.record_name(mid)),
            }
        }
        None
    }

    /// Whether a file name (without extension) is already a No-Intro name.
    fn is_known_name(&self, name: &str) -> bool {
        self.ok() && (0..self.record_count()).any(|i| file_name_for(self.record_name(i)) == name)
    }

    /// CRC32 of a ROM file, without its file header if it has one.
    fn rom_crc32(&self, path: &Path, buf: &mut [u8]) -> std::io::Result<u32> {
        let mut file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        let mut start = [0u8; 16];
        let n = file.read(&mut start)?;
        file.seek(SeekFrom::Start((self.header_size)(len, &start[..n])))?;
        let mut crc = 0u32;
        loop {
            let n = file.read(buf)?;
            if n == 0 {
                return Ok(crc);
            }
            crc = unsafe { esp_idf_svc::sys::esp_rom_crc32_le(crc, buf.as_ptr(), n as u32) };
        }
    }
}

/// A No-Intro name as a FAT file name.
fn file_name_for(name: &str) -> String {
    name.chars()
        .map(|c| if INVALID_FILE_NAME_CHARS.contains(c) { '_' } else { c })
        .collect()
}

/// Rename `from` to `to` if it exists and `to` doesn't.
fn rename_if_present(from: &Path, to: &Path) {
    if from.exists() && !to.exists() {
        if let Err(e) = std::fs::rename(from, to) {
            log::warn!("Couldn't rename {}: {e}", from.display());
        }
    }
}

/// Result of identifying the ROMs in a system's folder.
#[derive(Default)]
pub struct IdentifyResult {
    pub renamed: usize,
    pub unknown: usize,
}

/// Identify the ROMs in `system`'s folder whose names aren't No-Intro names, and rename
/// them (with their save files and box art) to the No-Intro name.
pub fn identify_library(
    system: &System,
    db: &RomDatabase,
    buf: &mut [u8],
    progress: &mut dyn FnMut(&str, f32),
) -> IdentifyResult {
    let mut result = IdentifyResult::default();
    let games: Vec<_> = system
        .games()
        .into_iter()
        .filter(|game| !db.is_known_name(&game.name))
        .collect();
    for (i, game) in games.iter().enumerate() {
        progress(
            &format!("Identifying ROM {} of {}\n{}", i + 1, games.len(), game.name),
            i as f32 / games.len() as f32,
        );
        let crc = match db.rom_crc32(&game.path, buf) {
            Ok(crc) => crc,
            Err(e) => {
                log::warn!("Couldn't read {}: {e}", game.path.display());
                result.unknown += 1;
                continue;
            }
        };
        let Some(name) = db.lookup(crc) else {
            log::info!("{}: CRC32 {crc:08x} not in the database", game.name);
            result.unknown += 1;
            continue;
        };
        let new_name = file_name_for(name);
        let extension = game.path.extension().unwrap_or_default();
        let new_path: PathBuf = game.path.with_file_name(&new_name).with_extension(extension);
        if new_path.exists() {
            log::info!("{}: {} already exists", game.name, new_path.display());
            continue;
        }
        if let Err(e) = std::fs::rename(&game.path, &new_path) {
            log::warn!("Couldn't rename {}: {e}", game.path.display());
            continue;
        }
        log::info!("Renamed {} to {}", game.path.display(), new_path.display());
        result.renamed += 1;

        // Keep the game's save files, box art and favorites with it.
        let save = db.save_extension;
        rename_if_present(&game.path.with_extension(save), &new_path.with_extension(save));
        rename_if_present(
            &crate::core::saves_dir_path(&game.path, save),
            &crate::core::saves_dir_path(&new_path, save),
        );
        rename_if_present(&system.thumbnail_path(&game.name), &system.thumbnail_path(&new_name));
        crate::core::rename_rom_in_settings(system.core_id, &game.path, &new_path);
    }
    result
}
