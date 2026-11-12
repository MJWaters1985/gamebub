//! Cartridge identification, ROM dumping and copying saves to and from the SD card.
//!
//! Mapper and save memory handling follows FlashGBX (https://github.com/lesserkuma/FlashGBX).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};

use super::db::{self, Crc32, GameEntry};
use super::link::{CartMode, EepromSize, Link};
use crate::bitstream::gba::save_type_detector::SaveTypeDetector;
use crate::bitstream::gba::SaveType as DetectedSaveType;

pub const DIR_ROMS: &str = "/sdcard/roms";

/// Progress callback: fraction done (0 to 1).
pub type Progress<'a> = &'a mut dyn FnMut(f32);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum System {
    GameBoy,
    GameBoyColor,
    GameBoyAdvance,
}

impl System {
    /// Folder name under /roms.
    pub fn folder(self) -> &'static str {
        match self {
            Self::GameBoy => "Game Boy",
            Self::GameBoyColor => "Game Boy Color",
            Self::GameBoyAdvance => "Game Boy Advance",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::GameBoy => "gb",
            Self::GameBoyColor => "gbc",
            Self::GameBoyAdvance => "gba",
        }
    }
}

/// Game Boy memory bank controller.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mbc {
    None,
    Mbc1,
    Mbc2,
    Mbc3,
    Mbc5,
    /// MBC7 (with an accelerometer and EEPROM save), e.g. Kirby Tilt 'n' Tumble
    Mbc7,
    Unsupported(u8),
}

impl Mbc {
    fn from_cart_type(t: u8) -> Self {
        match t {
            0x00 | 0x08 | 0x09 => Self::None,
            0x01..=0x03 => Self::Mbc1,
            0x05 | 0x06 => Self::Mbc2,
            0x0F..=0x13 => Self::Mbc3,
            0x19..=0x1E => Self::Mbc5,
            0x22 => Self::Mbc7,
            other => Self::Unsupported(other),
        }
    }

    pub fn name(self) -> String {
        match self {
            Self::None => "No mapper".into(),
            Self::Mbc1 => "MBC1".into(),
            Self::Mbc2 => "MBC2".into(),
            Self::Mbc3 => "MBC3".into(),
            Self::Mbc5 => "MBC5".into(),
            Self::Mbc7 => "MBC7".into(),
            Self::Unsupported(t) => format!("Unsupported mapper ({t:#04x})"),
        }
    }
}

/// Save memory on the cartridge.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SaveKind {
    None,
    /// Game Boy cartridge RAM
    DmgRam(u32),
    /// GBA SRAM / FRAM
    AgbSram(u32),
    /// GBA Flash
    AgbFlash(u32),
    /// GBA EEPROM
    AgbEeprom(EepromSize),
    /// Game Boy MBC7 serial EEPROM (93LC56)
    Mbc7Eeprom(u32),
    /// A save type this tool can't handle
    Unsupported(&'static str),
}

impl SaveKind {
    pub fn size(self) -> u32 {
        match self {
            Self::None | Self::Unsupported(_) => 0,
            Self::DmgRam(n) | Self::AgbSram(n) | Self::AgbFlash(n) | Self::Mbc7Eeprom(n) => n,
            Self::AgbEeprom(EepromSize::Small) => 512,
            Self::AgbEeprom(EepromSize::Large) => 8192,
        }
    }

    pub fn describe(self) -> String {
        let kib = |n: u32| {
            if n >= 1024 {
                format!("{} KiB", n / 1024)
            } else {
                format!("{n} bytes")
            }
        };
        match self {
            Self::None => "None".into(),
            Self::DmgRam(n) => format!("{} SRAM", kib(n)),
            Self::AgbSram(n) => format!("{} SRAM", kib(n)),
            Self::AgbFlash(n) => format!("{} Flash", kib(n)),
            Self::AgbEeprom(EepromSize::Small) => "512 bytes EEPROM".into(),
            Self::AgbEeprom(EepromSize::Large) => "8 KiB EEPROM".into(),
            Self::Mbc7Eeprom(n) => format!("{} EEPROM", kib(n)),
            Self::Unsupported(name) => format!("{name} (unsupported)"),
        }
    }
}

/// What's known about the inserted cartridge.
#[derive(Clone, Debug)]
pub struct CartInfo {
    pub mode: CartMode,
    pub system: System,
    /// Title from the cartridge header
    pub header_title: String,
    /// Database entry, if the cartridge is known
    pub db: Option<GameEntry>,
    pub rom_size: u32,
    pub mbc: Mbc,
    pub save: SaveKind,
    /// Whether the save type is a guess (GBA cartridge not in the database)
    pub save_guessed: bool,
    header: Vec<u8>,
}

impl CartInfo {
    /// Name used for files: the database name, or the header title.
    pub fn name(&self) -> String {
        let name = match &self.db {
            Some(entry) => entry.name.clone(),
            None if self.header_title.is_empty() => "Unknown cartridge".to_string(),
            None => self.header_title.clone(),
        };
        sanitize_file_name(&name)
    }

    pub fn rom_path(&self) -> PathBuf {
        Path::new(DIR_ROMS)
            .join(self.system.folder())
            .join(format!("{}.{}", self.name(), self.system.extension()))
    }

    /// Find this cartridge's ROM on the SD card: the file Dump ROM writes, or else a ROM in the
    /// system's folder (or the other Game Boy folder) with the same size and header.
    pub fn find_rom_on_sd(&self) -> Option<PathBuf> {
        let dumped = self.rom_path();
        if dumped.is_file() {
            return Some(dumped);
        }
        let header = match self.mode {
            CartMode::Dmg => 0x134..0x150,
            CartMode::Agb => 0xA0..0xC0,
        };
        let folders: &[System] = match self.system {
            System::GameBoyAdvance => &[System::GameBoyAdvance],
            System::GameBoy => &[System::GameBoy, System::GameBoyColor],
            System::GameBoyColor => &[System::GameBoyColor, System::GameBoy],
        };
        let extensions: &[&str] = match self.system {
            System::GameBoyAdvance => &["gba"],
            _ => &["gb", "gbc"],
        };
        for system in folders {
            let Ok(entries) = std::fs::read_dir(Path::new(DIR_ROMS).join(system.folder())) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
                if name.starts_with("._")
                    || !extension.is_some_and(|e| extensions.contains(&e.as_str()))
                {
                    continue;
                }
                // Only open files of the right size.
                if entry.metadata().map(|m| m.len()).ok() != Some(self.rom_size as u64) {
                    continue;
                }
                let path = entry.path();
                let mut bytes = [0u8; 0x150];
                let matches = File::open(&path)
                    .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes[..header.end]).map(|_| ()))
                    .is_ok_and(|_| bytes[header.clone()] == self.header[header.clone()]);
                if matches {
                    return Some(path);
                }
            }
        }
        None
    }

    pub fn has_save(&self) -> bool {
        self.save.size() > 0
    }

    /// Game Boy cartridge type byte (mapper and extras).
    pub fn cart_type(&self) -> u8 {
        self.header[0x147]
    }

    /// GBA game code, e.g. "BPEE".
    pub fn game_code(&self) -> [u8; 4] {
        self.header[0xAC..0xB0].try_into().unwrap()
    }
}

/// Whether the inserted cartridge is still the one `info` describes: same kind of cartridge
/// (the slot's GB / GBA switch), then the same header after a power cycle. Any read error counts
/// as a change.
pub fn is_same_cartridge(link: &mut Link, info: &CartInfo) -> bool {
    // Check the switch first, so a GBA cartridge is never powered at Game Boy voltage.
    if !matches!(link.cart_mode_switch(), Ok(mode) if mode == info.mode) {
        return false;
    }
    if link.power_cycle().is_err() {
        return false;
    }
    let mut header = vec![0u8; info.header.len()];
    let read = match info.mode {
        CartMode::Dmg => link.dmg_read_rom(0, &mut header),
        CartMode::Agb => link.agb_read_rom(0, &mut header),
    };
    read.is_ok() && header == info.header
}

/// Replace characters that FAT doesn't allow in file names.
fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    cleaned.trim().trim_end_matches('.').to_string()
}

const DMG_LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];

fn header_title(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end])
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .collect::<String>()
        .trim()
        .to_string()
}

/// Read the cartridge header and work out what the cartridge is.
pub fn identify(link: &mut Link) -> anyhow::Result<CartInfo> {
    let mode = link.cart_mode_switch()?;
    link.set_mode(mode)?;
    let mut header = vec![0u8; 0x180];
    match mode {
        CartMode::Dmg => link.dmg_read_rom(0, &mut header)?,
        CartMode::Agb => link.agb_read_rom(0, &mut header)?,
    }
    let db = db::lookup(mode, &header);
    match mode {
        CartMode::Dmg => identify_dmg(header, db),
        CartMode::Agb => identify_agb(link, header, db),
    }
}

fn identify_dmg(header: Vec<u8>, db: Option<GameEntry>) -> anyhow::Result<CartInfo> {
    let checksum = header[0x134..=0x14C]
        .iter()
        .fold(0u8, |x, &b| x.wrapping_sub(b).wrapping_sub(1));
    if header[0x104..0x134] != DMG_LOGO || checksum != header[0x14D] {
        bail!("Couldn't read the cartridge header.\nClean the cartridge contacts and try again.");
    }
    let cgb = header[0x143] & 0x80 != 0;
    // Titles are 11-16 characters; CGB games use the end for a manufacturer code / flag.
    let title = header_title(&header[0x134..if cgb { 0x143 } else { 0x144 }]);
    let mbc = Mbc::from_cart_type(header[0x147]);
    let rom_size = match header[0x148] {
        n @ 0..=8 => 0x8000u32 << n,
        _ => db.as_ref().map(|e| e.rom_size).unwrap_or(0x8000),
    };
    let save = match mbc {
        Mbc::Mbc2 => SaveKind::DmgRam(512),
        // 93LC56: 128 16-bit words. (Command Master's 93LC66 is twice the size; not handled.)
        Mbc::Mbc7 => SaveKind::Mbc7Eeprom(256),
        _ => match header[0x149] {
            1 => SaveKind::DmgRam(0x800),
            2 => SaveKind::DmgRam(0x2000),
            3 => SaveKind::DmgRam(0x8000),
            4 => SaveKind::DmgRam(0x20000),
            5 => SaveKind::DmgRam(0x10000),
            _ => SaveKind::None,
        },
    };
    // Only cartridges with a battery keep their save.
    let has_battery = matches!(header[0x147], 0x03 | 0x06 | 0x09 | 0x0F | 0x10 | 0x13 | 0x1B | 0x1E);
    Ok(CartInfo {
        mode: CartMode::Dmg,
        system: if cgb { System::GameBoyColor } else { System::GameBoy },
        header_title: title,
        db,
        rom_size,
        mbc,
        save: if has_battery || mbc == Mbc::Mbc7 { save } else { SaveKind::None },
        save_guessed: false,
        header,
    })
}

fn identify_agb(link: &mut Link, header: Vec<u8>, db: Option<GameEntry>) -> anyhow::Result<CartInfo> {
    let checksum = header[0xA0..0xBD]
        .iter()
        .fold(0u8, |x, &b| x.wrapping_sub(b))
        .wrapping_sub(0x19);
    if checksum != header[0xBD] || header[0xB2] != 0x96 {
        bail!("Couldn't read the cartridge header.\nClean the cartridge contacts and try again.");
    }
    let title = header_title(&header[0xA0..0xAC]);
    let (rom_size, save, save_guessed) = match &db {
        Some(entry) => {
            let save = match entry.agb_save_type {
                0 => SaveKind::None,
                1 => SaveKind::AgbEeprom(EepromSize::Small),
                2 => SaveKind::AgbEeprom(EepromSize::Large),
                3 => SaveKind::AgbSram(0x8000),
                4 => SaveKind::AgbFlash(0x10000),
                5 => SaveKind::AgbFlash(0x20000),
                7 => SaveKind::AgbSram(0x10000),
                6 => SaveKind::Unsupported("DACS Flash"),
                _ => SaveKind::Unsupported("1 Mbit SRAM"),
            };
            (entry.rom_size, save, false)
        }
        None => (detect_agb_rom_size(link)?, SaveKind::None, true),
    };
    Ok(CartInfo {
        mode: CartMode::Agb,
        system: System::GameBoyAdvance,
        header_title: title,
        db,
        rom_size,
        mbc: Mbc::None,
        save,
        save_guessed,
        header,
    })
}

/// Find the ROM size of an unknown GBA cartridge: past the end of the ROM, reads return the
/// low bits of the address (open bus).
fn detect_agb_rom_size(link: &mut Link) -> anyhow::Result<u32> {
    let mut buf = [0u8; 16];
    for size in [0x40_0000u32, 0x80_0000, 0x100_0000] {
        link.agb_read_rom(size, &mut buf)?;
        let open_bus = buf.chunks_exact(2).enumerate().all(|(i, h)| {
            u16::from_le_bytes([h[0], h[1]]) == (((size >> 1) + i as u32) & 0xFFFF) as u16
        });
        if open_bus {
            return Ok(size);
        }
    }
    Ok(0x200_0000)
}

/// Read the whole ROM, passing each chunk to `sink`.
fn read_rom(
    link: &mut Link,
    info: &CartInfo,
    progress: Progress,
    sink: &mut dyn FnMut(&[u8]) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 0x4000];
    match info.mode {
        CartMode::Agb => {
            let chunk = 0x4000;
            let mut big = vec![0u8; chunk];
            let mut pos = 0;
            while pos < info.rom_size {
                link.agb_read_rom(pos, &mut big)?;
                sink(&big)?;
                pos += chunk as u32;
                progress(pos as f32 / info.rom_size as f32);
            }
        }
        CartMode::Dmg => {
            if let Mbc::Unsupported(_) = info.mbc {
                bail!("This cartridge's mapper ({}) isn't supported.", info.mbc.name());
            }
            let banks = (info.rom_size / 0x4000).max(2);
            link.dmg_mbc_reset()?;
            for bank in 0..banks {
                let start = select_rom_bank(link, info.mbc, bank)?;
                link.dmg_read_rom(start, &mut buf)?;
                sink(&buf)?;
                progress((bank + 1) as f32 / banks as f32);
            }
        }
    }
    Ok(())
}

/// Map a ROM bank, returning the address it can be read from.
fn select_rom_bank(link: &mut Link, mbc: Mbc, bank: u32) -> anyhow::Result<u16> {
    let start = if bank == 0 { 0 } else { 0x4000 };
    match mbc {
        Mbc::None | Mbc::Unsupported(_) => {}
        Mbc::Mbc1 => {
            // Mode 1 makes banks 0x20, 0x40, 0x60 readable at 0x0000.
            link.dmg_write(0x6000, 1)?;
            link.dmg_write(0x2000, bank as u8)?;
            link.dmg_write(0x4000, (bank >> 5) as u8)?;
            return Ok(if bank & 0x1F != 0 { 0x4000 } else { 0 });
        }
        Mbc::Mbc2 | Mbc::Mbc3 | Mbc::Mbc7 => link.dmg_write(0x2100, bank as u8)?,
        Mbc::Mbc5 => {
            link.dmg_write(0x3000, (bank >> 8) as u8)?;
            link.dmg_write(0x2100, bank as u8)?;
        }
    }
    Ok(start)
}

/// Result of a ROM dump.
pub struct DumpResult {
    pub path: PathBuf,
    pub crc32: u32,
    /// Some(true) if the CRC matches the database, Some(false) if it doesn't, None if unknown
    pub verified: Option<bool>,
    /// Save type detected from the ROM (GBA cartridges not in the database)
    pub detected_save: Option<SaveKind>,
}

/// Dump the ROM to /roms/<system>/<name>.<ext>.
pub fn dump_rom(link: &mut Link, info: &CartInfo, progress: Progress) -> anyhow::Result<DumpResult> {
    let path = info.rom_path();
    std::fs::create_dir_all(path.parent().unwrap()).context("Couldn't create the ROM folder")?;
    let temp = crate::core::temp_path(&path);
    let mut file = File::create(&temp).context("Couldn't create the ROM file")?;
    let mut crc = Crc32::new();
    let mut detector = SaveTypeDetector::new();
    let mut dmg_sum: u16 = 0;
    let mut offset = 0usize;
    let result = read_rom(link, info, progress, &mut |data| {
        crc.update(data);
        if info.mode == CartMode::Agb && info.save_guessed {
            detector.process(data);
        }
        if info.mode == CartMode::Dmg {
            for (i, &b) in data.iter().enumerate() {
                if !matches!(offset + i, 0x14E | 0x14F) {
                    dmg_sum = dmg_sum.wrapping_add(b as u16);
                }
            }
        }
        offset += data.len();
        file.write_all(data).context("Couldn't write the ROM file")
    });
    if let Err(e) = result.and_then(|_| file.sync_all().context("Couldn't write the ROM file")) {
        drop(file);
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    drop(file);
    let _ = std::fs::remove_file(&path);
    std::fs::rename(&temp, &path).context("Couldn't save the ROM file")?;

    let crc32 = crc.finish();
    let verified = match &info.db {
        Some(entry) => Some(entry.crc32 == crc32),
        None if info.mode == CartMode::Dmg => {
            Some(dmg_sum == u16::from_be_bytes([info.header[0x14E], info.header[0x14F]]))
        }
        None => None,
    };
    let detected_save = info.save_guessed.then(|| agb_save_from_detector(detector.get()));
    Ok(DumpResult { path, crc32, verified, detected_save })
}

fn agb_save_from_detector(detected: DetectedSaveType) -> SaveKind {
    match detected {
        DetectedSaveType::EepromAuto | DetectedSaveType::Eeprom8K => SaveKind::AgbEeprom(EepromSize::Large),
        DetectedSaveType::Eeprom512 => SaveKind::AgbEeprom(EepromSize::Small),
        DetectedSaveType::Sram => SaveKind::AgbSram(0x8000),
        DetectedSaveType::Flash64K => SaveKind::AgbFlash(0x10000),
        DetectedSaveType::Flash128K => SaveKind::AgbFlash(0x20000),
        DetectedSaveType::None | DetectedSaveType::Autodetect => SaveKind::None,
    }
}

/// Work out the save type of a GBA cartridge that isn't in the database, by scanning the ROM.
pub fn detect_agb_save(link: &mut Link, info: &CartInfo, progress: Progress) -> anyhow::Result<SaveKind> {
    let mut detector = SaveTypeDetector::new();
    read_rom(link, info, progress, &mut |data| {
        detector.process(data);
        Ok(())
    })?;
    Ok(agb_save_from_detector(detector.get()))
}

// ---- Save memory ----

fn dmg_enable_ram(link: &mut Link, mbc: Mbc, enable: bool) -> anyhow::Result<()> {
    if mbc == Mbc::Mbc1 {
        link.dmg_write(0x6000, enable as u8)?;
    }
    link.dmg_write(0x0000, if enable { 0x0A } else { 0x00 })?;
    Ok(())
}

fn dmg_prepare_ram(link: &mut Link, mbc: Mbc) -> anyhow::Result<()> {
    if let Mbc::Unsupported(_) = mbc {
        bail!("This cartridge's mapper ({}) isn't supported.", mbc.name());
    }
    link.dmg_mbc_reset()?;
    link.dmg_set_cs_pulses(false, false)?;
    link.dmg_write(0x2000, 0x01)?;
    link.dmg_write(0x4000, 0x00)?;
    dmg_enable_ram(link, mbc, true)
}

/// Agb flash chip IDs that need different handling (Atmel, bootleg chips).
fn agb_flash_chip_id(link: &mut Link) -> anyhow::Result<u16> {
    link.agb_flash_commands(&[(0x5555, 0xAA), (0x2AAA, 0x55), (0x5555, 0x90)])?;
    let mut id = [0u8; 2];
    link.agb_read_sram(0, &mut id)?;
    link.agb_flash_commands(&[(0x5555, 0xAA), (0x2AAA, 0x55), (0x5555, 0xF0)])?;
    std::thread::sleep(Duration::from_millis(10));
    link.agb_flash_commands(&[(0, 0xF0)])?;
    std::thread::sleep(Duration::from_millis(10));
    Ok(u16::from_be_bytes(id))
}

fn agb_flash_select_bank(link: &mut Link, bank: u32) -> anyhow::Result<()> {
    link.agb_flash_commands(&[(0x5555, 0xAA), (0x2AAA, 0x55), (0x5555, 0xB0), (0, bank as u16)])?;
    Ok(())
}

/// Read the cartridge's save memory, passing it to `sink` in chunks.
pub fn read_save(
    link: &mut Link,
    info: &CartInfo,
    save: SaveKind,
    progress: Progress,
    sink: &mut dyn FnMut(&[u8]) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let size = save.size() as usize;
    match save {
        SaveKind::None => bail!("This cartridge has no save memory."),
        SaveKind::Unsupported(name) => bail!("{name} save memory isn't supported."),
        SaveKind::DmgRam(_) => {
            dmg_prepare_ram(link, info.mbc)?;
            let banks = size.div_ceil(0x2000);
            let mut buf = vec![0u8; 0x200];
            for bank in 0..banks {
                if info.mbc != Mbc::Mbc2 {
                    link.dmg_write(0x4000, bank as u8)?;
                }
                let bank_size = 0x2000.min(size - bank * 0x2000);
                for offset in (0..bank_size).step_by(0x200) {
                    let part = &mut buf[..0x200.min(bank_size - offset)];
                    link.dmg_read_ram(offset as u16, part)?;
                    if info.mbc == Mbc::Mbc2 {
                        part.iter_mut().for_each(|b| *b &= 0x0F);
                    }
                    sink(part)?;
                }
                progress((bank + 1) as f32 / banks as f32);
            }
            dmg_enable_ram(link, info.mbc, false)?;
        }
        SaveKind::AgbSram(_) => {
            let mut buf = vec![0u8; 0x2000];
            for offset in (0..size).step_by(0x2000) {
                link.agb_read_sram(offset as u32, &mut buf)?;
                sink(&buf)?;
                progress((offset + 0x2000) as f32 / size as f32);
            }
        }
        SaveKind::AgbFlash(_) => {
            let chip = agb_flash_chip_id(link)?;
            if matches!(chip, 0xBF4B | 0xBF5B | 0xFFFF) && size > 0x10000 {
                bail!("This cartridge's Flash save chip ({chip:#06x}) isn't supported.");
            }
            let mut buf = vec![0u8; 0x1000];
            for bank in 0..size.div_ceil(0x10000) {
                if size > 0x10000 {
                    agb_flash_select_bank(link, bank as u32)?;
                }
                for offset in (0..0x10000.min(size)).step_by(0x1000) {
                    link.agb_read_sram(offset as u32, &mut buf)?;
                    sink(&buf)?;
                    progress((bank * 0x10000 + offset + 0x1000) as f32 / size as f32);
                }
            }
            if size > 0x10000 {
                agb_flash_select_bank(link, 0)?;
            }
        }
        SaveKind::AgbEeprom(eeprom) => {
            let mut buf = vec![0u8; 0x100];
            for offset in (0..size).step_by(0x100) {
                link.agb_read_eeprom(eeprom, offset as u32, &mut buf)?;
                sink(&buf)?;
                progress((offset + 0x100) as f32 / size as f32);
            }
        }
        SaveKind::Mbc7Eeprom(_) => {
            let mut buf = vec![0u8; size];
            let mut eeprom = Mbc7Eeprom::open(link)?;
            let result = eeprom.read(&mut buf, progress);
            eeprom.close()?;
            result?;
            sink(&buf)?;
        }
    }
    Ok(())
}

// ---- MBC7 EEPROM ----

/// The MBC7's EEPROM pins, as bits of the register at 0xA080: chip select, clock, data in, and
/// data out (read only).
const MBC7_EEPROM_REGISTER: u16 = 0xA080;
const MBC7_CS: u8 = 0x80;
const MBC7_CLK: u8 = 0x40;
const MBC7_DI: u8 = 0x02;

/// The serial EEPROM in MBC7 cartridges (93LC56: 16-bit words, most significant bit first).
/// Save files hold the words high byte first, like the Game Bub's MBC7 core.
struct Mbc7Eeprom<'a> {
    link: &'a mut Link,
}

impl<'a> Mbc7Eeprom<'a> {
    fn open(link: &'a mut Link) -> anyhow::Result<Self> {
        link.dmg_mbc_reset()?;
        // Both RAM enables are needed to reach the EEPROM register.
        link.dmg_write(0x0000, 0x0A)?;
        link.dmg_write(0x4000, 0x40)?;
        link.dmg_set_cs_pulses(false, true)?;
        let mut eeprom = Mbc7Eeprom { link };
        eeprom.set(0)?;
        Ok(eeprom)
    }

    fn close(mut self) -> anyhow::Result<()> {
        self.set(0)?;
        self.link.dmg_write(0x4000, 0x00)?;
        self.link.dmg_write(0x0000, 0x00)?;
        self.link.dmg_set_cs_pulses(false, false)?;
        Ok(())
    }

    fn set(&mut self, pins: u8) -> anyhow::Result<()> {
        Ok(self.link.dmg_write(MBC7_EEPROM_REGISTER, pins)?)
    }

    fn data_out(&mut self) -> anyhow::Result<bool> {
        let mut value = [0u8];
        self.link.dmg_read_ram(MBC7_EEPROM_REGISTER - 0xA000, &mut value)?;
        Ok(value[0] & 1 != 0)
    }

    /// Clock one bit in (the EEPROM reads data in on the rising clock edge).
    fn send_bit(&mut self, bit: bool) -> anyhow::Result<()> {
        let di = if bit { MBC7_DI } else { 0 };
        self.set(MBC7_CS | di)?;
        self.set(MBC7_CS | MBC7_CLK | di)
    }

    /// Select the chip and send a start bit, a 2-bit opcode and an 8-bit word address.
    fn command(&mut self, opcode: u8, address: u8) -> anyhow::Result<()> {
        self.set(0)?;
        self.set(MBC7_CS)?;
        self.send_bit(true)?;
        for i in (0..2).rev() {
            self.send_bit(opcode >> i & 1 != 0)?;
        }
        for i in (0..8).rev() {
            self.send_bit(address >> i & 1 != 0)?;
        }
        Ok(())
    }

    /// Read the whole EEPROM in one sequential read.
    fn read(&mut self, out: &mut [u8], progress: Progress) -> anyhow::Result<()> {
        self.command(0b10, 0)?;
        // The EEPROM outputs a 0 before the data; a 1 means nothing answered.
        if self.data_out()? {
            bail!("The cartridge's save chip didn't answer.\nClean the contacts and try again.");
        }
        let len = out.len();
        for (i, byte) in out.iter_mut().enumerate() {
            let mut value = 0u8;
            for _ in 0..8 {
                self.set(MBC7_CS)?;
                self.set(MBC7_CS | MBC7_CLK)?;
                value = value << 1 | self.data_out()? as u8;
            }
            *byte = value;
            progress((i + 1) as f32 / len as f32);
        }
        self.set(0)
    }

    /// Allow or prevent writes.
    fn write_enable(&mut self, enable: bool) -> anyhow::Result<()> {
        self.command(0b00, if enable { 0xC0 } else { 0x00 })?;
        self.set(0)
    }

    /// Write one word, then wait for the EEPROM to finish (it signals ready on data out).
    fn write_word(&mut self, address: u8, word: u16) -> anyhow::Result<()> {
        self.command(0b01, address)?;
        for i in (0..16).rev() {
            self.send_bit(word >> i & 1 != 0)?;
        }
        // Lowering chip select starts the write; selecting it again shows the busy status.
        self.set(0)?;
        self.set(MBC7_CS)?;
        let start = Instant::now();
        while !self.data_out()? {
            if start.elapsed() > Duration::from_millis(100) {
                self.set(0)?;
                bail!("The cartridge's save chip didn't finish a write.");
            }
        }
        self.set(0)
    }

    /// Write the whole EEPROM.
    fn write(&mut self, data: &[u8], progress: Progress) -> anyhow::Result<()> {
        self.write_enable(true)?;
        let words = data.len() / 2;
        let result = (|| {
            for (i, pair) in data.chunks_exact(2).enumerate() {
                self.write_word(i as u8, u16::from_be_bytes([pair[0], pair[1]]))?;
                progress((i + 1) as f32 / words as f32);
            }
            Ok(())
        })();
        self.write_enable(false)?;
        result
    }
}

/// Save data read from a file in chunks. Files from other tools may be a different size than
/// the cartridge's save memory: they're padded (or cut) to fit.
pub struct SaveSource {
    file: File,
    fill: u8,
    mask_mbc2: bool,
}

impl SaveSource {
    pub fn open(path: &Path, save: SaveKind, mbc: Mbc) -> anyhow::Result<Self> {
        let file = File::open(path).with_context(|| format!("Couldn't open {}", path.display()))?;
        Ok(SaveSource {
            file,
            fill: if matches!(save, SaveKind::AgbFlash(_)) { 0xFF } else { 0x00 },
            mask_mbc2: mbc == Mbc::Mbc2,
        })
    }

    /// Fill `buf` with the next chunk of save data.
    fn next(&mut self, buf: &mut [u8]) -> anyhow::Result<()> {
        use std::io::Read;
        let mut filled = 0;
        while filled < buf.len() {
            let n = self.file.read(&mut buf[filled..]).context("Couldn't read the save file")?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        buf[filled..].fill(self.fill);
        if self.mask_mbc2 {
            buf.iter_mut().for_each(|b| *b &= 0x0F);
        }
        Ok(())
    }
}

/// Write save data from a file to the cartridge, then read it back to verify it.
pub fn write_save(
    link: &mut Link,
    info: &CartInfo,
    save: SaveKind,
    path: &Path,
    progress: Progress,
) -> anyhow::Result<()> {
    let size = save.size() as usize;
    let mut source = SaveSource::open(path, save, info.mbc)?;

    // Writing takes about half the time, verifying the other half.
    let mut write_progress = |f: f32| progress(f * 0.5);
    match save {
        SaveKind::None => bail!("This cartridge has no save memory."),
        SaveKind::Unsupported(name) => bail!("{name} save memory isn't supported."),
        SaveKind::DmgRam(_) => {
            dmg_prepare_ram(link, info.mbc)?;
            let banks = size.div_ceil(0x2000);
            let mut buf = vec![0u8; 0x200];
            for bank in 0..banks {
                if info.mbc != Mbc::Mbc2 {
                    link.dmg_write(0x4000, bank as u8)?;
                }
                let bank_size = 0x2000.min(size - bank * 0x2000);
                for offset in (0..bank_size).step_by(0x200) {
                    let part = &mut buf[..0x200.min(bank_size - offset)];
                    source.next(part)?;
                    link.dmg_write_ram(offset as u16, part)?;
                }
                write_progress((bank + 1) as f32 / banks as f32);
            }
            dmg_enable_ram(link, info.mbc, false)?;
        }
        SaveKind::AgbSram(_) => {
            let mut buf = vec![0u8; 0x2000];
            for offset in (0..size).step_by(0x2000) {
                source.next(&mut buf)?;
                link.agb_write_sram(offset as u32, &buf)?;
                write_progress((offset + 0x2000) as f32 / size as f32);
            }
        }
        SaveKind::AgbFlash(_) => {
            let chip = agb_flash_chip_id(link)?;
            if matches!(chip, 0x1F3D | 0xBF4B | 0xBF5B | 0xFFFF) {
                bail!("Restoring to this cartridge's Flash save chip ({chip:#06x}) isn't supported.");
            }
            link.agb_flash_status_setup()?;
            let mut buf = vec![0u8; 0x1000];
            for bank in 0..size.div_ceil(0x10000) {
                if size > 0x10000 {
                    agb_flash_select_bank(link, bank as u32)?;
                }
                for offset in (0..0x10000.min(size)).step_by(0x1000) {
                    let sector = offset as u32;
                    source.next(&mut buf)?;
                    link.agb_flash_commands(&[
                        (0x5555, 0xAA),
                        (0x2AAA, 0x55),
                        (0x5555, 0x80),
                        (0x5555, 0xAA),
                        (0x2AAA, 0x55),
                        (sector, 0x30),
                    ])?;
                    let mut erased = false;
                    for _ in 0..50 {
                        std::thread::sleep(Duration::from_millis(10));
                        let mut status = [0u8; 2];
                        link.agb_read_sram(sector, &mut status)?;
                        if status == [0xFF, 0xFF] {
                            erased = true;
                            break;
                        }
                    }
                    if !erased {
                        bail!("A Flash save sector didn't erase.");
                    }
                    if buf.iter().any(|&b| b != 0xFF) {
                        link.agb_write_flash(sector, &buf)?;
                    }
                    write_progress((bank * 0x10000 + offset + 0x1000) as f32 / size as f32);
                }
            }
            if size > 0x10000 {
                agb_flash_select_bank(link, 0)?;
            }
        }
        SaveKind::Mbc7Eeprom(_) => {
            let mut buf = vec![0u8; size];
            source.next(&mut buf)?;
            let mut eeprom = Mbc7Eeprom::open(link)?;
            let result = eeprom.write(&buf, &mut write_progress);
            eeprom.close()?;
            result?;
        }
        SaveKind::AgbEeprom(eeprom) => {
            let mut buf = vec![0u8; 0x40];
            for offset in (0..size).step_by(0x40) {
                source.next(&mut buf)?;
                link.agb_write_eeprom(eeprom, offset as u32, &buf)?;
                write_progress((offset + 0x40) as f32 / size as f32);
            }
        }
    }

    // Verify: read the save back and compare it with the file.
    let mut source = SaveSource::open(path, save, info.mbc)?;
    let mut expected = vec![];
    read_save(link, info, save, &mut |f| progress(0.5 + f * 0.5), &mut |data| {
        expected.resize(data.len(), 0);
        source.next(&mut expected)?;
        if data != expected.as_slice() {
            bail!("The save didn't verify after writing.\nTry again, or copy the save to the SD card first to see what's on the cartridge.");
        }
        Ok(())
    })
}

// ---- Save files ----

/// Read the cartridge save into a save file on the SD card, replacing it (the previous one is
/// kept as .bak).
pub fn copy_save_to_sd(link: &mut Link, info: &CartInfo, path: &Path, progress: Progress) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).context("Couldn't create the save folder")?;
    }
    let temp = crate::core::temp_path(path);
    let mut file = File::create(&temp).with_context(|| format!("Couldn't create {}", temp.display()))?;
    let result = read_save(link, info, info.save, progress, &mut |data| {
        file.write_all(data).context("Couldn't write the save file")
    })
    .and_then(|_| file.sync_all().context("Couldn't write the save file"));
    drop(file);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    crate::core::replace_file(&temp, path).context("Couldn't save the save file")
}

/// Whether the cartridge's save memory is empty (all 0xFF or all 0x00).
pub fn save_is_blank(link: &mut Link, info: &CartInfo, progress: Progress) -> anyhow::Result<bool> {
    let (mut all_ff, mut all_00) = (true, true);
    read_save(link, info, info.save, progress, &mut |data| {
        all_ff &= data.iter().all(|&b| b == 0xFF);
        all_00 &= data.iter().all(|&b| b == 0x00);
        Ok(())
    })?;
    Ok(all_ff || all_00)
}
