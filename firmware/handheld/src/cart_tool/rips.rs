//! Cartridge music from GBS rips: packages in /system/cart-music/rips (made by
//! scripts/gbs/package.py) hold a music player and what the rip's sound engine looks like. The
//! inserted cartridge is checked against it: each rip bank is found among the cartridge's banks
//! by a short probe, the code and data the rip uses is checked by CRC, and the player's
//! rip-bank -> ROM-bank table is filled in. A cartridge that doesn't match (another revision or
//! region) is refused.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use super::cart::{self, CartInfo, Mbc};
use super::db::Crc32;
use super::link::Link;

const PLAYER_SIZE: usize = 0x900;

struct Package {
    table_at: usize,
    table_size: usize,
    constants: Vec<usize>,
    /// (rip bank, offset, bytes)
    probes: Vec<(u8, u16, Vec<u8>)>,
    /// (rip bank, offset, length, CRC32)
    checks: Vec<(u8, u16, u16, u32)>,
    /// Bank numbers in the engine's data: (rip bank, offset, the rip's value). On the cartridge
    /// each is the same, or (renumbered by the ripper) the ROM bank that rip bank was found in.
    bank_bytes: Vec<(u8, u16, u8)>,
    /// Bytes in the checked ranges that may be bank numbers the ripper renumbered, sorted:
    /// (rip bank, offset, the rip's value). See `crc`.
    marks: Vec<(u8, u16, u8)>,
    /// Calls the ripper redirected to its copy of a bank 0 routine: (rip bank, offset of the
    /// address in the instruction, length, CRC32 of the ripper's copy). The cartridge's routine at
    /// the address it has there (through up to three jp) must have that CRC.
    redirects: Vec<(u8, u16, u16, u32)>,
    player: Vec<u8>,
}

fn parse(data: &[u8]) -> Result<Package> {
    let mut p = 0usize;
    let mut take = |n: usize| -> Result<&[u8]> {
        let s = data.get(p..p + n).context("package is cut short")?;
        p += n;
        Ok(s)
    };
    let version = match take(4)? {
        b"CMR1" => 1,
        b"CMR2" => 2,
        b"CMR3" => 3,
        b"CMR4" => 4,
        _ => bail!("not a cartridge music package"),
    };
    let u16_ = |b: &[u8]| u16::from_le_bytes([b[0], b[1]]);
    let h = take(3)?;
    let (table_at, table_size) = (u16_(h) as usize, h[2] as usize);
    let n = take(1)?[0] as usize;
    let constants: Vec<usize> = take(2 * n)?.chunks(2).map(|c| u16_(c) as usize).collect();
    let n = take(1)?[0] as usize;
    let mut probes = vec![];
    for _ in 0..n {
        let h = take(4)?;
        let (bank, off, len) = (h[0], u16_(&h[1..]), h[3] as usize);
        probes.push((bank, off, take(len)?.to_vec()));
    }
    let n = u16_(take(2)?) as usize;
    let mut checks = vec![];
    for _ in 0..n {
        let c = take(9)?;
        checks.push((c[0], u16_(&c[1..]), u16_(&c[3..]), u32::from_le_bytes([c[5], c[6], c[7], c[8]])));
    }
    let mut bank_bytes = vec![];
    if version >= 2 {
        let n = u16_(take(2)?) as usize;
        for _ in 0..n {
            let c = take(4)?;
            bank_bytes.push((c[0], u16_(&c[1..]), c[3]));
        }
    }
    let mut marks = vec![];
    if version >= 3 {
        let n = u16_(take(2)?) as usize;
        for _ in 0..n {
            let c = take(4)?;
            marks.push((c[0], u16_(&c[1..]), c[3]));
        }
        marks.sort_unstable();
    }
    let mut redirects = vec![];
    if version >= 4 {
        let n = u16_(take(2)?) as usize;
        for _ in 0..n {
            let c = take(9)?;
            redirects.push((c[0], u16_(&c[1..]), u16_(&c[3..]), u32::from_le_bytes([c[5], c[6], c[7], c[8]])));
        }
    }
    let _flags = take(1)?[0];
    let player = take(PLAYER_SIZE)?.to_vec();
    if table_at + table_size > PLAYER_SIZE || constants.iter().any(|&o: &usize| o >= PLAYER_SIZE) {
        bail!("package offsets out of range");
    }
    Ok(Package { table_at, table_size, constants, probes, checks, bank_bytes, marks, redirects, player })
}

/// Lower case, without bracketed parts, "The", a possessive "s", "version" and punctuation, the
/// same as scripts/gbs/gbsdiff.py's norm (package index names).
pub fn norm(name: &str) -> String {
    let mut s = String::new();
    let mut depth = 0;
    for c in name.to_lowercase().replace("legend of zelda, the", "zelda").replace('&', " and ").chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            c if c.is_ascii_alphanumeric() => s.push(c),
            _ => s.push(' '),
        }
    }
    const DROP: [&str; 6] = ["version", "the", "s", "special", "pikachu", "edition"];
    s.split_whitespace().filter(|w| !DROP.contains(w)).collect::<Vec<_>>().join(" ")
}

/// CRC of a range of the cartridge (ROM `bank`) that rip bank `rip_bank` should have. Where a
/// marked byte (a possible renumbered bank number) differs from the rip's, the rip's value is
/// used and (cartridge value, rip value) noted in `renumbered`, to check against the table.
fn crc(
    link: &mut Link,
    info: &CartInfo,
    pkg: &Package,
    (rip_bank, bank): (u8, u32),
    offset: u16,
    len: u16,
    renumbered: &mut Vec<(u8, u8)>,
) -> Result<u32> {
    // In pieces: a check can cover a whole 16 KB bank, and memory is short.
    let mut buf = [0u8; 512];
    let mut c = Crc32::new();
    let mut done = 0u16;
    let mut mark = pkg.marks.partition_point(|m| (m.0, m.1) < (rip_bank, offset));
    while done < len {
        let n = (len - done).min(buf.len() as u16);
        let at = offset + done;
        cart::read_rom_bank(link, info, bank, at, &mut buf[..n as usize])?;
        while let Some(&(b, o, v)) = pkg.marks.get(mark) {
            if b != rip_bank || o >= at + n {
                break;
            }
            let i = (o - at) as usize;
            if buf[i] != v {
                renumbered.push((buf[i], v));
                buf[i] = v;
            }
            mark += 1;
        }
        c.update(&buf[..n as usize]);
        done += n;
    }
    Ok(c.finish())
}

/// Check a package against the cartridge; the player file with the bank table filled in.
fn try_package(link: &mut Link, info: &CartInfo, pkg: &Package) -> Result<std::result::Result<Vec<u8>, String>> {
    let banks = (info.rom_size / 0x4000).max(2);
    let mut table: Vec<u8> = (0..pkg.table_size as u32).map(|b| b as u8).collect();
    let mut renumbered = vec![];
    for (rip_bank, off, probe) in &pkg.probes {
        let mut found = None;
        let mut buf = vec![0u8; probe.len()];
        for bank in 1..banks {
            cart::read_rom_bank(link, info, bank, *off, &mut buf)?;
            if buf != *probe {
                continue;
            }
            let mut all = true;
            let mut got = vec![];
            for &(_, o, len, want) in pkg.checks.iter().filter(|c| c.0 == *rip_bank) {
                if crc(link, info, pkg, (*rip_bank, bank), o, len, &mut got)? != want {
                    all = false;
                    break;
                }
            }
            if all {
                found = Some(bank);
                renumbered.extend(got);
                break;
            }
        }
        let Some(bank) = found else {
            return Ok(Err(format!("its sound engine (rip bank {rip_bank}) isn't on this cartridge")));
        };
        if info.mbc == Mbc::Mbc1 && bank >= 0x20 || bank > 0xFF {
            return Ok(Err(format!("it needs ROM bank {bank}, which the player can't select")));
        }
        if let Some(t) = table.get_mut(*rip_bank as usize) {
            *t = bank as u8;
        }
    }
    for &(_, o, len, want) in pkg.checks.iter().filter(|c| c.0 == 0) {
        if crc(link, info, pkg, (0, 0), o, len, &mut renumbered)? != want {
            return Ok(Err("the cartridge's code differs from the rip's".into()));
        }
    }
    for &(rip_bank, off, len, want) in &pkg.redirects {
        let bank = if rip_bank == 0 { 0 } else { table.get(rip_bank as usize).copied().unwrap_or(rip_bank) };
        let mut a = [0u8; 2];
        cart::read_rom_bank(link, info, bank as u32, off, &mut a)?;
        let mut x = u16::from_le_bytes(a);
        let mut ok = false;
        for hop in 0..4 {
            if x as u32 + len as u32 > 0x4000 {
                break;
            }
            if crc(link, info, pkg, (0xFF, 0), x, len, &mut vec![])? == want {
                ok = true;
                break;
            }
            // through a jump table: jp nn
            let mut j = [0u8; 3];
            cart::read_rom_bank(link, info, 0, x, &mut j)?;
            if hop == 3 || j[0] != 0xC3 {
                break;
            }
            x = u16::from_le_bytes([j[1], j[2]]);
        }
        if !ok {
            return Ok(Err("a routine its sound engine calls differs".into()));
        }
    }
    // possible bank numbers that differ must be the ROM bank of the rip bank they name
    if renumbered.iter().any(|&(cart, rip)| table.get(rip as usize) != Some(&cart)) {
        return Ok(Err("the bank numbers in its sound engine don't match".into()));
    }
    for &(rip_bank, off, value) in &pkg.bank_bytes {
        let bank = if rip_bank == 0 { 0 } else { table.get(rip_bank as usize).copied().unwrap_or(rip_bank) };
        let mut b = [0u8];
        cart::read_rom_bank(link, info, bank as u32, off, &mut b)?;
        // left alone by the ripper, or renumbered to the rip bank
        if b[0] != value && table.get(value as usize) != Some(&b[0]) {
            return Ok(Err("the bank numbers in its sound engine's data don't match".into()));
        }
    }
    let mut player = pkg.player.clone();
    player[pkg.table_at..pkg.table_at + table.len()].copy_from_slice(&table);
    for &o in &pkg.constants {
        let v = player[o] as usize;
        if let Some(&t) = table.get(v) {
            player[o] = t;
        }
    }
    Ok(Ok(player))
}

/// Packages to try for this cartridge, from rips/index.txt ("<name>\t<file>" lines, names as
/// made by norm()).
/// Rip packages and their index. Not get_system_file_path, which only looks on the SD card for
/// files that already exist and otherwise gives the (read-only) built-in /system.
const RIPS_DIR: &str = "/sdcard/system/cart-music/rips";
/// The player made from the package that matched, for the Game Boy core to load.
const RIP_PLAYER: &str = "/sdcard/system/cart-music/rip-player.bin";

fn candidates(info: &CartInfo) -> Result<Vec<PathBuf>> {
    let dir = std::path::Path::new(RIPS_DIR);
    // Read a line at a time: the index is too big to load in one piece.
    let index = match std::fs::File::open(dir.join("index.txt")) {
        Ok(index) => index,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && dir.parent().is_some_and(|d| d.is_dir()) => {
            return Ok(vec![]); // no rip packages installed
        }
        Err(e) => bail!("Couldn't read the music index on the SD card ({e}). Is the SD card in?"),
    };
    let mut names = vec![];
    if let Some(db) = &info.db {
        names.push(norm(&db.name));
    }
    names.push(norm(&info.header_title));
    // Matches for each name, so the database name's packages are tried first.
    let mut found: Vec<Vec<PathBuf>> = vec![vec![]; names.len()];
    let reader = std::io::BufReader::with_capacity(1024, index);
    for line in std::io::BufRead::lines(reader) {
        let line = line.context("Couldn't read the music index on the SD card")?;
        if let Some((n, file)) = line.split_once('\t') {
            for (name, list) in names.iter().zip(found.iter_mut()) {
                if !name.is_empty() && n == name {
                    list.push(dir.join(file));
                }
            }
        }
    }
    let mut out: Vec<PathBuf> = vec![];
    for p in found.into_iter().flatten() {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    log::info!("Cartridge Music: looked for {names:?}, {} package(s)", out.len());
    Ok(out)
}

/// Find a rip package that matches the cartridge and write its player to
/// /system/cart-music/rip-player.bin. Ok(None) if there's no package for it.
pub fn find(link: &mut Link, info: &CartInfo, busy: &dyn Fn(&str)) -> Result<Option<PathBuf>> {
    let list = candidates(info)?;
    if list.is_empty() {
        return Ok(None);
    }
    let mut reasons = vec![];
    for path in list {
        busy("Checking the cartridge's sound engine...");
        let data = std::fs::read(&path).with_context(|| format!("Couldn't read {}", path.display()))?;
        let pkg = parse(&data).with_context(|| format!("{}", path.display()))?;
        match try_package(link, info, &pkg)? {
            Ok(player) => {
                let out = PathBuf::from(RIP_PLAYER);
                std::fs::write(&out, &player).context("Couldn't write the music player")?;
                log::info!("Cartridge Music: {} matches", path.display());
                return Ok(Some(out));
            }
            Err(why) => {
                log::info!("Cartridge Music: {} doesn't match: {why}", path.display());
                reasons.push(why);
            }
        }
    }
    bail!(
        "There's music for this game, but not for this version of the cartridge: {}.",
        reasons.first().map(String::as_str).unwrap_or("it doesn't match")
    )
}
