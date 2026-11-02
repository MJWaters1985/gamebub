//! Cartridge Manager: identify the inserted cartridge, dump its ROM to the SD card, and copy
//! its save to or from the SD card, all on the device.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

use cart::{CartInfo, SaveKind};
use link::Link;

use crate::ui;

mod cart;
mod clock;
mod db;
mod link;

/// Requests from the UI.
#[derive(Debug)]
pub enum Request {
    /// Run the menu action at this index.
    Action(usize),
    /// Go ahead with the action that asked for confirmation (`Update::Confirm`).
    Confirm,
    /// Power off the cartridge and stop.
    Exit,
}

/// Updates for the UI.
#[derive(Debug, Clone)]
pub enum Update {
    /// An operation is running. Progress is 0-1, or negative if unknown.
    Busy { label: String, progress: f32 },
    /// Cartridge details and the actions available.
    Menu { title: String, details: String, actions: Vec<String> },
    /// Ask before overwriting a save (hold A to go ahead).
    Confirm { title: String, text: String },
    /// An operation finished.
    Result { ok: bool, message: String },
}

#[derive(Copy, Clone, Debug)]
enum Action {
    DumpRom,
    SaveToSd,
    SaveToCart,
    SetClock,
    ReadAgain,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::DumpRom => "Dump ROM to SD card",
            Self::SaveToSd => "Save: Cartridge to SD Card",
            Self::SaveToCart => "Save: SD Card to Cartridge",
            Self::SetClock => "Set cartridge clock to now",
            Self::ReadAgain => "Read cartridge again",
        }
    }
}

static SENDER: Mutex<Option<Sender<Request>>> = Mutex::new(None);

/// Start the Cartridge Manager (if it isn't running).
pub fn start() {
    let mut sender = SENDER.lock().unwrap();
    if sender.is_some() {
        return;
    }
    let (tx, rx) = channel();
    *sender = Some(tx);
    crate::jobs::run(move || {
        Worker::new().run(rx);
        *SENDER.lock().unwrap() = None;
    });
}

/// Send a request to the Cartridge Manager.
pub fn request(request: Request) {
    if let Some(sender) = SENDER.lock().unwrap().as_ref() {
        let _ = sender.send(request);
    }
}

fn send(update: Update) {
    ui::send(ui::Message::CartTool(update));
}

fn busy(label: &str, progress: f32) {
    send(Update::Busy { label: label.to_string(), progress });
}

struct Worker {
    link: Option<Link>,
    info: Option<CartInfo>,
    actions: Vec<Action>,
    /// Cartridge clock, if the cartridge has one (or an error reading it).
    clock: Option<Result<clock::Clock, String>>,
    /// This cartridge's ROM on the SD card, if there is one.
    rom: Option<std::path::PathBuf>,
    /// Action waiting for the user's confirmation.
    pending: Option<Action>,
}

impl Worker {
    fn new() -> Self {
        Worker { link: None, info: None, actions: vec![], clock: None, rom: None, pending: None }
    }

    fn run(mut self, rx: Receiver<Request>) {
        self.identify();
        while let Ok(request) = rx.recv() {
            match request {
                Request::Exit => break,
                Request::Action(i) => match self.actions.get(i).copied() {
                    Some(Action::DumpRom) => self.dump_rom(),
                    Some(Action::SaveToSd) => self.save_to_sd(false),
                    Some(Action::SaveToCart) => self.save_to_cart(false),
                    Some(Action::SetClock) => self.set_clock(),
                    Some(Action::ReadAgain) => self.identify(),
                    None => self.show_menu(),
                },
                Request::Confirm => self.confirmed(),
            }
        }
        if let Some(link) = self.link.take() {
            link.close();
        }
    }

    fn fail(&mut self, error: anyhow::Error) {
        log::error!("Cartridge Manager: {error:#}");
        send(Update::Result { ok: false, message: format!("{error:#}") });
    }

    fn identify(&mut self) {
        busy("Reading cartridge...", -1.0);
        self.info = None;
        self.clock = None;
        self.rom = None;
        if self.link.is_none() {
            match Link::open() {
                Ok(link) => self.link = Some(link),
                Err(e) => return self.fail(e.context("Couldn't start the cartridge reader")),
            }
        }
        match self.read_cartridge() {
            Ok(()) => self.show_menu(),
            Err(e) => {
                self.actions = vec![Action::ReadAgain];
                send(Update::Menu {
                    title: "No cartridge found".into(),
                    details: format!("{e:#}"),
                    actions: self.actions.iter().map(|a| a.label().to_string()).collect(),
                });
            }
        }
    }

    /// Identify the inserted cartridge (the link must be open).
    fn read_cartridge(&mut self) -> anyhow::Result<()> {
        self.info = None;
        self.clock = None;
        self.rom = None;
        let link = self.link.as_mut().unwrap();
        // Power cycle so the cartridge starts from a clean state (e.g. after a swap).
        let _ = link.power_cycle();
        let info = cart::identify(link)?;
        log::info!("Cartridge: {info:?}");
        if clock::expected(&info) {
            busy("Reading the cartridge clock...", -1.0);
            self.clock = Some(clock::read(link, &info).map_err(|e| format!("{e:#}")));
        }
        busy("Looking for the ROM on the SD card...", -1.0);
        self.rom = info.find_rom_on_sd();
        self.info = Some(info);
        Ok(())
    }

    /// Check that the cartridge hasn't been swapped since it was read. If it has, read the new
    /// one, tell the user nothing was done, and return false.
    fn check_cartridge(&mut self) -> bool {
        let (Some(link), Some(info)) = (self.link.as_mut(), self.info.as_ref()) else {
            self.identify();
            return false;
        };
        busy("Checking the cartridge...", -1.0);
        if cart::is_same_cartridge(link, info) {
            return true;
        }
        log::warn!("Cartridge Manager: the cartridge changed since it was read");
        let now = match self.read_cartridge() {
            Ok(()) => format!("Now inserted: {}", self.info.as_ref().unwrap().name()),
            Err(e) => format!("No cartridge could be read now: {e:#}"),
        };
        send(Update::Result {
            ok: false,
            message: format!(
                "The cartridge was changed since it was read, so nothing was done.\n\n{now}\n\nCheck the details and choose again."
            ),
        });
        false
    }

    fn show_menu(&mut self) {
        let Some(info) = &self.info else {
            return self.identify();
        };
        let mut summary = format!("{}, {} ROM", info.system.folder(), format_size(info.rom_size));
        if info.mode == link::CartMode::Dmg {
            summary += &format!(", {}", info.mbc.name());
        }
        let mut details = vec![summary];
        if info.db.is_none() {
            details.push(format!("Not in the database (header title \"{}\")", info.header_title));
        }
        details.push(match (info.save, info.save_guessed) {
            (SaveKind::None, true) => "Save: detected when needed".to_string(),
            (save, _) => format!("Save: {}", save.describe()),
        });
        match &self.clock {
            Some(Ok(clock)) => details.push(clock.describe()),
            Some(Err(e)) => details.push(format!("Clock: couldn't be read ({e})")),
            None => {}
        }
        details.push(match &self.rom {
            // Only name the file when it isn't the name Dump ROM uses.
            Some(rom) if *rom == info.rom_path() => "ROM on SD card".to_string(),
            Some(rom) => format!("ROM on SD card: {}", file_name(rom)),
            None => "No matching ROM on SD card".to_string(),
        });

        self.actions = vec![Action::DumpRom];
        if info.has_save() || info.save_guessed {
            self.actions.push(Action::SaveToSd);
            self.actions.push(Action::SaveToCart);
        }
        if self.clock.is_some() {
            self.actions.push(Action::SetClock);
        }
        self.actions.push(Action::ReadAgain);
        send(Update::Menu {
            title: info.name(),
            details: details.join("\n"),
            actions: self.actions.iter().map(|a| a.label().to_string()).collect(),
        });
    }

    fn dump_rom(&mut self) {
        if !self.check_cartridge() {
            return;
        }
        let (Some(link), Some(info)) = (self.link.as_mut(), self.info.as_mut()) else {
            return self.identify();
        };
        busy("Dumping ROM...", 0.0);
        match cart::dump_rom(link, info, &mut |p| busy("Dumping ROM...", p)) {
            Ok(result) => {
                if let Some(save) = result.detected_save {
                    info.save = save;
                    info.save_guessed = false;
                }
                self.rom = Some(result.path.clone());
                let check = match result.verified {
                    Some(true) => "Verified: matches a known good dump.",
                    Some(false) if info.db.is_some() => {
                        "Warning: doesn't match the known good dump.\nClean the contacts and dump again."
                    }
                    Some(false) => "Warning: the ROM checksum is wrong.\nClean the contacts and dump again.",
                    None => "Not in the database, so it couldn't be verified.",
                };
                let boxart = fetch_boxart(info);
                send(Update::Result {
                    ok: result.verified != Some(false),
                    message: format!(
                        "Saved {}\n\nCRC32: {:08x}\n{}{}",
                        display_path(&result.path),
                        result.crc32,
                        check,
                        boxart
                    ),
                });
            }
            Err(e) => self.fail(e),
        }
    }

    /// Make sure the save type of an unknown GBA cartridge is known.
    fn ensure_save_type(&mut self) -> anyhow::Result<()> {
        let (Some(link), Some(info)) = (self.link.as_mut(), self.info.as_mut()) else {
            anyhow::bail!("No cartridge");
        };
        if info.save_guessed {
            busy("Detecting save type...", 0.0);
            info.save = cart::detect_agb_save(link, info, &mut |p| busy("Detecting save type...", p))?;
            info.save_guessed = false;
        }
        if !info.has_save() {
            anyhow::bail!("This cartridge has no save memory.");
        }
        Ok(())
    }

    /// The save file on the SD card for this cartridge: the matching ROM's save, or the save
    /// of the ROM Dump ROM would create.
    fn sd_save_path(&self) -> Option<std::path::PathBuf> {
        let info = self.info.as_ref()?;
        let rom = self.rom.clone().unwrap_or_else(|| info.rom_path());
        Some(crate::core::save_path_for_rom(&rom))
    }

    /// Copy the cartridge save to the SD card, asking first if that replaces a save there.
    fn save_to_sd(&mut self, confirmed: bool) {
        if !self.check_cartridge() {
            return;
        }
        if let Err(e) = self.ensure_save_type() {
            return self.fail(e);
        }
        let path = self.sd_save_path().unwrap();
        if !confirmed && path.is_file() {
            self.pending = Some(Action::SaveToSd);
            return send(Update::Confirm {
                title: "Replace the save on the SD card?".into(),
                text: format!(
                    "{}\nis replaced by the cartridge's save.\nThe old one is kept as .bak.",
                    file_name(&path)
                ),
            });
        }
        let (link, info) = (self.link.as_mut().unwrap(), self.info.as_ref().unwrap());
        busy("Copying save to the SD card...", 0.0);
        match cart::copy_save_to_sd(link, info, &path, &mut |p| busy("Copying save to the SD card...", p)) {
            Ok(()) => send(Update::Result {
                ok: true,
                message: format!("Save copied to the SD card.\n\n{}", display_path(&path)),
            }),
            Err(e) => self.fail(e),
        }
    }

    /// Copy the SD card's save to the cartridge, asking first unless the cartridge's save memory
    /// is empty.
    fn save_to_cart(&mut self, confirmed: bool) {
        if !self.check_cartridge() {
            return;
        }
        if let Err(e) = self.ensure_save_type() {
            return self.fail(e);
        }
        let path = self.sd_save_path().unwrap();
        if !path.is_file() {
            return send(Update::Result {
                ok: false,
                message: format!(
                    "There's no save for this game on the SD card yet, so nothing was written.\n\n{}",
                    display_path(&path)
                ),
            });
        }
        let (link, info) = (self.link.as_mut().unwrap(), self.info.as_ref().unwrap());
        if !confirmed {
            busy("Checking the cartridge save...", 0.0);
            match cart::save_is_blank(link, info, &mut |p| busy("Checking the cartridge save...", p)) {
                Ok(true) => {}
                Ok(false) => {
                    self.pending = Some(Action::SaveToCart);
                    return send(Update::Confirm {
                        title: "Overwrite the save on the cartridge?".into(),
                        text: format!(
                            "From the SD card: {}\nThe cartridge's current save is lost.",
                            file_name(&path)
                        ),
                    });
                }
                Err(e) => return self.fail(e),
            }
        }
        busy("Writing save to the cartridge...", 0.0);
        match cart::write_save(link, info, info.save, &path, &mut |p| {
            busy("Writing save to the cartridge...", p)
        }) {
            Ok(()) => send(Update::Result {
                ok: true,
                message: format!(
                    "Save written to the cartridge and verified.\n\nFrom: {}",
                    display_path(&path)
                ),
            }),
            Err(e) => self.fail(e),
        }
    }

    /// Go ahead with the action the user confirmed.
    fn confirmed(&mut self) {
        match self.pending.take() {
            Some(Action::SaveToSd) => self.save_to_sd(true),
            Some(Action::SaveToCart) => self.save_to_cart(true),
            _ => self.show_menu(),
        }
    }
}

impl Worker {
    fn set_clock(&mut self) {
        if !self.check_cartridge() {
            return;
        }
        let (Some(link), Some(info)) = (self.link.as_mut(), self.info.as_ref()) else {
            return self.identify();
        };
        let result = (|| -> anyhow::Result<clock::Clock> {
            busy("Setting the cartridge clock...", -1.0);
            let now = crate::device::Device::lock().get_local_datetime();
            clock::set(link, info, now)
        })();
        match result {
            Ok(clock) => {
                let mut message = format!("Cartridge clock set.\n{}", clock.describe());
                if matches!(clock, clock::Clock::Mbc3 { .. }) {
                    message += "\n\nGame Boy clocks count days, not dates: the game keeps its own offset, \
                        so its in-game clock may still need setting in the game.";
                }
                self.clock = Some(Ok(clock));
                send(Update::Result { ok: true, message });
            }
            Err(e) => self.fail(e),
        }
    }
}

/// CRC32 of the known good dump of a ROM, looked up by its header.
pub fn known_crc32(gba: bool, header: &[u8]) -> Option<u32> {
    let mode = if gba { link::CartMode::Agb } else { link::CartMode::Dmg };
    db::lookup(mode, header).map(|entry| entry.crc32)
}

/// Download box art for a dumped cartridge, if enabled. Returns a note for the result.
fn fetch_boxart(info: &CartInfo) -> String {
    if !crate::kvs::keys::BOXART_AFTER_DUMP.get().unwrap_or(true) || crate::net::saved_network().is_none() {
        return String::new();
    }
    let Some(system) = crate::library::SYSTEMS.iter().find(|s| s.folder == info.system.folder()) else {
        return String::new();
    };
    let name = info.name();
    if system.thumbnail_path(&name).exists() {
        return String::new();
    }
    match crate::boxart::fetch(&[(*system, name)], &mut |label, p| busy(label, p)) {
        Ok(r) if r.found > 0 => "\nBox art downloaded.".into(),
        Ok(r) if r.not_found > 0 => "\nNo box art found for this game.".into(),
        Ok(_) => "\nThe box art couldn't be downloaded.".into(),
        Err(e) => format!("\nBox art: {e:#}"),
    }
}

fn format_size(bytes: u32) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} MiB", bytes / (1024 * 1024))
    } else {
        format!("{} KiB", bytes / 1024)
    }
}

/// Path as the user sees it on the SD card.
fn file_name(path: &std::path::Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn display_path(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    s.strip_prefix("/sdcard").unwrap_or(&s).to_string()
}
