//! Identify SNES ROM files by CRC32 and rename them to their No-Intro names, which is what
//! the box art library uses (see scripts/gen_snes_db.py).

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::library::System;

static DATABASE: &[u8] = include_bytes!("../res/snesdb.bin");

const HEADER_SIZE: usize = 16;
const RECORD_SIZE: usize = 16;
/// Size of a copier (SMC) header, which isn't part of the No-Intro dump.
const COPIER_HEADER_SIZE: u64 = 512;
/// Characters FAT doesn't allow in file names.
const INVALID_FILE_NAME_CHARS: &str = "\"*/:<>?\\|";

fn u32_at(offset: usize) -> u32 {
    u32::from_le_bytes(DATABASE[offset..offset + 4].try_into().unwrap())
}

fn database_ok() -> bool {
    DATABASE.len() >= HEADER_SIZE && &DATABASE[0..4] == b"SNDB"
}

fn record_count() -> usize {
    u32_at(8) as usize
}

fn record_name(index: usize) -> &'static str {
    let record = HEADER_SIZE + index * RECORD_SIZE;
    let offset = u32_at(12) as usize + u32_at(record + 8) as usize;
    let length = u16::from_le_bytes([DATABASE[record + 12], DATABASE[record + 13]]) as usize;
    std::str::from_utf8(&DATABASE[offset..offset + length]).unwrap_or("")
}

/// No-Intro name of the ROM with this CRC32 (without a copier header).
pub fn lookup(crc32: u32) -> Option<&'static str> {
    if !database_ok() {
        return None;
    }
    let (mut lo, mut hi) = (0, record_count());
    while lo < hi {
        let mid = (lo + hi) / 2;
        match u32_at(HEADER_SIZE + mid * RECORD_SIZE).cmp(&crc32) {
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
            std::cmp::Ordering::Equal => return Some(record_name(mid)),
        }
    }
    None
}

/// Whether a file name (without extension) is already a No-Intro name.
fn is_known_name(name: &str) -> bool {
    database_ok() && (0..record_count()).any(|i| file_name_for(record_name(i)) == name)
}

/// A No-Intro name as a FAT file name.
fn file_name_for(name: &str) -> String {
    name.chars()
        .map(|c| if INVALID_FILE_NAME_CHARS.contains(c) { '_' } else { c })
        .collect()
}

/// CRC32 of a ROM file, without its copier header if it has one.
fn rom_crc32(path: &Path, buf: &mut [u8]) -> std::io::Result<u32> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len % 1024 == COPIER_HEADER_SIZE {
        file.seek(SeekFrom::Start(COPIER_HEADER_SIZE))?;
    }
    let mut crc = 0u32;
    loop {
        let n = file.read(buf)?;
        if n == 0 {
            return Ok(crc);
        }
        crc = unsafe { esp_idf_svc::sys::esp_rom_crc32_le(crc, buf.as_ptr(), n as u32) };
    }
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
    buf: &mut [u8],
    progress: &mut dyn FnMut(&str, f32),
) -> IdentifyResult {
    let mut result = IdentifyResult::default();
    let games: Vec<_> = system
        .games()
        .into_iter()
        .filter(|game| !is_known_name(&game.name))
        .collect();
    for (i, game) in games.iter().enumerate() {
        progress(
            &format!("Identifying ROM {} of {}\n{}", i + 1, games.len(), game.name),
            i as f32 / games.len() as f32,
        );
        let crc = match rom_crc32(&game.path, buf) {
            Ok(crc) => crc,
            Err(e) => {
                log::warn!("Couldn't read {}: {e}", game.path.display());
                result.unknown += 1;
                continue;
            }
        };
        let Some(name) = lookup(crc) else {
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
        rename_if_present(&game.path.with_extension("srm"), &new_path.with_extension("srm"));
        rename_if_present(
            &crate::core::saves_dir_path(&game.path, "srm"),
            &crate::core::saves_dir_path(&new_path, "srm"),
        );
        rename_if_present(&system.thumbnail_path(&game.name), &system.thumbnail_path(&new_name));
        crate::core::rename_rom_in_settings(system.core_id, &game.path, &new_path);
    }
    result
}
