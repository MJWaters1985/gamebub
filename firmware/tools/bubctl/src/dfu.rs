//! Flashing through the update firmware's USB protocol (handheld-dfu/src/protocol.rs): 32-byte
//! command headers and data on the vendor interface's bulk endpoints, a zero-length packet to
//! acknowledge, and a control request for the command's status.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use nusb::transfer::{Control, ControlType, Direction, Recipient, RequestBuffer};

pub const PRODUCT: &str = "Game Bub Handheld DFU";
const MAGIC: u32 = 0x431F_D10B;
const FLASH_BASE: u32 = 0x3C00_0000;
/// The update firmware refuses to write below this (bootloader, partition table, itself).
pub const PROTECT: u32 = 0x8_0000;
const SECTOR: u32 = 4096;
const CMD_ERASE: u8 = 0x03;
const CMD_READ: u8 = 0x84;
const CMD_WRITE: u8 = 0x05;
const REQ_GET_INFO: u8 = 0;
const REQ_REBOOT: u8 = 1;
const REQ_STATUS: u8 = 0x42;
const CHUNK: u32 = 4096;

type Result<T> = std::result::Result<T, String>;

pub struct Dfu {
    dev: nusb::Device,
    iface: nusb::Interface,
    number: u8,
    ep_out: u8,
    ep_in: u8,
    token: u32,
}

impl Dfu {
    pub fn find() -> Result<Option<Dfu>> {
        let devices = nusb::list_devices().map_err(|e| format!("Couldn't list USB devices: {e}"))?;
        let Some(info) = devices.into_iter().find(|d| d.product_string() == Some(PRODUCT)) else {
            return Ok(None);
        };
        let dev = info.open().map_err(|e| format!("Couldn't open the update firmware: {e}"))?;
        let config = dev.active_configuration().map_err(|e| format!("{e}"))?;
        let mut found = None;
        for alt in config.interface_alt_settings() {
            if alt.class() != 0xFF {
                continue;
            }
            let mut out = None;
            let mut inp = None;
            for ep in alt.endpoints() {
                match ep.direction() {
                    Direction::Out => out = Some(ep.address()),
                    Direction::In => inp = Some(ep.address()),
                }
            }
            if let (Some(o), Some(i)) = (out, inp) {
                found = Some((alt.interface_number(), o, i));
                break;
            }
        }
        let (number, ep_out, ep_in) = found.ok_or("The update firmware has no vendor interface")?;
        let iface = dev.claim_interface(number).map_err(|e| format!("Couldn't claim the interface: {e}"))?;
        Ok(Some(Dfu { dev, iface, number, ep_out, ep_in, token: 0 }))
    }

    fn control(&self, request: u8, value: u16) -> Control {
        Control {
            control_type: ControlType::Vendor,
            recipient: Recipient::Interface,
            request,
            value,
            index: self.number as u16,
        }
    }

    /// Hardware version (product in the top byte, major below it).
    pub fn hardware(&self) -> Result<u32> {
        let mut b = [0u8; 24];
        let n = self
            .dev
            .control_in_blocking(self.control(REQ_GET_INFO, 0), &mut b, Duration::from_secs(2))
            .map_err(|e| format!("Info request failed: {e}"))?;
        if n < 12 {
            return Err("Short info reply".into());
        }
        Ok(u32::from_le_bytes(b[8..12].try_into().unwrap()))
    }

    fn status(&self) -> String {
        let mut b = [0u8; 16];
        match self.dev.control_in_blocking(self.control(REQ_STATUS, 0), &mut b, Duration::from_secs(2)) {
            Ok(16) => format!(
                "command {:#04x}, status {}{}",
                b[8],
                u32::from_le_bytes(b[4..8].try_into().unwrap()),
                if b[9] != 0 { ", still running" } else { "" }
            ),
            _ => "status unknown".into(),
        }
    }

    pub fn reboot(&self) -> Result<()> {
        self.dev
            .control_out_blocking(self.control(REQ_REBOOT, 1), &[], Duration::from_secs(2))
            .map(|_| ())
            .map_err(|e| format!("Restart request failed: {e}"))
    }

    fn wait<T: Send + 'static>(
        fut: impl std::future::Future<Output = nusb::transfer::Completion<T>> + Send + 'static,
        timeout: Duration,
    ) -> std::result::Result<T, String> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(futures_lite::future::block_on(fut).into_result());
        });
        match rx.recv_timeout(timeout) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(format!("{e}")),
            Err(_) => Err("timed out".into()),
        }
    }

    fn out(&self, data: Vec<u8>, timeout: Duration) -> Result<()> {
        Self::wait(self.iface.bulk_out(self.ep_out, data), timeout).map(|_| ())
    }

    fn input(&self, len: usize, timeout: Duration) -> Result<Vec<u8>> {
        Self::wait(self.iface.bulk_in(self.ep_in, RequestBuffer::new(len)), timeout)
    }

    /// Run one command: header, then data out or in, then the acknowledgement.
    fn command(&mut self, cmd: u8, a: u32, b: u32, data: Option<&[u8]>, read: usize, timeout: Duration) -> Result<Vec<u8>> {
        self.token = self.token.wrapping_add(1);
        let transfer = data.map_or(read, <[u8]>::len) as u32;
        let mut h = Vec::with_capacity(32);
        h.extend_from_slice(&MAGIC.to_le_bytes());
        h.extend_from_slice(&self.token.to_le_bytes());
        h.extend_from_slice(&[cmd, 8, 0, 0]);
        h.extend_from_slice(&transfer.to_le_bytes());
        h.extend_from_slice(&a.to_le_bytes());
        h.extend_from_slice(&b.to_le_bytes());
        h.resize(32, 0);
        let fail = |this: &Self, what: &str, e: String| format!("{what} failed ({e}; {})", this.status());
        self.out(h, Duration::from_secs(2)).map_err(|e| fail(self, "Sending a command", e))?;
        if let Some(d) = data {
            self.out(d.to_vec(), timeout).map_err(|e| fail(self, "Sending data", e))?;
        }
        let mut got = Vec::with_capacity(read);
        while got.len() < read {
            let part = self.input(read - got.len(), timeout).map_err(|e| fail(self, "Reading", e))?;
            if part.is_empty() {
                return Err(fail(self, "Reading", "short read".into()));
            }
            got.extend_from_slice(&part);
        }
        if cmd & 0x80 != 0 {
            self.out(vec![], Duration::from_secs(2)).map_err(|e| fail(self, "Acknowledging", e))?;
        } else {
            let ack = self.input(64, timeout).map_err(|e| fail(self, "The command", e))?;
            if !ack.is_empty() {
                return Err("Unexpected reply instead of an acknowledgement".into());
            }
        }
        Ok(got)
    }

    /// Erase, write and read back each run of (flash offset, bytes).
    pub fn flash(&mut self, runs: &[(u32, Vec<u8>)]) -> Result<()> {
        let total: usize = runs.iter().map(|r| r.1.len()).sum();
        for (start, data) in runs {
            let end = start + data.len() as u32;
            if *start < PROTECT || start % SECTOR != 0 {
                return Err(format!("Refusing to write at {start:#x}"));
            }
            let erase_end = end.div_ceil(SECTOR) * SECTOR;
            let mut at = *start;
            while at < erase_end {
                let n = (erase_end - at).min(64 * 1024);
                self.command(CMD_ERASE, at, n, None, 0, Duration::from_secs(30))?;
                at += n;
                eprint!("\rErasing {:3}%", (at - start) as u64 * 100 / (erase_end - start) as u64);
            }
            eprintln!();
        }
        let started = Instant::now();
        let mut done = 0usize;
        for (start, data) in runs {
            for (i, piece) in data.chunks(CHUNK as usize).enumerate() {
                let addr = FLASH_BASE + start + i as u32 * CHUNK;
                self.command(CMD_WRITE, addr, piece.len() as u32, Some(piece), 0, Duration::from_secs(10))?;
                done += piece.len();
                if i % 16 == 0 {
                    eprint!("\rWriting {:3}%", done * 100 / total);
                }
            }
        }
        eprintln!("\rWriting 100% ({:.0} s)", started.elapsed().as_secs_f32());
        done = 0;
        for (start, data) in runs {
            for (i, piece) in data.chunks(CHUNK as usize).enumerate() {
                let addr = FLASH_BASE + start + i as u32 * CHUNK;
                let back = self.command(CMD_READ, addr, piece.len() as u32, None, piece.len(), Duration::from_secs(10))?;
                if back != piece {
                    return Err(format!("Verify failed at {:#x}", start + i as u32 * CHUNK));
                }
                done += piece.len();
                if i % 16 == 0 {
                    eprint!("\rVerifying {:3}%", done * 100 / total);
                }
            }
        }
        eprintln!("\rVerifying 100%");
        Ok(())
    }
}

/// The flash contents of a UF2 file for this hardware, as runs of (flash offset, bytes), with
/// the same checks the update drive makes; and the commit hash it carries, if any.
pub fn read_uf2(file: &[u8], hardware: u32) -> Result<(Vec<(u32, Vec<u8>)>, Option<String>)> {
    const FAMILY: u32 = 0xC47E_5767;
    const TAG_DEVICE: u32 = 0xC8A729;
    const TAG_COMMIT: u32 = 0x8A4E54;
    let (product, major) = ((hardware >> 24) as u8, (hardware >> 16) as u8);
    let mut blocks = std::collections::BTreeMap::new();
    let mut commit = None;
    for b in file.chunks(512) {
        let w = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        if b.len() != 512 || w(0) != 0x0A32_4655 || w(4) != 0x9E5D_5157 || w(508) != 0x0AB1_6F30 {
            continue;
        }
        let (flags, addr, len) = (w(8), w(12), w(16) as usize);
        if flags & 0x2000 == 0 || w(28) != FAMILY || len != 256 || addr % 256 != 0 {
            continue;
        }
        let mut tags = &b[32 + len..508];
        if flags & 0x8000 != 0 {
            while tags.len() >= 4 {
                let tag_len = tags[0] as usize;
                let tag = u32::from_le_bytes(tags[0..4].try_into().unwrap()) >> 8;
                if tag_len < 4 || tag_len > tags.len() {
                    break;
                }
                let payload = &tags[4..tag_len];
                if tag == TAG_DEVICE {
                    if payload.len() != 8 || &payload[0..4] != b"bub!" {
                        return Err("This UF2 isn't Game Bub firmware".into());
                    }
                    if major != 0 && major != 255 && (payload[6] != major || payload[7] != product) {
                        return Err(format!(
                            "This firmware is for hardware revision {}, the Game Bub is revision {major}",
                            payload[6]
                        ));
                    }
                }
                if tag == TAG_COMMIT {
                    commit = Some(String::from_utf8_lossy(payload).trim_end_matches('\0').to_string());
                }
                tags = &tags[(tag_len + 3) & !3..];
            }
        }
        if addr < PROTECT {
            return Err(format!("This UF2 writes the protected area at {addr:#x}"));
        }
        blocks.insert(addr, b[32..32 + len].to_vec());
    }
    if blocks.is_empty() {
        return Err("No Game Bub firmware blocks in this file".into());
    }
    let mut runs: Vec<(u32, Vec<u8>)> = vec![];
    for (addr, data) in blocks {
        match runs.last_mut() {
            Some((start, bytes)) if *start + bytes.len() as u32 == addr => bytes.extend_from_slice(&data),
            _ => runs.push((addr, data)),
        }
    }
    Ok((runs, commit))
}
