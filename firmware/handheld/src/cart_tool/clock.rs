//! Cartridge clocks: the Seiko S-3511 real-time clock on GBA cartridges (Pokémon Ruby /
//! Sapphire / Emerald, Boktai, ...) and the MBC3 clock on Game Boy cartridges (Pokémon Gold /
//! Silver / Crystal, ...).
//!
//! The access sequences follow FlashGBX (https://github.com/lesserkuma/FlashGBX).

use anyhow::bail;
use time::OffsetDateTime;

use super::cart::{CartInfo, Mbc};
use super::link::{CartMode, Link};

/// What a cartridge clock says.
#[derive(Copy, Clone, Debug)]
pub enum Clock {
    /// GBA: calendar date and time.
    Agb {
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        /// The clock lost power at some point (dead or replaced battery).
        power_lost: bool,
    },
    /// Game Boy MBC3: a day counter and time of day. Games add their own offset, so this isn't
    /// a calendar date.
    Mbc3 { days: u16, hour: u8, minute: u8, second: u8, halted: bool, carry: bool },
}

impl Clock {
    pub fn describe(&self) -> String {
        match *self {
            Clock::Agb { power_lost: true, .. } => "Clock: lost power (battery may be dead)".into(),
            Clock::Agb { year, month, day, hour, minute, second, .. } => {
                format!("Clock: {year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
            }
            Clock::Mbc3 { days, hour, minute, second, halted, carry } => format!(
                "Clock: day {days}, {hour:02}:{minute:02}:{second:02}{}{}",
                if halted { " (stopped)" } else { "" },
                if carry { " (day counter overflowed)" } else { "" }
            ),
        }
    }
}

/// Whether the cartridge should have a clock (from its header / the game database).
pub fn expected(info: &CartInfo) -> bool {
    match info.mode {
        CartMode::Dmg => info.mbc == Mbc::Mbc3 && matches!(info.cart_type(), 0x0F | 0x10),
        CartMode::Agb => crate::bitstream::gba::game_db::lookup(&info.game_code())
            .is_some_and(|config| config.has_rtc),
    }
}

/// Read the cartridge clock.
pub fn read(link: &mut Link, info: &CartInfo) -> anyhow::Result<Clock> {
    match info.mode {
        CartMode::Dmg => mbc3::read(link),
        CartMode::Agb => agb::read(link),
    }
}

/// Set the cartridge clock to `now` (local time). For MBC3 only the time of day is set and the
/// day counter is kept, since games keep their own offset from it.
pub fn set(link: &mut Link, info: &CartInfo, now: OffsetDateTime) -> anyhow::Result<Clock> {
    match info.mode {
        CartMode::Dmg => mbc3::set(link, now),
        CartMode::Agb => agb::set(link, now),
    }
}

fn bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

fn from_bcd(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0F)
}

mod agb {
    use super::*;

    // GPIO port registers in the ROM area.
    const DATA: u32 = 0xC4;
    const DIRECTION: u32 = 0xC6;
    const CONTROL: u32 = 0xC8;

    const CMD_RESET: u8 = 0x60;
    const CMD_WRITE_STATUS: u8 = 0x62;
    const CMD_READ_STATUS: u8 = 0x63;
    const CMD_WRITE_DATE: u8 = 0x64;
    const CMD_READ_DATE: u8 = 0x65;

    const STATUS_POWER_LOST: u8 = 0x80;
    const STATUS_24H: u8 = 0x40;

    fn w(link: &mut Link, address: u32, value: u16) -> anyhow::Result<()> {
        Ok(link.agb_write(address, value)?)
    }

    /// Select the clock (chip select high) and make SIO an output.
    fn begin(link: &mut Link) -> anyhow::Result<()> {
        w(link, CONTROL, 1)?;
        w(link, DATA, 1)?;
        w(link, DATA, 5)?;
        w(link, DIRECTION, 7)
    }

    fn end(link: &mut Link) -> anyhow::Result<()> {
        w(link, DATA, 1)?;
        w(link, DATA, 1)?;
        w(link, CONTROL, 0)
    }

    /// Clock out one bit (SCK low three times, then high).
    fn send_bit(link: &mut Link, bit: u8) -> anyhow::Result<()> {
        let value = 4 | ((bit as u16) << 1);
        for _ in 0..3 {
            w(link, DATA, value)?;
        }
        w(link, DATA, value | 1)
    }

    /// Commands are sent most significant bit first.
    fn command(link: &mut Link, command: u8) -> anyhow::Result<()> {
        (0..8).try_for_each(|i| send_bit(link, (command >> (7 - i)) & 1))
    }

    /// Data is sent least significant bit first.
    fn write_byte(link: &mut Link, data: u8) -> anyhow::Result<()> {
        (0..8).try_for_each(|i| send_bit(link, (data >> i) & 1))
    }

    fn read_byte(link: &mut Link) -> anyhow::Result<u8> {
        let mut data = 0u8;
        let mut buf = [0u8; 2];
        for _ in 0..8 {
            for _ in 0..5 {
                w(link, DATA, 4)?;
            }
            w(link, DATA, 5)?;
            link.agb_read_rom(DATA, &mut buf)?;
            data = (data >> 1) | (((buf[0] >> 1) & 1) << 7);
        }
        Ok(data)
    }

    fn read_status(link: &mut Link) -> anyhow::Result<u8> {
        begin(link)?;
        command(link, CMD_READ_STATUS)?;
        w(link, DIRECTION, 5)?;
        let status = read_byte(link)?;
        end(link)?;
        Ok(status)
    }

    fn write_status(link: &mut Link, status: u8) -> anyhow::Result<()> {
        begin(link)?;
        command(link, CMD_WRITE_STATUS)?;
        write_byte(link, status)?;
        end(link)
    }

    /// Whether a clock answers: with the GPIO port enabled, its registers read differently
    /// from the ROM underneath.
    fn present(link: &mut Link) -> anyhow::Result<bool> {
        let mut rom = [0u8; 6];
        let mut gpio = [0u8; 6];
        link.agb_read_rom(DATA, &mut rom)?;
        w(link, CONTROL, 1)?;
        link.agb_read_rom(DATA, &mut gpio)?;
        w(link, CONTROL, 0)?;
        Ok(rom != gpio)
    }

    pub fn read(link: &mut Link) -> anyhow::Result<Clock> {
        if !present(link)? {
            bail!("No clock answered on this cartridge.");
        }
        let status = read_status(link)?;
        begin(link)?;
        command(link, CMD_READ_DATE)?;
        w(link, DIRECTION, 5)?;
        let mut date = [0u8; 7];
        for byte in date.iter_mut() {
            *byte = read_byte(link)?;
        }
        end(link)?;
        log::info!("Cartridge clock: status {status:#04x}, date {date:02x?}");
        Ok(Clock::Agb {
            year: 2000 + from_bcd(date[0]) as u16,
            month: from_bcd(date[1] & 0x1F),
            day: from_bcd(date[2] & 0x3F),
            hour: from_bcd(date[4] & 0x3F),
            minute: from_bcd(date[5] & 0x7F),
            second: from_bcd(date[6] & 0x7F),
            power_lost: status & STATUS_POWER_LOST != 0,
        })
    }

    pub fn set(link: &mut Link, now: OffsetDateTime) -> anyhow::Result<Clock> {
        if !(2000..=2099).contains(&now.year()) {
            bail!("The Game Bub's clock isn't set (cartridge clocks only go from 2000 to 2099).");
        }
        if !present(link)? {
            bail!("No clock answered on this cartridge.");
        }
        if read_status(link)? & STATUS_POWER_LOST != 0 {
            // After the clock loses power it needs a reset before it keeps time again.
            begin(link)?;
            command(link, CMD_RESET)?;
            end(link)?;
        }
        let hour = bcd(now.hour()) | if now.hour() >= 12 { 0x80 } else { 0 };
        let date = [
            bcd((now.year() - 2000) as u8),
            bcd(now.month() as u8),
            bcd(now.day()),
            bcd(now.weekday().number_days_from_sunday()),
            hour,
            bcd(now.minute()),
            bcd(now.second()),
        ];
        begin(link)?;
        command(link, CMD_WRITE_DATE)?;
        for byte in date {
            write_byte(link, byte)?;
        }
        end(link)?;
        write_status(link, STATUS_24H)?;
        read(link)
    }
}

mod mbc3 {
    use super::*;

    /// Select a clock register (0x08 seconds ... 0x0C day high / flags) in the RAM area.
    fn select(link: &mut Link, register: u8) -> anyhow::Result<()> {
        Ok(link.dmg_write(0x4000, register)?)
    }

    /// Copy the running clock into the readable registers.
    fn latch(link: &mut Link) -> anyhow::Result<()> {
        link.dmg_write(0x6000, 0)?;
        Ok(link.dmg_write(0x6000, 1)?)
    }

    fn finish(link: &mut Link) -> anyhow::Result<()> {
        select(link, 0)?;
        Ok(link.dmg_write(0x0000, 0x00)?)
    }

    pub fn read(link: &mut Link) -> anyhow::Result<Clock> {
        link.dmg_write(0x0000, 0x0A)?;
        latch(link)?;
        let mut regs = [0u8; 5];
        for (i, reg) in regs.iter_mut().enumerate() {
            select(link, 0x08 + i as u8)?;
            let mut b = [0u8];
            link.dmg_read_ram(0, &mut b)?;
            *reg = b[0];
        }
        finish(link)?;
        log::info!("Cartridge clock registers: {regs:02x?}");
        if regs.iter().all(|&r| r == 0xFF) {
            bail!("The clock didn't answer.");
        }
        Ok(Clock::Mbc3 {
            second: regs[0] & 0x3F,
            minute: regs[1] & 0x3F,
            hour: regs[2] & 0x1F,
            days: regs[3] as u16 | ((regs[4] as u16 & 1) << 8),
            halted: regs[4] & 0x40 != 0,
            carry: regs[4] & 0x80 != 0,
        })
    }

    pub fn set(link: &mut Link, now: OffsetDateTime) -> anyhow::Result<Clock> {
        let Clock::Mbc3 { days, .. } = read(link)? else { unreachable!() };
        link.dmg_write(0x0000, 0x0A)?;
        latch(link)?;
        // Stop the clock while writing, then write the registers; clearing the halt bit last
        // starts it again.
        select(link, 0x0C)?;
        link.dmg_write_ram(0, &[0x40])?;
        let regs = [now.second(), now.minute(), now.hour(), (days & 0xFF) as u8, (days >> 8) as u8 & 1];
        for (i, value) in regs.into_iter().enumerate() {
            select(link, 0x08 + i as u8)?;
            link.dmg_write_ram(0, &[value])?;
        }
        latch(link)?;
        finish(link)?;
        read(link)
    }
}
