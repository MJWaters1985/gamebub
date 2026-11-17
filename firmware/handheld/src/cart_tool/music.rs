//! Cartridge music from the game's own ROM, with nothing to install: built-in data
//! (scripts/gbs/music_db.py) says how to rebuild the game's GBS rip from its ROM, read from
//! the cartridge or from a ROM file. The rebuilt rip is written to the SD card as a ROM image
//! and run in the Game Boy core with the music boot ROM (scripts/gbs/gbs2rom.py), which plays
//! it: Left/Right choose the song, A restarts it.
//!
//! A newer table can be put at /system/cart-music/db.bin; it's looked at before the built-in
//! one.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::cart::{self, CartInfo};
use super::db::Crc32;
use super::link::Link;

static BUILTIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cart-music.db"));
static BOOT: &[u8] = include_bytes!("../../res/cart-music-boot.bin");

const SD_TABLE: &str = "/sdcard/system/cart-music/db.bin";
const IMAGE: &str = "/sdcard/system/cart-music/music.gbc";
const BOOT_PATH: &str = "/sdcard/system/cart-music/music-boot.bin";

/// Where the parameters go in the boot ROM (gbs2rom.py PARAMS).
const PARAMS: usize = 0x800;
const TIMER_VECTOR: usize = 0x50;
const PLAYER: u16 = 0x8000;
/// MBC3 + RAM, 32 KB of RAM: the mapper the image runs with (it has no header).
const MAPPER: u8 = 0x12;
const RAM_SIZE: u8 = 0x03;

/// How to rebuild one game's rip.
pub struct Entry {
    load: u16,
    init: u16,
    play: u16,
    sp: u16,
    songs: u8,
    first: u8,
    tma: u8,
    tac: u8,
    rip_len: u32,
    rip_crc: u32,
    ops: Ops,
}

/// Where the entry's rebuild steps are read from.
enum Ops {
    Builtin(&'static [u8]),
    File { path: PathBuf, offset: u64, len: u32 },
}

/// A ROM image ready to run.
pub struct Music {
    pub boot: PathBuf,
    pub image: PathBuf,
    /// Cartridge type, ROM size and RAM size codes for the image (as in a ROM header).
    pub mapper: [u8; 3],
}

const ENTRY_SIZE: usize = 28;

fn u16_(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

fn u32_(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// The key for a ROM header (from address 0): CRC-32 of $0134-$014F.
fn header_key(header: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(&header[0x134..0x150]);
    crc.finish()
}

fn parse_entry(b: &[u8]) -> (Entry, u32) {
    let entry = Entry {
        load: u16_(&b[0..]),
        init: u16_(&b[2..]),
        play: u16_(&b[4..]),
        sp: u16_(&b[6..]),
        songs: b[8],
        first: b[9],
        tma: b[10],
        tac: b[11],
        rip_len: u32_(&b[12..]),
        rip_crc: u32_(&b[16..]),
        ops: Ops::Builtin(&[]),
    };
    // b[20..24]: the ROM's size
    (entry, u32_(&b[24..]))
}

/// Look the key up in a table's index: the entry's offset.
fn search(count: usize, key: u32, mut index_at: impl FnMut(usize) -> Result<(u32, u32)>) -> Result<Option<u32>> {
    let (mut lo, mut hi) = (0, count);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let (k, offset) = index_at(mid)?;
        if k == key {
            return Ok(Some(offset));
        }
        if k < key {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(None)
}

fn check_head(head: &[u8]) -> Option<usize> {
    (head.len() >= 8 && &head[0..4] == b"CMDB" && u16_(&head[4..]) == 1).then(|| u16_(&head[6..]) as usize)
}

fn find_builtin(key: u32) -> Option<Entry> {
    let data = BUILTIN;
    let count = check_head(data)?;
    let offset = search(count, key, |i| {
        let at = 8 + i * 8;
        Ok((u32_(&data[at..]), u32_(&data[at + 4..])))
    })
    .ok()??;
    let at = offset as usize;
    let (mut entry, len) = parse_entry(data.get(at..at + ENTRY_SIZE)?);
    entry.ops = Ops::Builtin(data.get(at + ENTRY_SIZE..at + ENTRY_SIZE + len as usize)?);
    Some(entry)
}

fn find_in_file(path: &Path, key: u32) -> Result<Option<Entry>> {
    let mut file = File::open(path)?;
    let mut head = [0u8; 8];
    file.read_exact(&mut head)?;
    let Some(count) = check_head(&head) else {
        bail!("{} isn't a cartridge music table", path.display());
    };
    let found = search(count, key, |i| {
        let mut b = [0u8; 8];
        file.seek(SeekFrom::Start(8 + i as u64 * 8))?;
        file.read_exact(&mut b)?;
        Ok((u32_(&b), u32_(&b[4..])))
    })?;
    let Some(offset) = found else {
        return Ok(None);
    };
    let mut b = [0u8; ENTRY_SIZE];
    file.seek(SeekFrom::Start(offset as u64))?;
    file.read_exact(&mut b)?;
    let (mut entry, len) = parse_entry(&b);
    entry.ops = Ops::File { path: path.to_owned(), offset: offset as u64 + ENTRY_SIZE as u64, len };
    Ok(Some(entry))
}

/// The music data for a ROM, from its header: the SD card's table first, then the built-in one.
pub fn find(header: &[u8]) -> Option<Entry> {
    if header.len() < 0x150 {
        return None;
    }
    let key = header_key(header);
    if Path::new(SD_TABLE).is_file() {
        match find_in_file(Path::new(SD_TABLE), key) {
            Ok(Some(entry)) => return Some(entry),
            Ok(None) => {}
            Err(e) => log::warn!("Cartridge music table on the SD card: {e:#}"),
        }
    }
    find_builtin(key)
}

/// Where the ROM's bytes come from.
pub enum Rom<'a> {
    Cart(&'a mut Link, &'a CartInfo),
    File(File),
}

impl Rom<'_> {
    fn read(&mut self, offset: u32, out: &mut [u8]) -> Result<()> {
        match self {
            Rom::File(file) => {
                file.seek(SeekFrom::Start(offset as u64))?;
                file.read_exact(out).context("The ROM file is shorter than the game")?;
            }
            Rom::Cart(link, info) => {
                let mut done = 0;
                while done < out.len() {
                    let at = offset + done as u32;
                    let bank = at / 0x4000;
                    let off = (at % 0x4000) as usize;
                    let n = (out.len() - done).min(0x4000 - off);
                    cart::read_rom_bank(link, info, bank, off as u16, &mut out[done..done + n])?;
                    done += n;
                }
            }
        }
        Ok(())
    }
}

fn varint(r: &mut impl Read) -> Result<Option<u32>> {
    let mut v = 0u32;
    let mut shift = 0;
    loop {
        let mut b = [0u8];
        if r.read(&mut b)? == 0 {
            if shift == 0 {
                return Ok(None);
            }
            bail!("The music data is cut short");
        }
        v |= ((b[0] & 0x7F) as u32) << shift;
        if b[0] & 0x80 == 0 {
            return Ok(Some(v));
        }
        shift += 7;
        if shift > 28 {
            bail!("The music data is damaged");
        }
    }
}

/// Bank 0 below the load address: rst vectors to load+n (as GBS rips expect), the timer
/// vector to the player and `reti` at the other interrupt vectors.
fn bank0(load: u16) -> Vec<u8> {
    let load = load as usize;
    let mut out = vec![0u8; load];
    for n in (0..0x40).step_by(8) {
        let t = (load + n) as u16;
        out[n..n + 3].copy_from_slice(&[0xC3, t as u8, (t >> 8) as u8]);
    }
    for v in (0x40..load.min(0x68)).step_by(8) {
        out[v] = 0xD9;
    }
    out[TIMER_VECTOR..TIMER_VECTOR + 3].copy_from_slice(&[0xC3, PLAYER as u8, (PLAYER >> 8) as u8]);
    out
}

/// Rebuild the rip from the ROM and write the image and boot ROM to the SD card.
pub fn build(entry: &Entry, rom: &mut Rom, progress: &dyn Fn(f32)) -> Result<Music> {
    if (entry.load as usize) < TIMER_VECTOR + 3 {
        bail!("This game's music can't be played this way");
    }
    let end = entry.load as u32 + entry.rip_len;
    let mut size = 0x8000u32;
    while size < end {
        size *= 2;
    }
    let size_code = (size / 0x8000).trailing_zeros() as u8;

    let dir = Path::new(IMAGE).parent().unwrap();
    std::fs::create_dir_all(dir).context("Couldn't create /system/cart-music")?;
    let file = File::create(IMAGE).context("Couldn't create the music image")?;
    let mut out = BufWriter::with_capacity(2048, file);
    out.write_all(&bank0(entry.load))?;

    let mut ops: Box<dyn Read> = match &entry.ops {
        Ops::Builtin(data) => Box::new(*data),
        Ops::File { path, offset, len } => {
            let mut file = File::open(path)?;
            file.seek(SeekFrom::Start(*offset))?;
            Box::new(BufReader::with_capacity(512, file.take(*len as u64)))
        }
    };
    let mut crc = Crc32::new();
    let mut written = 0u32;
    let mut buf = [0u8; 512];
    let mut last = 0.0;
    while let Some(tag) = varint(&mut ops)? {
        let mut n = tag >> 1;
        let mut from = if tag & 1 == 1 {
            Some(varint(&mut ops)?.context("The music data is cut short")?)
        } else {
            None
        };
        if written + n > entry.rip_len {
            bail!("The music data is damaged");
        }
        while n > 0 {
            let k = n.min(buf.len() as u32) as usize;
            match from.as_mut() {
                Some(at) => {
                    rom.read(*at, &mut buf[..k])?;
                    *at += k as u32;
                }
                None => ops.read_exact(&mut buf[..k]).context("The music data is cut short")?,
            }
            crc.update(&buf[..k]);
            out.write_all(&buf[..k]).context("Couldn't write the music image")?;
            written += k as u32;
            n -= k as u32;
            let p = written as f32 / entry.rip_len as f32;
            if p - last >= 0.02 {
                progress(p);
                last = p;
            }
        }
    }
    if written != entry.rip_len || crc.finish() != entry.rip_crc {
        bail!("This version of the game isn't the one the music data was made from");
    }
    let zeros = [0u8; 512];
    let mut at = end;
    while at < size {
        let k = (size - at).min(zeros.len() as u32);
        out.write_all(&zeros[..k as usize])?;
        at += k;
    }
    out.into_inner().map_err(|e| e.into_error()).and_then(|f| f.sync_all())
        .context("Couldn't write the music image")?;

    let mut boot = BOOT.to_vec();
    let flags = (entry.tac & 0x80 != 0) as u8 | (((entry.tac & 4 != 0) as u8) << 1);
    let p = &mut boot[PARAMS..PARAMS + 11];
    p[0..2].copy_from_slice(&entry.init.to_le_bytes());
    p[2..4].copy_from_slice(&entry.play.to_le_bytes());
    p[4..6].copy_from_slice(&entry.sp.to_le_bytes());
    p[6] = entry.songs.max(1);
    p[7] = entry.first.saturating_sub(1).min(entry.songs.max(1) - 1);
    p[8] = flags;
    p[9] = entry.tma;
    p[10] = entry.tac;
    std::fs::write(BOOT_PATH, &boot).context("Couldn't write the music boot ROM")?;

    Ok(Music { boot: BOOT_PATH.into(), image: IMAGE.into(), mapper: [MAPPER, size_code, RAM_SIZE] })
}

/// Build the music for the inserted cartridge, from its ROM file on the SD card if there is
/// one (faster), else from the cartridge. None if there's no music data for it.
pub fn for_cartridge(link: &mut Link, info: &CartInfo, progress: &dyn Fn(f32)) -> Result<Option<Music>> {
    let Some(entry) = find(info.header()) else {
        return Ok(None);
    };
    if let Some(path) = info.find_rom_on_sd() {
        if let Ok(file) = File::open(&path) {
            log::info!("Cartridge music: rebuilding from {}", path.display());
            return build(&entry, &mut Rom::File(file), progress).map(Some);
        }
    }
    log::info!("Cartridge music: rebuilding from the cartridge");
    build(&entry, &mut Rom::Cart(link, info), progress).map(Some)
}

/// Build the music for a ROM file. None if there's no music data for it.
pub fn for_rom_file(path: &Path, progress: &dyn Fn(f32)) -> Result<Option<Music>> {
    let mut file = File::open(path).context("Couldn't open the ROM")?;
    let mut header = [0u8; 0x150];
    file.read_exact(&mut header).context("Couldn't read the ROM")?;
    let Some(entry) = find(&header) else {
        return Ok(None);
    };
    build(&entry, &mut Rom::File(file), progress).map(Some)
}

/// Whether there's music data for a ROM header (for showing the option).
pub fn has_music(header: &[u8]) -> bool {
    find(header).is_some()
}

/// Run a built image in the Game Boy core.
pub fn run(music: Music) {
    *crate::bitstream::gameboy::CART_MUSIC_PLAYER.lock().unwrap() = Some(music.boot);
    *crate::bitstream::gameboy::CART_MUSIC_MAPPER.lock().unwrap() = Some(music.mapper);
    crate::worker::send(crate::worker::Message::RunRom {
        core_id: "Game-Bub.GB".to_string(),
        path: music.image,
    });
}
