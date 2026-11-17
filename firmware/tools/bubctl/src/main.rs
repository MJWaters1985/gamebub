//! bubctl: control a Game Bub over USB from a computer (macOS/Linux), through the firmware's
//! vendor control interface (handheld/src/control/mod.rs) and its USB serial log.
//!
//!   bubctl info                         serial number, firmware version and commit
//!   bubctl reboot                       restart
//!   bubctl press <buttons>... [--hold MS] [--gap MS]
//!                                       press buttons in turn; combine with +, e.g. home a+b;
//!                                       wait:MS pauses. Buttons: a b x y up down left right
//!                                       home select start l r
//!   bubctl screenshot <out.png>         the menus (not the game's picture)
//!   bubctl log <seconds> [--until TEXT] the USB serial log (read only)
//!   bubctl install <file.uf2>           install firmware: copied to the GAME BUB drive if
//!                                       update mode is on, else flashed over USB through the
//!                                       update firmware's protocol (restarting into it if the
//!                                       running firmware says that's safe), then verified
//!
//! Restarting into update mode over USB needs firmware that latches the FPGA power, DAC reset
//! and cartridge power pins off first (info flag bit 0): without that the update firmware
//! leaves them floating, and the vibration motor runs and the amplifier whines.
//!
//! The gamepad uses the quiet slot (no "Gamepad connected" notification).

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use nusb::transfer::{Control, ControlType, Recipient};

mod dfu;

const PRODUCT: &str = "Game Bub Handheld";
const GET_INFO: u8 = 0;
const REBOOT: u8 = 1;
const GAMEPAD_CONNECT: u8 = 4;
const GAMEPAD_DISCONNECT: u8 = 5;
const GAMEPAD_DATA: u8 = 6;
const SCREENSHOT: u8 = 7;
const SCREENSHOT_READ: u8 = 9;
const SLOT: u32 = 0xB0B;
const TIMEOUT: Duration = Duration::from_secs(2);
const UPDATE_VOLUME: &str = "/Volumes/GAME BUB";

type Result<T> = std::result::Result<T, String>;

struct Bub {
    dev: nusb::Device,
    serial: String,
}

fn control(request: u8, value: u16) -> Control {
    Control { control_type: ControlType::Vendor, recipient: Recipient::Interface, request, value, index: 0 }
}

impl Bub {
    fn find() -> Result<Option<Bub>> {
        let devices = nusb::list_devices().map_err(|e| format!("Couldn't list USB devices: {e}"))?;
        for d in devices {
            if d.product_string() == Some(PRODUCT) {
                let serial = d.serial_number().unwrap_or_default().to_string();
                let dev = d.open().map_err(|e| format!("Couldn't open the Game Bub: {e}"))?;
                return Ok(Some(Bub { dev, serial }));
            }
        }
        Ok(None)
    }

    fn open() -> Result<Bub> {
        Bub::find()?.ok_or_else(|| "No Game Bub found on USB (is it on, and not in update mode?)".into())
    }

    fn out(&self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        self.dev
            .control_out_blocking(control(request, value), data, TIMEOUT)
            .map(|_| ())
            .map_err(|e| format!("USB request {request} failed: {e}"))
    }

    fn input(&self, request: u8, value: u16, buf: &mut [u8]) -> std::result::Result<usize, String> {
        self.dev
            .control_in_blocking(control(request, value), buf, TIMEOUT)
            .map_err(|e| format!("USB request {request} failed: {e}"))
    }

    /// Flags (bit 0: restarting into update mode over USB is safe), serial, version, commit.
    fn info_flags(&self) -> Result<(u32, u32, String, String)> {
        let mut b = [0u8; 24];
        let n = self.input(GET_INFO, 0, &mut b)?;
        if n < 24 {
            return Err("Short info reply".into());
        }
        let flags = u32::from_le_bytes(b[0..4].try_into().unwrap());
        let (serial, version, commit) = Self::parse_info(&b);
        Ok((flags, serial, version, commit))
    }

    fn info(&self) -> Result<(u32, String, String)> {
        self.info_flags().map(|(_, s, v, c)| (s, v, c))
    }

    fn parse_info(b: &[u8; 24]) -> (u32, String, String) {
        let serial = u32::from_le_bytes(b[4..8].try_into().unwrap());
        let version = format!("{}.{}.{}", b[15], b[14], b[13]);
        let commit: String = b[16..24].iter().map(|x| format!("{x:02x}")).collect();
        (serial, version, commit)
    }
}

fn button_bits(name: &str) -> Result<[u8; 3]> {
    let mut b = [0u8; 3];
    for part in name.split('+') {
        match part {
            "a" => b[0] |= 0x01,
            "b" => b[0] |= 0x02,
            "x" => b[0] |= 0x04,
            "y" => b[0] |= 0x08,
            "up" => b[0] |= 0x10,
            "down" => b[0] |= 0x20,
            "right" => b[0] |= 0x40,
            "left" => b[0] |= 0x80,
            "home" => b[1] |= 0x01,
            "select" => b[1] |= 0x02,
            "start" => b[1] |= 0x04,
            "l" => b[1] |= 0x10,
            "r" => b[1] |= 0x20,
            other => return Err(format!("Unknown button '{other}'")),
        }
    }
    Ok(b)
}

fn gamepad_data(bub: &Bub, bits: [u8; 3]) -> Result<()> {
    let mut d = [0u8; 20];
    d[0..4].copy_from_slice(&SLOT.to_le_bytes());
    d[4..7].copy_from_slice(&bits);
    bub.out(GAMEPAD_DATA, 0, &d)
}

fn press(args: &[String]) -> Result<()> {
    let mut hold = 120u64;
    let mut gap = 250u64;
    let mut steps = vec![];
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--hold" => {
                i += 1;
                hold = args.get(i).and_then(|v| v.parse().ok()).ok_or("--hold needs milliseconds")?;
            }
            "--gap" => {
                i += 1;
                gap = args.get(i).and_then(|v| v.parse().ok()).ok_or("--gap needs milliseconds")?;
            }
            s => steps.push(s.to_string()),
        }
        i += 1;
    }
    let bub = Bub::open()?;
    let mut c = [0u8; 48];
    c[0..4].copy_from_slice(&SLOT.to_le_bytes());
    c[16..22].copy_from_slice(b"bubctl");
    bub.out(GAMEPAD_CONNECT, 0, &c)?;
    std::thread::sleep(Duration::from_millis(50));
    let result = (|| {
        for s in &steps {
            if let Some(ms) = s.strip_prefix("wait:") {
                std::thread::sleep(Duration::from_millis(ms.parse().map_err(|_| "wait:MS needs a number")?));
                continue;
            }
            gamepad_data(&bub, button_bits(s)?)?;
            std::thread::sleep(Duration::from_millis(hold));
            gamepad_data(&bub, [0; 3])?;
            std::thread::sleep(Duration::from_millis(gap));
        }
        Ok(())
    })();
    let _ = gamepad_data(&bub, [0; 3]);
    let _ = bub.out(GAMEPAD_DISCONNECT, 0, &SLOT.to_le_bytes());
    result
}

fn screenshot(out: &str) -> Result<()> {
    let bub = Bub::open()?;
    let mut data = Vec::new();
    let mut buf = [0u8; 512];
    // The first piece is refused until the screenshot is saved, and for good when the Game Bub
    // refused to take it (short of memory, e.g. with box art showing): ask again a few times.
    'attempts: for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(1000));
        }
        bub.out(SCREENSHOT, 1, &[])?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(n) = bub.input(SCREENSHOT_READ, 0, &mut buf) {
                data.extend_from_slice(&buf[..n]);
                break 'attempts;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if data.is_empty() {
        return Err("No screenshot (the Game Bub refused it, probably short of memory)".into());
    }
    if data.len() < 18 {
        return Err("Screenshot too short".into());
    }
    let w = u16::from_le_bytes([data[12], data[13]]) as usize;
    let h = u16::from_le_bytes([data[14], data[15]]) as usize;
    let total = 18 + w * h * 2;
    let mut piece = 1u16;
    while data.len() < total {
        let n = bub.input(SCREENSHOT_READ, piece, &mut buf)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        piece += 1;
    }
    if data.len() < total {
        return Err(format!("Screenshot cut short ({} of {} bytes)", data.len(), total));
    }
    // ARGB1555, top-left origin -> RGB8
    let mut rgb = Vec::with_capacity(w * h * 3);
    for p in data[18..total].chunks(2) {
        let v = u16::from_le_bytes([p[0], p[1]]);
        for shift in [10, 5, 0] {
            let c = ((v >> shift) & 0x1F) as u8;
            rgb.push((c << 3) | (c >> 2));
        }
    }
    let file = std::fs::File::create(out).map_err(|e| format!("{out}: {e}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(&rgb))
        .map_err(|e| format!("{out}: {e}"))?;
    println!("{out}: {w}x{h}");
    Ok(())
}

/// The serial log port: /dev/cu.usbmodem<serial>* on macOS.
fn log_port(serial: &str) -> Option<std::path::PathBuf> {
    let mut ports: Vec<_> = std::fs::read_dir("/dev")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            (n.starts_with("cu.usbmodem") && n.contains(serial)) || n.starts_with("ttyACM")
        })
        .collect();
    ports.sort();
    ports.into_iter().next()
}

fn log(args: &[String]) -> Result<()> {
    let secs: f64 = args.first().and_then(|v| v.parse().ok()).ok_or("log needs seconds")?;
    let until = args.iter().position(|a| a == "--until").and_then(|i| args.get(i + 1)).cloned();
    let serial = Bub::open()?.serial;
    let end = Instant::now() + Duration::from_secs_f64(secs);
    let mut port: Option<std::fs::File> = None;
    let mut line = String::new();
    let stdout = std::io::stdout();
    while Instant::now() < end {
        if port.is_none() {
            use std::os::unix::fs::OpenOptionsExt;
            port = log_port(&serial).and_then(|p| {
                std::fs::OpenOptions::new().read(true).custom_flags(0x0004 /* O_NONBLOCK */).open(p).ok()
            });
            if port.is_none() {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        }
        let mut buf = [0u8; 1024];
        match port.as_mut().unwrap().read(&mut buf) {
            Ok(0) => {
                port = None;
            }
            Ok(n) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                let mut out = stdout.lock();
                let _ = out.write_all(text.as_bytes());
                let _ = out.flush();
                line.push_str(&text);
                if let Some(u) = &until {
                    if line.contains(u.as_str()) {
                        return Ok(());
                    }
                }
                if let Some(i) = line.rfind('\n') {
                    line.drain(..=i);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => {
                port = None;
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    if until.is_some() {
        return Err("Timed out waiting for the log text".into());
    }
    Ok(())
}

fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!("Timed out waiting for {what}"))
}

fn install(uf2: &str) -> Result<()> {
    let path = std::path::Path::new(uf2);
    let file = std::fs::read(path).map_err(|e| format!("{uf2}: {e}"))?;
    let expected_commit;
    if std::path::Path::new(UPDATE_VOLUME).is_dir() {
        // Update mode entered with the buttons: copy to the drive.
        expected_commit = None;
        let dest = std::path::Path::new(UPDATE_VOLUME).join(path.file_name().unwrap());
        println!("Copying {} ...", path.display());
        // The drive goes away as soon as the copy completes, so errors closing it are expected.
        let status = std::process::Command::new("cp").arg("-X").arg(path).arg(&dest).status();
        if !matches!(status, Ok(s) if s.success()) && std::path::Path::new(UPDATE_VOLUME).is_dir() {
            return Err("Copying the firmware failed".into());
        }
    } else {
        let mut d = match dfu::Dfu::find()? {
            Some(d) => d,
            None => {
                let bub = Bub::open()?;
                let (flags, ..) = bub.info_flags()?;
                if flags & 1 == 0 {
                    return Err("The running firmware can't restart into update mode safely over USB \
                                (the vibration motor and amplifier would be left on): enter update \
                                mode with the buttons"
                        .into());
                }
                bub.out(REBOOT, 2, &[])?;
                println!("Restarting into update mode...");
                let mut found = None;
                wait_for("the update firmware", 30, || {
                    found = dfu::Dfu::find().ok().flatten();
                    found.is_some()
                })?;
                found.unwrap()
            }
        };
        let (runs, commit) = dfu::read_uf2(&file, d.hardware()?)?;
        expected_commit = commit;
        let size: usize = runs.iter().map(|r| r.1.len()).sum();
        println!("Flashing {} ({} KB) ...", path.display(), size / 1024);
        d.flash(&runs)?;
        d.reboot()?;
    }
    println!("Installing...");
    wait_for("the Game Bub to restart", 120, || Bub::find().ok().flatten().is_some())?;
    std::thread::sleep(Duration::from_secs(2));
    let (_, version, commit) = Bub::open()?.info()?;
    println!("Running {version} ({})", &commit[..7]);
    if let Some(want) = expected_commit {
        if !want.starts_with(&commit[..7]) {
            return Err(format!("Expected commit {}, the Game Bub runs {}", &want[..7.min(want.len())], &commit[..7]));
        }
    }
    Ok(())
}

fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("info") => {
            let bub = Bub::open()?;
            let (serial, version, commit) = bub.info()?;
            println!("serial {serial:08X}, firmware {version}, commit {}", &commit[..7]);
            Ok(())
        }
        Some("reboot") => Bub::open()?.out(REBOOT, 1, &[]),
        Some("press") => press(&args[1..]),
        Some("screenshot") => screenshot(args.get(1).ok_or("screenshot needs an output file")?),
        Some("log") => log(&args[1..]),
        Some("install") => install(args.get(1).ok_or("install needs a .uf2 file")?),
        Some("uf2-info") => {
            let f = args.get(1).ok_or("uf2-info needs a .uf2 file")?;
            let data = std::fs::read(f).map_err(|e| format!("{f}: {e}"))?;
            // Hardware 1.4.1.0 unless given (product.major.minor.variant)
            let hw = args.get(2).map_or(Ok(0x0104_0100), |v| {
                let p: Vec<u32> = v.split('.').filter_map(|x| x.parse().ok()).collect();
                if p.len() == 4 { Ok(p[0] << 24 | p[1] << 16 | p[2] << 8 | p[3]) } else { Err("hardware as 1.4.1.0".to_string()) }
            })?;
            let (runs, commit) = dfu::read_uf2(&data, hw)?;
            for (start, bytes) in &runs {
                println!("{start:#010x} .. {:#010x}  {} KB", start + bytes.len() as u32, bytes.len() / 1024);
            }
            println!("commit {}", commit.unwrap_or_else(|| "?".into()));
            Ok(())
        }
        _ => Err("Usage: bubctl info | reboot | press <buttons>... [--hold MS] [--gap MS] | \
                  screenshot <out.png> | log <seconds> [--until TEXT] | install <file.uf2> | uf2-info <file.uf2> [hw]"
            .into()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("bubctl: {e}");
        std::process::exit(1);
    }
}
