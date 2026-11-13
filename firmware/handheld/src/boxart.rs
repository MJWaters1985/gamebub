//! Box art: download from the libretro thumbnail library and convert to the small thumbnails
//! shown by "Run ROM" (see library::load_thumbnail).
//!
//! libretro's box art is large PNG files. Decoding PNG needs a 32 KiB window in one piece, more
//! than the ESP32 has free (the heap is fragmented into blocks of about 30 KiB). So the images
//! go through images.weserv.nl, an open source image proxy, which returns a small baseline
//! JPEG. That is decoded by the TJpgDec decoder in the ESP32-S3 ROM, which needs about 3 KiB.

use std::ffi::c_void;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use crate::library::System;

/// Thumbnail size (largest side, pixels).
pub const THUMBNAIL_SIZE: u32 = 120;

const LIBRETRO_THUMBNAILS: &str = "thumbnails.libretro.com";
/// The same library on GitHub (one repository per system), used first: it stays up when
/// thumbnails.libretro.com doesn't.
const GITHUB_THUMBNAILS: &str = "raw.githubusercontent.com/libretro-thumbnails";
const IMAGE_PROXY: &str = "http://images.weserv.nl";

/// libretro playlist (folder) name for a system.
fn libretro_system(system: &System) -> &'static str {
    match system.folder {
        "Game Boy" => "Nintendo - Game Boy",
        "Game Boy Color" => "Nintendo - Game Boy Color",
        "Super Nintendo" => "Nintendo - Super Nintendo Entertainment System",
        _ => "Nintendo - Game Boy Advance",
    }
}

/// URLs of a game's box art (from the libretro thumbnail library: GitHub mirror, then
/// libretro's server) as a THUMBNAIL_SIZE JPEG. Game names are No-Intro names;
/// libretro replaces &*/:`<>?\| with _ in file names.
pub fn thumbnail_urls(system: &System, game_name: &str) -> [String; 2] {
    let file_name: String = game_name
        .chars()
        .map(|c| if "&*/:`<>?\\|".contains(c) { '_' } else { c })
        .collect();
    let system_name = libretro_system(system);
    let repository = system_name.replace(' ', "_");
    let proxied = |base: String| {
        format!(
            "{IMAGE_PROXY}/?url={base}/Named_Boxarts/{}.png\
             &w={THUMBNAIL_SIZE}&h={THUMBNAIL_SIZE}&fit=inside&output=jpg&q=85",
            percent_encode(&file_name)
        )
    };
    [
        proxied(format!("{GITHUB_THUMBNAILS}/{}/master", percent_encode(&repository))),
        proxied(format!("{LIBRETRO_THUMBNAILS}/{}", percent_encode(system_name))),
    ]
}

/// Game Boy for Game Boy Color, and the other way round.
fn other_game_boy_system(system: &System) -> Option<System> {
    let other = match system.folder {
        "Game Boy" => "Game Boy Color",
        "Game Boy Color" => "Game Boy",
        _ => return None,
    };
    crate::library::SYSTEMS.iter().find(|s| s.folder == other).copied()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'(' | b')' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Where a downloaded JPEG is kept until it's converted.
pub fn download_path(system: &System, game_name: &str) -> PathBuf {
    system.thumbnail_path(game_name).with_extension("jpg")
}

// ---- TJpgDec in the ESP32-S3 ROM (esp_rom/esp32s3/include/esp32s3/rom/tjpgd.h) ----

#[repr(C)]
struct JRect {
    left: u16,
    right: u16,
    top: u16,
    bottom: u16,
}

#[repr(C)]
struct JDec {
    dctr: u32,
    dptr: *mut u8,
    inbuf: *mut u8,
    dmsk: u8,
    scale: u8,
    msx: u8,
    msy: u8,
    qtid: [u8; 3],
    dcv: [i16; 3],
    nrst: u16,
    width: u32,
    height: u32,
    huffbits: [[*mut u8; 2]; 2],
    huffcode: [[*mut u16; 2]; 2],
    huffdata: [[*mut u8; 2]; 2],
    qttbl: [*mut i32; 4],
    workbuf: *mut c_void,
    mcubuf: *mut u8,
    pool: *mut c_void,
    sz_pool: u32,
    infunc: Option<unsafe extern "C" fn(*mut JDec, *mut u8, u32) -> u32>,
    device: *mut c_void,
}

type InFunc = unsafe extern "C" fn(*mut JDec, *mut u8, u32) -> u32;
type OutFunc = unsafe extern "C" fn(*mut JDec, *mut c_void, *mut JRect) -> u32;

extern "C" {
    fn jd_prepare(jd: *mut JDec, infunc: InFunc, pool: *mut c_void, sz_pool: u32, dev: *mut c_void) -> i32;
    fn jd_decomp(jd: *mut JDec, outfunc: OutFunc, scale: u8) -> i32;
}

/// Decoder session state (TJpgDec's "device").
struct Session {
    input: std::fs::File,
    output: std::fs::File,
    width: u32,
    error: Option<std::io::Error>,
}

unsafe extern "C" fn jpeg_input(jd: *mut JDec, buf: *mut u8, len: u32) -> u32 {
    let session = &mut *((*jd).device as *mut Session);
    if buf.is_null() {
        // Skip bytes.
        return match session.input.seek(SeekFrom::Current(len as i64)) {
            Ok(_) => len,
            Err(_) => 0,
        };
    }
    let out = std::slice::from_raw_parts_mut(buf, len as usize);
    let mut filled = 0;
    while filled < out.len() {
        match session.input.read(&mut out[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => filled += n,
        }
    }
    filled as u32
}

unsafe extern "C" fn jpeg_output(jd: *mut JDec, bitmap: *mut c_void, rect: *mut JRect) -> u32 {
    let session = &mut *((*jd).device as *mut Session);
    let rect = &*rect;
    let w = (rect.right - rect.left + 1) as usize;
    let h = (rect.bottom - rect.top + 1) as usize;
    let pixels = std::slice::from_raw_parts(bitmap as *const u8, w * h * 3);
    for row in 0..h {
        let offset = 8 + ((rect.top as usize + row) * session.width as usize + rect.left as usize) * 3;
        let result = session
            .output
            .seek(SeekFrom::Start(offset as u64))
            .and_then(|_| session.output.write_all(&pixels[row * w * 3..(row + 1) * w * 3]));
        if let Err(e) = result {
            session.error = Some(e);
            return 0; // Interrupt decoding.
        }
    }
    1
}

/// Convert a baseline JPEG to a thumbnail file (see library::load_thumbnail).
pub fn jpeg_to_thumbnail(jpeg_path: &Path, thumb_path: &Path) -> anyhow::Result<()> {
    if let Some(dir) = thumb_path.parent() {
        std::fs::create_dir_all(dir).context("Couldn't create the box art folder")?;
    }
    let temp = crate::core::temp_path(thumb_path);
    let mut session = Box::new(Session {
        input: std::fs::File::open(jpeg_path).context("Couldn't open the box art")?,
        output: std::fs::File::create(&temp).context("Couldn't create the thumbnail")?,
        width: 0,
        error: None,
    });
    let mut pool = vec![0u8; 4096];
    let mut jd: Box<JDec> = Box::new(unsafe { std::mem::zeroed() });
    let session_ptr = &mut *session as *mut Session as *mut c_void;
    let result = unsafe { jd_prepare(&mut *jd, jpeg_input, pool.as_mut_ptr() as *mut c_void, pool.len() as u32, session_ptr) };
    if result != 0 {
        bail!("Unsupported box art image (error {result})");
    }
    let (width, height) = (jd.width, jd.height);
    if width == 0 || height == 0 || width > 192 || height > 192 {
        bail!("Unexpected box art size {width}x{height}");
    }
    session.width = width;
    let mut header = Vec::with_capacity(8);
    header.extend_from_slice(b"BBX1");
    header.extend_from_slice(&(width as u16).to_le_bytes());
    header.extend_from_slice(&(height as u16).to_le_bytes());
    session.output.write_all(&header)?;
    let result = unsafe { jd_decomp(&mut *jd, jpeg_output, 0) };
    if let Some(e) = session.error.take() {
        return Err(e).context("Couldn't write the thumbnail");
    }
    if result != 0 {
        bail!("Couldn't decode the box art (error {result})");
    }
    session.output.sync_all()?;
    drop(session);
    let _ = std::fs::remove_file(thumb_path);
    std::fs::rename(&temp, thumb_path)?;
    Ok(())
}

/// Games in /roms/<system> that don't have box art yet.
pub fn missing_in_library() -> Vec<(System, String)> {
    crate::library::SYSTEMS
        .iter()
        .flat_map(|system| {
            system
                .games()
                .into_iter()
                .filter(|game| !system.thumbnail_path(&game.name).exists())
                .map(|game| (*system, game.name))
        })
        .collect()
}

/// Result of fetching box art.
pub struct FetchResult {
    pub found: usize,
    pub not_found: usize,
    pub failed: usize,
}

/// Download box art for games, then convert it. Wi-Fi is on only while downloading (decoding
/// and Wi-Fi together would need more memory than is free). `progress` gets a status line and
/// the fraction done.
pub fn fetch(games: &[(System, String)], progress: &mut dyn FnMut(&str, f32)) -> anyhow::Result<FetchResult> {
    let mut result = FetchResult { found: 0, not_found: 0, failed: 0 };
    if games.is_empty() {
        return Ok(result);
    }
    progress("Connecting to Wi-Fi...", -1.0);
    let mut downloaded = vec![];
    {
        let mut wifi = crate::net::connect_saved()?;
        let mut errors_in_a_row = 0;
        for (i, (system, name)) in games.iter().enumerate() {
            progress(&format!("Downloading {} of {}\n{name}", i + 1, games.len()), i as f32 / games.len() as f32 * 0.8);
            let path = download_path(system, name);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).context("Couldn't create the box art folder")?;
            }
            // Game Boy games with Color enhancements (e.g. Pokemon Yellow) are filed as Game Boy
            // Color by their header, but are Game Boy games in the thumbnail library (and the
            // other way round): try the other system's folder if needed.
            let mut found = Ok(None);
            let systems = std::iter::once(*system).chain(other_game_boy_system(system));
            'search: for system in systems {
                for url in thumbnail_urls(&system, name) {
                    found = wifi.download(&url, &path);
                    if matches!(found, Ok(Some(_))) {
                        break 'search;
                    }
                }
            }
            match found {
                Ok(Some(_)) => {
                    downloaded.push((*system, name.clone(), path));
                    errors_in_a_row = 0;
                }
                Ok(None) => {
                    result.not_found += 1;
                    errors_in_a_row = 0;
                }
                Err(e) => {
                    log::warn!("Box art download failed for {name}: {e:#}");
                    result.failed += 1;
                    errors_in_a_row += 1;
                    if errors_in_a_row >= 3 {
                        bail!("Box art downloads keep failing: {e:#}");
                    }
                }
            }
        }
    }
    for (i, (system, name, path)) in downloaded.iter().enumerate() {
        progress(&format!("Converting {} of {}", i + 1, downloaded.len()), 0.8 + 0.2 * i as f32 / downloaded.len() as f32);
        match jpeg_to_thumbnail(path, &system.thumbnail_path(name)) {
            Ok(()) => result.found += 1,
            Err(e) => {
                log::warn!("Box art conversion failed for {name}: {e:#}");
                result.failed += 1;
            }
        }
        let _ = std::fs::remove_file(path);
    }
    Ok(result)
}
