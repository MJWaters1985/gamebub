//! Link to the cartridge utility firmware running on the boot bitstream's soft CPU.
//!
//! This is the same firmware used for "FlashGBX Cartridge Backup / Restore" mode
//! (fw_cart_backup.bin), which speaks the GBxCart RW protocol (as implemented by FlashGBX,
//! https://github.com/lesserkuma/FlashGBX, GPL-3.0). Here the protocol bytes go through the
//! FPGA FIFO directly instead of being relayed from USB.

use std::time::{Duration, Instant};

use esp_idf_svc::hal::units::Hertz;

use crate::bitstream::boot::{FIFO_BASE, REG_BASE};
use crate::device::{drivers::fpga, Device};

const REG_CPU_RESET_N: u32 = REG_BASE | 0x0;
const REG_TX_COUNT: u32 = REG_BASE | 0x100;
const REG_TX_READ_LIMIT: u32 = REG_BASE | 0x104;
const REG_RX_COUNT: u32 = REG_BASE | 0x110;
const REG_CART_EN: u32 = REG_BASE | 0x120;

const RX_SIZE: u32 = 4096;
const MAX_CLOCK: Hertz = Hertz(10_000_000);
const TIMEOUT: Duration = Duration::from_secs(2);

/// Largest read transfer the firmware supports.
pub const MAX_READ: usize = 1024;
/// Largest write transfer used.
pub const MAX_WRITE: usize = 256;

mod cmd {
    pub const QUERY_FW_INFO: u8 = 0xA1;
    pub const SET_MODE_AGB: u8 = 0xA2;
    pub const SET_MODE_DMG: u8 = 0xA3;
    pub const SET_VOLTAGE_3_3V: u8 = 0xA4;
    pub const SET_VOLTAGE_5V: u8 = 0xA5;
    pub const SET_VARIABLE: u8 = 0xA6;
    pub const DMG_CART_READ: u8 = 0xB1;
    pub const DMG_CART_WRITE: u8 = 0xB2;
    pub const DMG_CART_WRITE_SRAM: u8 = 0xB3;
    pub const DMG_MBC_RESET: u8 = 0xB4;
    pub const AGB_CART_READ: u8 = 0xC1;
    pub const AGB_CART_WRITE: u8 = 0xC2;
    pub const AGB_CART_READ_SRAM: u8 = 0xC3;
    pub const AGB_CART_WRITE_SRAM: u8 = 0xC4;
    pub const AGB_CART_READ_EEPROM: u8 = 0xC5;
    pub const AGB_CART_WRITE_EEPROM: u8 = 0xC6;
    pub const AGB_CART_WRITE_FLASH_DATA: u8 = 0xC7;
    pub const AGB_BOOTUP_SEQUENCE: u8 = 0xC9;
    pub const CART_WRITE_FLASH_CMD: u8 = 0xD4;
    pub const CART_PWR_ON: u8 = 0xF2;
    pub const CART_PWR_OFF: u8 = 0xF3;
    pub const QUERY_CART_PWR: u8 = 0xF4;
    pub const GET_SWITCH_STATE: u8 = 0xF6;
}

/// Firmware variables: (size in bytes, id).
mod var {
    pub const ADDRESS: (u8, u32) = (4, 0x00);
    pub const TRANSFER_SIZE: (u8, u32) = (2, 0x00);
    pub const STATUS_REGISTER_MASK: (u8, u32) = (2, 0x05);
    pub const STATUS_REGISTER_VALUE: (u8, u32) = (2, 0x06);
    pub const CART_MODE: (u8, u32) = (1, 0x00);
    pub const DMG_ACCESS_MODE: (u8, u32) = (1, 0x01);
    pub const DMG_READ_CS_PULSE: (u8, u32) = (1, 0x08);
    pub const DMG_WRITE_CS_PULSE: (u8, u32) = (1, 0x09);
    pub const DMG_READ_METHOD: (u8, u32) = (1, 0x0B);
    pub const AGB_READ_METHOD: (u8, u32) = (1, 0x0C);
    pub const AGB_IRQ_ENABLED: (u8, u32) = (1, 0x10);
}

/// DMG_ACCESS_MODE values.
const ACCESS_ROM_READ: u32 = 1;
const ACCESS_RAM_READ: u32 = 3;
const ACCESS_RAM_WRITE: u32 = 4;

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("the cartridge reader didn't respond")]
    Timeout,
    #[error("the cartridge reader reported an error ({0:#04x})")]
    BadAck(u8),
    #[error("unexpected firmware response")]
    Protocol,
}

pub type Result<T> = std::result::Result<T, LinkError>;

/// Which kind of cartridge is inserted, from the cartridge slot switch.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CartMode {
    /// Game Boy / Game Boy Color
    Dmg,
    /// Game Boy Advance
    Agb,
}

/// GBA EEPROM size, as used by the EEPROM commands.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EepromSize {
    /// 4 Kbit (512 bytes)
    Small,
    /// 64 Kbit (8 KiB)
    Large,
}

impl EepromSize {
    fn code(self) -> u8 {
        match self {
            Self::Small => 1,
            Self::Large => 2,
        }
    }
}

pub struct Link {
    cart_powered: bool,
    mode: Option<CartMode>,
}

impl Link {
    /// Load the cartridge utility firmware into the soft CPU and start it.
    ///
    /// The boot bitstream must be loaded (it is whenever the main menu is shown).
    pub fn open() -> anyhow::Result<Self> {
        let file = crate::util::open_system_file("fw_cart_backup.bin")?;
        {
            let mut device = Device::lock();
            device.fpga.write_u32(REG_CPU_RESET_N, 0)?;
            crate::bitstream::boot::load_cpu_memory(&mut device, 0, file)?;
            device.fpga.write_u32(REG_CPU_RESET_N, 1)?;
        }
        let mut link = Link {
            cart_powered: false,
            mode: None,
        };
        // Give the firmware a moment to start, discard anything left in the FIFO from an
        // earlier session, then check that it responds.
        std::thread::sleep(Duration::from_millis(50));
        link.drain();
        link.query_firmware()?;
        Ok(link)
    }

    /// Power off the cartridge and stop the soft CPU.
    pub fn close(mut self) {
        if self.cart_powered {
            let _ = self.command_ack(&[cmd::CART_PWR_OFF]);
        }
        let mut device = Device::lock();
        let _ = device.fpga.write_u32(REG_CPU_RESET_N, 0);
        device.set_cart_power(false);
    }

    /// Mirror the firmware's cartridge power request onto the power switch.
    fn sync_power(&mut self, device: &mut Device) {
        let enabled = device.fpga.read_u32(REG_CART_EN).unwrap_or(0) != 0;
        if enabled != self.cart_powered {
            device.set_cart_power(enabled);
            self.cart_powered = enabled;
        }
    }

    fn write(&mut self, mut data: &[u8]) -> Result<()> {
        let deadline = Instant::now() + TIMEOUT;
        while !data.is_empty() {
            let mut device = Device::lock();
            self.sync_power(&mut device);
            let used = device.fpga.read_u32(REG_RX_COUNT).map_err(|_| LinkError::Protocol)?;
            let space = RX_SIZE.saturating_sub(used) as usize;
            if space > 0 {
                let n = space.min(data.len());
                let command = fpga::SpiCommand::new(fpga::FpgaSpiWordSize::Bits8);
                device
                    .fpga
                    .spi_write(Some(MAX_CLOCK), command, FIFO_BASE, &data[..n])
                    .map_err(|_| LinkError::Protocol)?;
                data = &data[n..];
            } else {
                drop(device);
                if Instant::now() > deadline {
                    return Err(LinkError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(())
    }

    fn read(&mut self, mut out: &mut [u8]) -> Result<()> {
        let mut deadline = Instant::now() + TIMEOUT;
        while !out.is_empty() {
            let mut device = Device::lock();
            self.sync_power(&mut device);
            let ready = device.fpga.read_u32(REG_TX_COUNT).map_err(|_| LinkError::Protocol)?;
            if ready > 0 {
                let n = (ready as usize).min(out.len());
                device
                    .fpga
                    .write_u32(REG_TX_READ_LIMIT, n as u32)
                    .map_err(|_| LinkError::Protocol)?;
                let command = fpga::SpiCommand::new(fpga::FpgaSpiWordSize::Bits8);
                let (chunk, rest) = out.split_at_mut(n);
                device
                    .fpga
                    .spi_read(Some(MAX_CLOCK), command, FIFO_BASE, chunk)
                    .map_err(|_| LinkError::Protocol)?;
                out = rest;
                deadline = Instant::now() + TIMEOUT;
            } else {
                drop(device);
                if Instant::now() > deadline {
                    return Err(LinkError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(())
    }

    /// Discard any bytes waiting from the firmware.
    fn drain(&mut self) {
        let mut buf = [0u8; 256];
        loop {
            let ready = Device::lock().fpga.read_u32(REG_TX_COUNT).unwrap_or(0) as usize;
            if ready == 0 {
                break;
            }
            let n = ready.min(buf.len());
            if self.read(&mut buf[..n]).is_err() {
                break;
            }
        }
    }

    fn read_u8(&mut self) -> Result<u8> {
        let mut b = [0u8];
        self.read(&mut b)?;
        Ok(b[0])
    }

    fn wait_ack(&mut self) -> Result<()> {
        match self.read_u8()? {
            0x01 | 0x03 => Ok(()),
            other => Err(LinkError::BadAck(other)),
        }
    }

    fn command_ack(&mut self, data: &[u8]) -> Result<()> {
        self.write(data)?;
        self.wait_ack()
    }

    fn query_firmware(&mut self) -> Result<()> {
        self.write(&[cmd::QUERY_FW_INFO])?;
        if self.read_u8()? != 8 {
            return Err(LinkError::Protocol);
        }
        let mut info = [0u8; 8];
        self.read(&mut info)?;
        let fw_ver = u16::from_be_bytes([info[1], info[2]]);
        if info[0] != b'L' || fw_ver < 15 {
            log::warn!("Unexpected cartridge firmware {}{}", info[0] as char, fw_ver);
            return Err(LinkError::Protocol);
        }
        // Device name, capability flags and bootloader flag follow.
        let name_len = self.read_u8()? as usize;
        let mut name = vec![0u8; name_len];
        self.read(&mut name)?;
        self.read_u8()?;
        self.read_u8()?;
        Ok(())
    }

    fn set_var(&mut self, var: (u8, u32), value: u32) -> Result<()> {
        let mut buf = [0u8; 10];
        buf[0] = cmd::SET_VARIABLE;
        buf[1] = var.0;
        buf[2..6].copy_from_slice(&var.1.to_be_bytes());
        buf[6..10].copy_from_slice(&value.to_be_bytes());
        self.command_ack(&buf)
    }

    /// Which kind of cartridge the slot switch reports.
    pub fn cart_mode_switch(&mut self) -> Result<CartMode> {
        self.write(&[cmd::GET_SWITCH_STATE])?;
        let state = self.read_u8()?;
        Ok(if (state >> 1) & 1 == 1 {
            CartMode::Dmg
        } else {
            CartMode::Agb
        })
    }

    /// Select the cartridge mode and power on the cartridge.
    pub fn set_mode(&mut self, mode: CartMode) -> Result<()> {
        match mode {
            CartMode::Dmg => {
                self.command_ack(&[cmd::SET_MODE_DMG])?;
                self.command_ack(&[cmd::SET_VOLTAGE_5V])?;
                self.set_var(var::DMG_READ_METHOD, 1)?;
                self.set_var(var::CART_MODE, 1)?;
            }
            CartMode::Agb => {
                self.command_ack(&[cmd::SET_MODE_AGB])?;
                self.command_ack(&[cmd::SET_VOLTAGE_3_3V])?;
                self.set_var(var::AGB_READ_METHOD, 2)?;
                self.set_var(var::CART_MODE, 2)?;
                self.set_var(var::AGB_IRQ_ENABLED, 0)?;
            }
        }
        self.set_var(var::ADDRESS, 0)?;
        self.mode = Some(mode);

        self.write(&[cmd::QUERY_CART_PWR])?;
        if self.read_u8()? == 0 {
            let set_mode = match mode {
                CartMode::Dmg => cmd::SET_MODE_DMG,
                CartMode::Agb => cmd::SET_MODE_AGB,
            };
            self.command_ack(&[set_mode])?;
            self.command_ack(&[cmd::CART_PWR_ON])?;
            self.write(&[cmd::QUERY_CART_PWR])?;
            if self.read_u8()? != 1 {
                log::warn!("Cartridge power didn't turn on");
            }
            match mode {
                CartMode::Dmg => self.command_ack(&[cmd::DMG_MBC_RESET])?,
                CartMode::Agb => self.command_ack(&[cmd::AGB_BOOTUP_SEQUENCE])?,
            }
        }
        Ok(())
    }

    /// Power the cartridge off and on again, resetting its mapper.
    pub fn power_cycle(&mut self) -> Result<()> {
        self.command_ack(&[cmd::CART_PWR_OFF])?;
        std::thread::sleep(Duration::from_millis(100));
        let mode = self.mode.ok_or(LinkError::Protocol)?;
        self.set_mode(mode)
    }

    // ---- Game Boy ----

    /// Write a byte to the cartridge bus (mapper registers).
    pub fn dmg_write(&mut self, address: u16, value: u8) -> Result<()> {
        let mut buf = [0u8; 6];
        buf[0] = cmd::DMG_CART_WRITE;
        buf[1..5].copy_from_slice(&(address as u32).to_be_bytes());
        buf[5] = value;
        self.command_ack(&buf)
    }

    pub fn dmg_mbc_reset(&mut self) -> Result<()> {
        self.command_ack(&[cmd::DMG_MBC_RESET])
    }

    /// Read from the ROM area (0x0000-0x7FFF).
    pub fn dmg_read_rom(&mut self, address: u16, out: &mut [u8]) -> Result<()> {
        let chunk = MAX_READ.min(out.len());
        self.set_var(var::TRANSFER_SIZE, chunk as u32)?;
        self.set_var(var::ADDRESS, address as u32)?;
        self.set_var(var::DMG_ACCESS_MODE, ACCESS_ROM_READ)?;
        for part in out.chunks_mut(chunk) {
            assert_eq!(part.len(), chunk);
            self.write(&[cmd::DMG_CART_READ])?;
            self.read(part)?;
        }
        Ok(())
    }

    /// Read from cartridge RAM (offset from 0xA000).
    pub fn dmg_read_ram(&mut self, offset: u16, out: &mut [u8]) -> Result<()> {
        let chunk = MAX_READ.min(out.len());
        self.set_var(var::TRANSFER_SIZE, chunk as u32)?;
        self.set_var(var::ADDRESS, 0xA000 + offset as u32)?;
        self.set_var(var::DMG_ACCESS_MODE, ACCESS_RAM_READ)?;
        self.set_var(var::DMG_READ_CS_PULSE, 1)?;
        for part in out.chunks_mut(chunk) {
            assert_eq!(part.len(), chunk);
            self.write(&[cmd::DMG_CART_READ])?;
            self.read(part)?;
        }
        self.set_var(var::DMG_READ_CS_PULSE, 0)
    }

    /// Write to cartridge RAM (offset from 0xA000).
    pub fn dmg_write_ram(&mut self, offset: u16, data: &[u8]) -> Result<()> {
        let chunk = MAX_WRITE.min(data.len());
        self.set_var(var::TRANSFER_SIZE, chunk as u32)?;
        self.set_var(var::ADDRESS, 0xA000 + offset as u32)?;
        self.set_var(var::DMG_ACCESS_MODE, ACCESS_RAM_WRITE)?;
        self.set_var(var::DMG_WRITE_CS_PULSE, 1)?;
        for part in data.chunks(chunk) {
            assert_eq!(part.len(), chunk);
            self.write(&[cmd::DMG_CART_WRITE_SRAM])?;
            self.command_ack(part)?;
        }
        self.set_var(var::ADDRESS, 0)?;
        self.set_var(var::DMG_WRITE_CS_PULSE, 0)
    }

    pub fn dmg_set_cs_pulses(&mut self, read: bool, write: bool) -> Result<()> {
        self.set_var(var::DMG_READ_CS_PULSE, read as u32)?;
        self.set_var(var::DMG_WRITE_CS_PULSE, write as u32)
    }

    // ---- Game Boy Advance ----

    /// Read ROM starting at a byte address.
    pub fn agb_read_rom(&mut self, address: u32, out: &mut [u8]) -> Result<()> {
        let chunk = MAX_READ.min(out.len());
        self.set_var(var::TRANSFER_SIZE, chunk as u32)?;
        self.set_var(var::ADDRESS, address >> 1)?;
        for part in out.chunks_mut(chunk) {
            assert_eq!(part.len(), chunk);
            self.write(&[cmd::AGB_CART_READ])?;
            self.read(part)?;
        }
        Ok(())
    }

    /// Write a 16-bit value to the ROM area (e.g. the GPIO port used by cartridge clocks).
    pub fn agb_write(&mut self, address: u32, value: u16) -> Result<()> {
        let mut buf = [0u8; 7];
        buf[0] = cmd::AGB_CART_WRITE;
        buf[1..5].copy_from_slice(&(address >> 1).to_be_bytes());
        buf[5..7].copy_from_slice(&value.to_be_bytes());
        self.command_ack(&buf)
    }

    /// Read SRAM / FRAM / Flash save memory.
    pub fn agb_read_sram(&mut self, address: u32, out: &mut [u8]) -> Result<()> {
        let chunk = MAX_READ.min(out.len());
        self.set_var(var::TRANSFER_SIZE, chunk as u32)?;
        self.set_var(var::ADDRESS, address)?;
        for part in out.chunks_mut(chunk) {
            assert_eq!(part.len(), chunk);
            self.write(&[cmd::AGB_CART_READ_SRAM])?;
            self.read(part)?;
        }
        Ok(())
    }

    /// Write SRAM / FRAM save memory.
    pub fn agb_write_sram(&mut self, address: u32, data: &[u8]) -> Result<()> {
        self.agb_write_with(address, data, &[cmd::AGB_CART_WRITE_SRAM])
    }

    /// Program Flash save memory (the sector must have been erased).
    pub fn agb_write_flash(&mut self, address: u32, data: &[u8]) -> Result<()> {
        self.agb_write_with(address, data, &[cmd::AGB_CART_WRITE_FLASH_DATA, 1])
    }

    fn agb_write_with(&mut self, address: u32, data: &[u8], command: &[u8]) -> Result<()> {
        let chunk = MAX_WRITE.min(data.len());
        self.set_var(var::TRANSFER_SIZE, chunk as u32)?;
        self.set_var(var::ADDRESS, address)?;
        for part in data.chunks(chunk) {
            assert_eq!(part.len(), chunk);
            self.write(command)?;
            self.command_ack(part)?;
        }
        Ok(())
    }

    /// Set up status register polling used when programming Flash save memory.
    pub fn agb_flash_status_setup(&mut self) -> Result<()> {
        self.set_var(var::STATUS_REGISTER_MASK, 0x80)?;
        self.set_var(var::STATUS_REGISTER_VALUE, 0x80)
    }

    /// Write a sequence of (address, value) commands to Flash save memory.
    pub fn agb_flash_commands(&mut self, commands: &[(u32, u16)]) -> Result<()> {
        let mut buf = vec![cmd::CART_WRITE_FLASH_CMD, 0, commands.len() as u8];
        for &(address, value) in commands {
            buf.extend_from_slice(&address.to_be_bytes());
            buf.extend_from_slice(&value.to_be_bytes());
        }
        self.command_ack(&buf)
    }

    /// Read EEPROM save memory, starting at a byte offset (multiple of 8).
    pub fn agb_read_eeprom(&mut self, size: EepromSize, offset: u32, out: &mut [u8]) -> Result<()> {
        // Like FlashGBX, start a new transfer for every chunk.
        let chunk = 0x100.min(out.len());
        for (i, part) in out.chunks_mut(chunk).enumerate() {
            assert_eq!(part.len(), chunk);
            self.set_var(var::TRANSFER_SIZE, chunk as u32)?;
            self.set_var(var::ADDRESS, (offset + (i * chunk) as u32) / 8)?;
            self.write(&[cmd::AGB_CART_READ_EEPROM, size.code()])?;
            self.read(part)?;
        }
        Ok(())
    }

    /// Write EEPROM save memory, starting at a byte offset (multiple of 8).
    pub fn agb_write_eeprom(&mut self, size: EepromSize, offset: u32, data: &[u8]) -> Result<()> {
        let chunk = 0x40.min(data.len());
        for (i, part) in data.chunks(chunk).enumerate() {
            assert_eq!(part.len(), chunk);
            self.set_var(var::TRANSFER_SIZE, chunk as u32)?;
            self.set_var(var::ADDRESS, (offset + (i * chunk) as u32) / 8)?;
            self.write(&[cmd::AGB_CART_WRITE_EEPROM, size.code()])?;
            self.command_ack(part)?;
        }
        Ok(())
    }
}
