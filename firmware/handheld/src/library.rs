//! ROM library for "Run ROM": ROM files in /roms/<system>/, and their box art thumbnails in
//! /boxart/<system>/.

use std::io::Read;
use std::path::{Path, PathBuf};

use slint::{Rgb8Pixel, SharedPixelBuffer};

use crate::core::browse::{has_extension, natural_cmp};

pub const DIR_ROMS: &str = "/sdcard/roms";
pub const DIR_BOXART: &str = "/sdcard/boxart";

/// Most games listed per system (keeps memory use bounded).
const MAX_GAMES: usize = 2000;

/// Largest thumbnail accepted (pixels per side).
const MAX_THUMBNAIL: u16 = 192;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct System {
    /// Folder name under /roms and /boxart
    pub folder: &'static str,
    /// ROM file extensions (with the dot)
    pub extensions: &'static [&'static str],
    /// Core that runs these ROMs
    pub core_id: &'static str,
}

pub const SYSTEMS: [System; 3] = [
    System { folder: "Game Boy", extensions: &[".gb", ".sgb"], core_id: "Game-Bub.GB" },
    System { folder: "Game Boy Color", extensions: &[".gbc", ".gb"], core_id: "Game-Bub.GB" },
    System { folder: "Game Boy Advance", extensions: &[".gba"], core_id: "Game-Bub.GBA" },
];

#[derive(Clone, Debug)]
pub struct Game {
    /// Name shown in the list (the file name without its extension)
    pub name: String,
    pub path: PathBuf,
}

impl System {
    pub fn rom_dir(&self) -> PathBuf {
        Path::new(DIR_ROMS).join(self.folder)
    }

    /// ROMs in this system's folder, sorted by name.
    pub fn games(&self) -> Vec<Game> {
        let mut games: Vec<Game> = std::fs::read_dir(self.rom_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let file_name = entry.file_name().to_string_lossy().into_owned();
                // Skip macOS metadata files ("._Game.gba").
                if file_name.starts_with("._") || !has_extension(&file_name, self.extensions) {
                    return None;
                }
                let name = Path::new(&file_name).file_stem()?.to_string_lossy().into_owned();
                Some(Game { name, path: entry.path() })
            })
            .take(MAX_GAMES)
            .collect();
        games.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        games
    }

    /// Path of a game's box art thumbnail.
    pub fn thumbnail_path(&self, game_name: &str) -> PathBuf {
        Path::new(DIR_BOXART).join(self.folder).join(format!("{game_name}.thumb"))
    }
}

/// Free memory to leave after loading a thumbnail.
const THUMBNAIL_HEADROOM: usize = 24 * 1024;

/// Rows per strip: the heap is fragmented into blocks of ~30 KiB, so a thumbnail is loaded as
/// several small images (about 8 KiB each at 120 pixels wide) shown stacked.
const STRIP_ROWS: usize = 24;

/// A loaded thumbnail: its size, and its image strips (top to bottom).
pub struct LoadedThumbnail {
    pub width: u32,
    pub height: u32,
    /// Loaded at reduced resolution (low memory)
    pub reduced: bool,
    pub strips: Vec<SharedPixelBuffer<Rgb8Pixel>>,
}

/// Load a thumbnail file: b"BBX1", u16 width, u16 height (little endian), then RGB888 rows.
///
/// If there isn't enough free memory for the full size, the image is shrunk while it's read
/// (every 2nd or 3rd pixel). Returns None if the file is missing or invalid.
pub fn load_thumbnail(path: &Path) -> Option<Result<LoadedThumbnail, LowMemory>> {
    let mut file = std::io::BufReader::with_capacity(2048, std::fs::File::open(path).ok()?);
    let mut header = [0u8; 8];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"BBX1" {
        return None;
    }
    let width = u16::from_le_bytes([header[4], header[5]]) as usize;
    let height = u16::from_le_bytes([header[6], header[7]]) as usize;
    if width == 0 || height == 0 || width > MAX_THUMBNAIL as usize || height > MAX_THUMBNAIL as usize {
        return None;
    }
    let (free, largest_free) = unsafe {
        use esp_idf_svc::sys::{heap_caps_get_free_size, heap_caps_get_largest_free_block, MALLOC_CAP_8BIT};
        (heap_caps_get_free_size(MALLOC_CAP_8BIT), heap_caps_get_largest_free_block(MALLOC_CAP_8BIT))
    };
    let fits = |step: usize| {
        let (w, h) = (width / step, height / step);
        w * h * 3 + THUMBNAIL_HEADROOM <= free && w * STRIP_ROWS.min(h) * 3 + 4096 <= largest_free
    };
    let Some(step) = (1..=3).find(|&step| fits(step)) else {
        log::warn!("Not enough memory for box art (free {free}, largest block {largest_free})");
        return Some(Err(LowMemory { largest_free: free }));
    };
    let (out_width, out_height) = (width / step, height / step);
    let mut strips = vec![];
    let mut row = vec![0u8; width * 3];
    let mut out_y = 0;
    while out_y < out_height {
        let rows = STRIP_ROWS.min(out_height - out_y);
        let mut strip = SharedPixelBuffer::<Rgb8Pixel>::new(out_width as u32, rows as u32);
        let out = strip.make_mut_bytes();
        for r in 0..rows {
            // Read `step` source rows, keep the first.
            for skip in 0..step {
                file.read_exact(&mut row).ok()?;
                if skip == 0 {
                    let out_row = &mut out[r * out_width * 3..][..out_width * 3];
                    for x in 0..out_width {
                        out_row[x * 3..x * 3 + 3].copy_from_slice(&row[x * step * 3..x * step * 3 + 3]);
                    }
                }
            }
        }
        strips.push(strip);
        out_y += rows;
    }
    Some(Ok(LoadedThumbnail { width: out_width as u32, height: out_height as u32, reduced: step > 1, strips }))
}

/// A thumbnail wasn't loaded because there wasn't enough free memory.
pub struct LowMemory {
    /// Free memory, in bytes
    pub largest_free: usize,
}
