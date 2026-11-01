//! Game database for identifying cartridges (see scripts/gen_game_db.py).

use super::link::CartMode;

static DATABASE: &[u8] = include_bytes!("../../res/gamedb.bin");

const RECORD_SIZE: usize = 24;

/// A known game.
#[derive(Clone, Debug)]
pub struct GameEntry {
    /// No-Intro name, e.g. "Pokemon - Emerald Version (USA, Europe)"
    pub name: String,
    /// CRC32 of a good ROM dump
    pub crc32: u32,
    /// ROM size in bytes
    pub rom_size: u32,
    /// GBA save type (FlashGBX numbering): 0 none, 1 4K EEPROM, 2 64K EEPROM, 3 SRAM,
    /// 4 512K Flash, 5 1M Flash, 6 DACS, 7 512K SRAM, 8 1M SRAM. Always 0 for Game Boy.
    pub agb_save_type: u8,
}

fn u32_at(offset: usize) -> u32 {
    u32::from_le_bytes(DATABASE[offset..offset + 4].try_into().unwrap())
}

/// Look up a game by its cartridge header (the first 0x180 bytes of the ROM).
pub fn lookup(mode: CartMode, header: &[u8]) -> Option<GameEntry> {
    if DATABASE.len() < 32 || &DATABASE[0..4] != b"GBDB" {
        return None;
    }
    let table = match mode {
        CartMode::Dmg => 8,
        CartMode::Agb => 16,
    };
    let (start, count) = (u32_at(table) as usize, u32_at(table + 4) as usize);
    let strings = u32_at(24) as usize;

    let digest = sha1(header);
    let key = &digest[..8];
    let records = &DATABASE[start..start + count * RECORD_SIZE];
    let index = records
        .chunks_exact(RECORD_SIZE)
        .collect::<Vec<_>>()
        .binary_search_by(|r| r[..8].cmp(key))
        .ok()?;
    let r = &records[index * RECORD_SIZE..(index + 1) * RECORD_SIZE];
    let field = |o: usize| u32::from_le_bytes(r[o..o + 4].try_into().unwrap());
    let name_offset = strings + field(16) as usize;
    let name_len = u16::from_le_bytes([r[20], r[21]]) as usize;
    Some(GameEntry {
        name: String::from_utf8_lossy(&DATABASE[name_offset..name_offset + name_len]).into_owned(),
        crc32: field(8),
        rom_size: field(12),
        agb_save_type: r[22],
    })
}

/// SHA-1 (only used for database keys).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for block in message.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }

    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// CRC32 (IEEE), computed incrementally.
pub struct Crc32(u32);

impl Crc32 {
    pub fn new() -> Self {
        Crc32(0xFFFF_FFFF)
    }

    pub fn update(&mut self, data: &[u8]) {
        let mut crc = self.0;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
            }
        }
        self.0 = crc;
    }

    pub fn finish(&self) -> u32 {
        !self.0
    }
}
