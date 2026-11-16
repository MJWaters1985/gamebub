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
    player: Vec<u8>,
}

fn parse(data: &[u8]) -> Result<Package> {
    let mut p = 0usize;
    let mut take = |n: usize| -> Result<&[u8]> {
        let s = data.get(p..p + n).context("package is cut short")?;
        p += n;
        Ok(s)
    };
    if take(4)? != b"CMR1" {
        bail!("not a cartridge music package");
    }
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
    let _flags = take(1)?[0];
    let player = take(PLAYER_SIZE)?.to_vec();
    if table_at + table_size > PLAYER_SIZE || constants.iter().any(|&o: &usize| o >= PLAYER_SIZE) {
        bail!("package offsets out of range");
    }
    Ok(Package { table_at, table_size, constants, probes, checks, player })
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

fn crc(link: &mut Link, info: &CartInfo, bank: u32, offset: u16, len: u16) -> Result<u32> {
    let mut buf = vec![0u8; len as usize];
    cart::read_rom_bank(link, info, bank, offset, &mut buf)?;
    let mut c = Crc32::new();
    c.update(&buf);
    Ok(c.finish())
}

/// Check a package against the cartridge; the player file with the bank table filled in.
fn try_package(link: &mut Link, info: &CartInfo, pkg: &Package) -> Result<std::result::Result<Vec<u8>, String>> {
    let banks = (info.rom_size / 0x4000).max(2);
    let mut table: Vec<u8> = (0..pkg.table_size as u32).map(|b| b as u8).collect();
    for (rip_bank, off, probe) in &pkg.probes {
        let mut found = None;
        let mut buf = vec![0u8; probe.len()];
        for bank in 1..banks {
            cart::read_rom_bank(link, info, bank, *off, &mut buf)?;
            if buf != *probe {
                continue;
            }
            let mut all = true;
            for &(b, o, len, want) in pkg.checks.iter().filter(|c| c.0 == *rip_bank) {
                let _ = b;
                if crc(link, info, bank, o, len)? != want {
                    all = false;
                    break;
                }
            }
            if all {
                found = Some(bank);
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
        if crc(link, info, 0, o, len)? != want {
            return Ok(Err("the cartridge's code differs from the rip's".into()));
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
fn candidates(info: &CartInfo) -> Result<Vec<PathBuf>> {
    let dir = crate::util::get_system_file_path("cart-music/rips");
    let index = match std::fs::read_to_string(dir.join("index.txt")) {
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
    let mut out: Vec<PathBuf> = vec![];
    for name in names.iter().filter(|n| !n.is_empty()) {
        for line in index.lines() {
            if let Some((n, file)) = line.split_once('\t') {
                let p = dir.join(file);
                if n == name && !out.contains(&p) {
                    out.push(p);
                }
            }
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
                let out = crate::util::get_system_file_path("cart-music/rip-player.bin");
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
