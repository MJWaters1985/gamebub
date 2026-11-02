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

    /// Names to show in the list: without the trailing tags of No-Intro style names, e.g.
    /// "Super Mario Land 2 - 6 Golden Coins" for "... (USA, Europe) (Rev 2)". Shorter names also
    /// keep the renderer's per-frame memory down. Games whose short names would be the same
    /// keep their full names.
    pub fn titles(games: &[Game]) -> Vec<String> {
        let short: Vec<&str> = games.iter().map(|g| short_title(&g.name)).collect();
        games
            .iter()
            .zip(&short)
            .map(|(game, title)| {
                if short.iter().filter(|t| *t == title).count() > 1 {
                    game.name.clone()
                } else {
                    title.to_string()
                }
            })
            .collect()
    }

    /// Path of a game's box art thumbnail.
    pub fn thumbnail_path(&self, game_name: &str) -> PathBuf {
        Path::new(DIR_BOXART).join(self.folder).join(format!("{game_name}.thumb"))
    }
}

/// Free memory to leave after loading a thumbnail.
const THUMBNAIL_HEADROOM: usize = 24 * 1024;

/// Largest free block to leave after loading a thumbnail, for the renderer.
const RENDER_HEADROOM: usize = 12 * 1024;

/// Rows per strip: the heap is fragmented, so a thumbnail is loaded as several small images
/// (about 3 KiB each at 120 pixels wide) shown stacked. Small strips fill small gaps and leave
/// the larger blocks the renderer needs.
const STRIP_ROWS: usize = 8;

/// A loaded thumbnail: its size, and its image strips (top to bottom).
pub struct LoadedThumbnail {
    pub width: u32,
    pub height: u32,
    /// Loaded at reduced resolution (low memory)
    pub reduced: bool,
    pub strips: Vec<SharedPixelBuffer<Rgb8Pixel>>,
}

/// Size of the box art buffers: the size Download Box Art makes thumbnails at. Larger
/// thumbnails are shrunk to fit.
const POOL_SIZE: usize = 120;

/// Pixel buffers for box art, allocated once for the largest thumbnail and reused for every
/// game until leaving Run ROM. Allocating and freeing them per game fragments the heap until
/// the renderer can't get the block it needs.
#[derive(Default)]
pub struct StripPool {
    /// Strips of STRIP_ROWS rows, POOL_SIZE / step pixels wide
    strips: Vec<SharedPixelBuffer<Rgb8Pixel>>,
    /// Every `step`th pixel is kept (1 = full size)
    step: usize,
}

impl StripPool {
    /// Free the buffers (when leaving Run ROM).
    pub fn clear(&mut self) {
        *self = StripPool::default();
    }

    /// Allocate buffers for the largest thumbnail size (no larger than 1/`min_step`) that leaves
    /// enough memory.
    fn allocate(&mut self, min_step: usize) -> bool {
        let free = unsafe { esp_idf_svc::sys::heap_caps_get_free_size(esp_idf_svc::sys::MALLOC_CAP_8BIT) };
        for step in min_step..=3 {
            let size = POOL_SIZE / step;
            if size * size * 3 + THUMBNAIL_HEADROOM > free {
                continue;
            }
            let strips: Vec<_> = (0..size.div_ceil(STRIP_ROWS))
                .map(|_| SharedPixelBuffer::<Rgb8Pixel>::new(size as u32, STRIP_ROWS as u32))
                .collect();
            // The renderer needs a block of a few KB per frame for its list of things to draw
            // (more with more text on screen): leave room for it, or use a smaller size.
            let largest_after = unsafe {
                esp_idf_svc::sys::heap_caps_get_largest_free_block(esp_idf_svc::sys::MALLOC_CAP_8BIT)
            };
            if largest_after >= RENDER_HEADROOM {
                *self = StripPool { strips, step };
                return true;
            }
            log::warn!("Box art at 1/{step} size leaves too little memory (largest block {largest_after})");
        }
        log::warn!("Not enough memory for box art (free {free})");
        false
    }
}

/// Load a thumbnail file: b"BBX1", u16 width, u16 height (little endian), then RGB888 rows.
///
/// The image is read into the strips of `pool` (allocated on first use), shrunk if the pool is
/// for a smaller size (every 2nd or 3rd pixel). The thumbnail's strips share the pool's
/// buffers, so release them before loading the next one; each strip is the pool's width, so
/// show only `width` columns and `height` rows in total. Returns None if the file is missing
/// or invalid.
pub fn load_thumbnail(
    path: &Path,
    pool: &mut StripPool,
    min_step: usize,
) -> Option<Result<LoadedThumbnail, LowMemory>> {
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
    if pool.strips.is_empty() && !pool.allocate(min_step) {
        let free = unsafe { esp_idf_svc::sys::heap_caps_get_free_size(esp_idf_svc::sys::MALLOC_CAP_8BIT) };
        return Some(Err(LowMemory { largest_free: free }));
    }
    // Shrink to fit the buffers.
    let capacity = POOL_SIZE / pool.step;
    let mut step = pool.step;
    while width / step > capacity || height / step > capacity {
        step += 1;
    }
    let (out_width, out_height) = (width / step, height / step);
    let used = out_height.div_ceil(STRIP_ROWS);
    let mut row = vec![0u8; width * 3];
    for (k, strip) in pool.strips[..used].iter_mut().enumerate() {
        let stride = strip.width() as usize * 3;
        let rows = STRIP_ROWS.min(out_height - k * STRIP_ROWS);
        let out = strip.make_mut_bytes();
        for r in 0..rows {
            // Read `step` source rows, keep the first.
            for skip in 0..step {
                file.read_exact(&mut row).ok()?;
                if skip == 0 {
                    let out_row = &mut out[r * stride..][..out_width * 3];
                    for x in 0..out_width {
                        out_row[x * 3..x * 3 + 3].copy_from_slice(&row[x * step * 3..x * step * 3 + 3]);
                    }
                }
            }
        }
    }
    Some(Ok(LoadedThumbnail {
        width: out_width as u32,
        height: out_height as u32,
        // Smaller than intended, for lack of memory
        reduced: step > min_step,
        strips: pool.strips[..used].to_vec(),
    }))
}

/// A thumbnail wasn't loaded because there wasn't enough free memory.
pub struct LowMemory {
    /// Free memory, in bytes
    pub largest_free: usize,
}

/// A name without trailing "(...)" and "[...]" tags.
fn short_title(name: &str) -> &str {
    let mut title = name.trim_end();
    while let Some(open) = match title.chars().last() {
        Some(')') => title.rfind('('),
        Some(']') => title.rfind('['),
        _ => None,
    } {
        let rest = title[..open].trim_end();
        if rest.is_empty() {
            break;
        }
        title = rest;
    }
    title
}
