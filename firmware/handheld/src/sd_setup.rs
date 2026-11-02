//! Tools > Set Up SD Card: create the folders Game Bub uses, check ROMs, remove the hidden files
//! macOS leaves behind, or erase and format the card first.

use std::path::Path;

use anyhow::Context;

use crate::device::{drivers::sdcard, Device};
use crate::wifi_setup::TaskUpdate;
use crate::{library, ui};

const ROOT: &str = "/sdcard";
const SYSTEM_DIR: &str = "/sdcard/system";

/// BIOS files kept when erasing: the user's own copies, which can't be downloaded again (see
/// "Custom BIOS files" in the Game Bub user guide).
const BIOS_FILES: &[&str] =
    &["gameboy.bios-dmg.bin", "gameboy.bios-cgb.bin", "gba.bios.bin", "gba.bios-fast.bin"];

/// Folders Game Bub uses, besides one per system under /roms and /boxart.
const FOLDERS: &[&str] = &["roms", "saves", "boxart", "system", "settings"];

/// Why the card isn't usable, for the Set Up SD Card screen (empty if it is).
pub fn status() -> String {
    match std::fs::metadata(ROOT) {
        Ok(_) => String::new(),
        Err(stat) => match sdcard::MOUNT_ERROR.lock().unwrap().as_deref() {
            Some(mount) => format!("The SD card couldn't be read: {mount}"),
            None => format!("The SD card isn't available: {stat}"),
        },
    }
}

fn task(label: &str, result: Option<(bool, String)>) {
    ui::send(ui::Message::Task(TaskUpdate { label: label.to_string(), progress: -1.0, result }));
}

/// Read every ROM in /roms and compare it with the database, in the background. Only reads.
pub fn start_check() {
    task("Checking ROMs...", None);
    crate::jobs::run(|| match check_roms() {
        Ok((ok, message)) => task("", Some((ok, message))),
        Err(e) => task("", Some((false, format!("{e:#}")))),
    });
}

fn check_roms() -> anyhow::Result<(bool, String)> {
    if !Path::new(ROOT).exists() {
        anyhow::bail!("There's no SD card, or it isn't formatted.");
    }
    let mut buf: Vec<u8> = Vec::new();
    for size in [16 * 1024, 4 * 1024] {
        if buf.try_reserve_exact(size).is_ok() {
            buf.resize(size, 0);
            break;
        }
    }
    if buf.is_empty() {
        anyhow::bail!("Not enough free memory.");
    }
    let roms: Vec<(bool, library::Game)> = library::SYSTEMS
        .iter()
        .flat_map(|system| {
            let gba = system.folder == "Game Boy Advance";
            system.games().into_iter().map(move |game| (gba, game))
        })
        .collect();
    let (mut good, mut unknown) = (0, 0);
    let mut bad = vec![];
    for (i, (gba, game)) in roms.iter().enumerate() {
        task(&format!("Checking ROM {} of {}\n{}", i + 1, roms.len(), game.name), None);
        let mut file = std::fs::File::open(&game.path)
            .with_context(|| format!("Couldn't open {}", game.name))?;
        let mut crc = 0u32;
        let mut header = Vec::new();
        loop {
            let n = std::io::Read::read(&mut file, &mut buf)
                .with_context(|| format!("Couldn't read {}", game.name))?;
            if n == 0 {
                break;
            }
            if header.len() < 0x180 {
                let take = (0x180 - header.len()).min(n);
                header.extend_from_slice(&buf[..take]);
            }
            crc = unsafe { esp_idf_svc::sys::esp_rom_crc32_le(crc, buf.as_ptr(), n as u32) };
        }
        match crate::cart_tool::known_crc32(*gba, &header) {
            Some(expected) if expected == crc => good += 1,
            Some(expected) => {
                log::warn!("{}: CRC32 {crc:08x}, expected {expected:08x}", game.name);
                bad.push(game.name.clone());
            }
            None => unknown += 1,
        }
    }
    let mut message = format!("{good} of {} ROMs read back correctly.", roms.len());
    if unknown > 0 {
        message += &format!("\n{unknown} aren't in the database, so couldn't be checked.");
    }
    if !bad.is_empty() {
        message += &format!("\n\nRead back wrong:\n{}", bad.join("\n"));
    }
    Ok((bad.is_empty(), message))
}

/// Remove the hidden files macOS leaves on the card, in the background.
pub fn start_mac_cleanup() {
    task("Removing macOS files...", None);
    crate::jobs::run(|| match remove_mac_files() {
        Ok(message) => task("", Some((true, message))),
        Err(e) => task("", Some((false, format!("{e:#}")))),
    });
}

/// Folders macOS creates (Spotlight index, file system events, trash, ZIP resource forks).
const MAC_FOLDERS: &[&str] = &[
    ".Spotlight-V100",
    ".fseventsd",
    ".Trashes",
    ".TemporaryItems",
    ".DocumentRevisions-V100",
    "__MACOSX",
];

/// Whether a file or folder is one macOS leaves behind: `._` resource fork files (one next to
/// every file copied from a Mac), `.DS_Store`, `.apdisk` and the folders above.
fn is_mac_file(name: &str, is_dir: bool) -> bool {
    if is_dir {
        MAC_FOLDERS.contains(&name)
    } else {
        name.starts_with("._") || name == ".DS_Store" || name == ".apdisk"
    }
}

#[derive(Default)]
struct Removed {
    files: u32,
    bytes: u64,
}

fn remove_mac_files() -> anyhow::Result<String> {
    if !Path::new(ROOT).exists() {
        anyhow::bail!("There's no SD card, or it isn't formatted.");
    }
    let mut removed = Removed::default();
    clean_folder(Path::new(ROOT), &mut removed, false)?;
    Ok(match removed.files {
        0 => "There were no macOS files on the card.".into(),
        n => format!(
            "Removed {n} macOS file{} ({}).",
            if n == 1 { "" } else { "s" },
            describe_size(removed.bytes)
        ),
    })
}

/// Remove the macOS files in a folder and its subfolders, or everything in it if `all` is set.
/// Works in small batches, so folders with thousands of files don't need much memory.
fn clean_folder(dir: &Path, removed: &mut Removed, all: bool) -> anyhow::Result<()> {
    const BATCH: usize = 32;
    let shown = dir.strip_prefix(ROOT).unwrap_or(dir).display().to_string();
    task(&format!("Removing macOS files...\n/{shown}"), None);

    // Delete in batches, reading the folder again after each one: deleting while reading
    // a folder isn't safe on FAT.
    let mut subfolders = vec![];
    loop {
        let mut batch: Vec<(std::path::PathBuf, bool)> = Vec::with_capacity(BATCH);
        subfolders.clear();
        for entry in std::fs::read_dir(dir).with_context(|| format!("Couldn't read /{shown}"))? {
            let entry = entry.with_context(|| format!("Couldn't read /{shown}"))?;
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if all || is_mac_file(&name, is_dir) {
                batch.push((entry.path(), is_dir));
                if batch.len() == BATCH {
                    break;
                }
            } else if is_dir {
                subfolders.push(entry.path());
            }
        }
        if batch.is_empty() {
            break;
        }
        for (path, is_dir) in batch {
            if is_dir {
                clean_folder(&path, removed, true)?;
                std::fs::remove_dir(&path)
                    .with_context(|| format!("Couldn't remove {}", path.display()))?;
            } else {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                std::fs::remove_file(&path)
                    .with_context(|| format!("Couldn't remove {}", path.display()))?;
                removed.files += 1;
                removed.bytes += size;
            }
        }
    }
    for folder in subfolders {
        clean_folder(&folder, removed, false)?;
    }
    Ok(())
}

fn describe_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000 => format!("{:.1} MB", b as f64 / 1e6),
        b if b >= 1_000 => format!("{} KB", b / 1_000),
        b => format!("{b} bytes"),
    }
}

/// Set up the SD card in the background, erasing it first if `erase` is set.
pub fn start(erase: bool) {
    task(if erase { "Erasing the SD card..." } else { "Creating folders..." }, None);
    crate::jobs::run(move || {
        let result = if erase { erase_and_set_up() } else { create_folders().map(describe_created) };
        match result {
            Ok(message) => task("", Some((true, message))),
            Err(e) => task("", Some((false, format!("{e:#}")))),
        }
    });
}

fn describe_created(created: Vec<String>) -> String {
    if created.is_empty() {
        "All the folders were already there.".into()
    } else {
        format!("Created:\n{}", created.join("\n"))
    }
}

/// Create any missing folders. Returns the ones created.
fn create_folders() -> anyhow::Result<Vec<String>> {
    if !Path::new(ROOT).exists() {
        anyhow::bail!("There's no SD card, or it isn't formatted.\nUse \"Erase and format\" to set it up.");
    }
    let mut folders: Vec<String> = FOLDERS.iter().map(|f| f.to_string()).collect();
    for system in library::SYSTEMS {
        folders.push(format!("roms/{}", system.folder));
        folders.push(format!("boxart/{}", system.folder));
    }
    let mut created = vec![];
    for folder in folders {
        let path = Path::new(ROOT).join(&folder);
        if !path.is_dir() {
            std::fs::create_dir_all(&path).with_context(|| format!("Couldn't create /{folder}"))?;
            created.push(format!("/{folder}"));
        }
    }
    Ok(created)
}

fn erase_and_set_up() -> anyhow::Result<String> {
    let bios = read_bios_files()?;
    let card = Device::lock().sdcard;
    let card = match card {
        Some(card) => card,
        None => {
            // The card couldn't be read (e.g. exFAT): set it up so it can be formatted.
            task("Preparing the card...\nThis can take a minute.", None);
            let (card, temporary) = sdcard::mount_or_format(ROOT)
                .map_err(|e| anyhow::anyhow!("No usable SD card was found ({e}).\nCheck that the card is inserted."))?;
            log::info!("SD card mounted for formatting (temporary file system: {temporary})");
            Device::lock().sdcard = Some(card);
            card
        }
    };
    task("Erasing and formatting...\nThis can take a minute.", None);
    card.format(ROOT).map_err(|e| anyhow::anyhow!("Formatting failed: {e}"))?;
    log::info!("SD card formatted");

    task("Creating folders...", None);
    create_folders()?;
    let mut message = format!("The SD card was erased and set up.\n{}", card_size());
    let mut kept = vec![];
    for (name, data) in &bios {
        match std::fs::write(Path::new(SYSTEM_DIR).join(name), data) {
            Ok(()) => kept.push(*name),
            Err(e) => message += &format!("\n{name} couldn't be put back: {e}"),
        }
    }
    if !kept.is_empty() {
        message += &format!("\nKept in /system: {}", kept.join(", "));
    }
    Ok(message)
}

/// Read the BIOS files on the card, to put back after formatting. Fails (before anything is
/// erased) if one can't be read.
fn read_bios_files() -> anyhow::Result<Vec<(&'static str, Vec<u8>)>> {
    let mut files = vec![];
    for &name in BIOS_FILES {
        let path = Path::new(SYSTEM_DIR).join(name);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let mut data = Vec::new();
        if data.try_reserve_exact(meta.len() as usize).is_err() {
            anyhow::bail!("Not enough free memory to keep /system/{name}.\nNothing was erased.");
        }
        std::io::Read::read_to_end(&mut std::fs::File::open(&path)?, &mut data)
            .with_context(|| format!("Couldn't read /system/{name}.\nNothing was erased."))?;
        files.push((name, data));
    }
    Ok(files)
}

/// Card size, e.g. "Size: 63.9 GB".
fn card_size() -> String {
    let (mut total, mut free) = (0u64, 0u64);
    let path = c"/sdcard";
    let ok = unsafe { esp_idf_svc::sys::esp_vfs_fat_info(path.as_ptr(), &mut total, &mut free) } == 0;
    if !ok {
        return String::new();
    }
    format!("Size: {:.1} GB", total as f64 / 1e9)
}
