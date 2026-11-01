//! Cartridge Manager: identify the inserted cartridge, dump its ROM to the SD card, and back
//! up / restore its save, all on the device.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

use cart::{CartInfo, SaveFile, SaveKind};
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
    /// Restore the save at this index of the last save list.
    Restore(usize),
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
    /// Saves that can be restored.
    Saves(Vec<String>),
    /// An operation finished.
    Result { ok: bool, message: String },
}

#[derive(Copy, Clone, Debug)]
enum Action {
    DumpRom,
    BackupSave,
    RestoreSave,
    SetClock,
    ReadAgain,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::DumpRom => "Dump ROM to SD card",
            Self::BackupSave => "Back up save",
            Self::RestoreSave => "Restore save to cartridge",
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
    saves: Vec<SaveFile>,
    /// Cartridge clock, if the cartridge has one (or an error reading it).
    clock: Option<Result<clock::Clock, String>>,
}

impl Worker {
    fn new() -> Self {
        Worker { link: None, info: None, actions: vec![], saves: vec![], clock: None }
    }

    fn run(mut self, rx: Receiver<Request>) {
        self.identify();
        while let Ok(request) = rx.recv() {
            match request {
                Request::Exit => break,
                Request::Action(i) => match self.actions.get(i).copied() {
                    Some(Action::DumpRom) => self.dump_rom(),
                    Some(Action::BackupSave) => self.backup_save(),
                    Some(Action::RestoreSave) => self.list_saves(),
                    Some(Action::SetClock) => self.set_clock(),
                    Some(Action::ReadAgain) => self.identify(),
                    None => self.show_menu(),
                },
                Request::Restore(i) => self.restore_save(i),
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
        if self.link.is_none() {
            match Link::open() {
                Ok(link) => self.link = Some(link),
                Err(e) => return self.fail(e.context("Couldn't start the cartridge reader")),
            }
        }
        let link = self.link.as_mut().unwrap();
        // Power cycle so the cartridge starts from a clean state (e.g. after a swap).
        let _ = link.power_cycle();
        match cart::identify(link) {
            Ok(info) => {
                log::info!("Cartridge: {info:?}");
                if clock::expected(&info) {
                    busy("Reading the cartridge clock...", -1.0);
                    self.clock = Some(clock::read(link, &info).map_err(|e| format!("{e:#}")));
                }
                self.info = Some(info);
                self.show_menu();
            }
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
        let rom_path = info.rom_path();
        if rom_path.exists() {
            details.push("ROM already on SD card".to_string());
        }

        self.actions = vec![Action::DumpRom];
        if info.has_save() || info.save_guessed {
            self.actions.push(Action::BackupSave);
            self.actions.push(Action::RestoreSave);
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

    fn backup_save(&mut self) {
        if let Err(e) = self.ensure_save_type() {
            return self.fail(e);
        }
        let (link, info) = (self.link.as_mut().unwrap(), self.info.as_ref().unwrap());
        busy("Backing up save...", 0.0);
        match cart::backup_save(link, info, "", true, &mut |p| busy("Backing up save...", p)) {
            Ok((backup, live)) => send(Update::Result {
                ok: true,
                message: format!(
                    "Save backed up.\n\nGame save: {}\nCopy: {}",
                    live.as_deref().map(display_path).unwrap_or_default(),
                    display_path(&backup)
                ),
            }),
            Err(e) => self.fail(e),
        }
    }

    fn list_saves(&mut self) {
        if let Err(e) = self.ensure_save_type() {
            return self.fail(e);
        }
        self.saves = cart::list_saves(self.info.as_ref().unwrap());
        if self.saves.is_empty() {
            send(Update::Result {
                ok: false,
                message: "There are no saves for this game on the SD card yet.\nBack up the save first."
                    .into(),
            });
        } else {
            send(Update::Saves(self.saves.iter().map(|s| s.label.clone()).collect()));
        }
    }

    fn restore_save(&mut self, index: usize) {
        let (Some(link), Some(info)) = (self.link.as_mut(), self.info.as_ref()) else {
            return self.identify();
        };
        let Some(save) = self.saves.get(index).cloned() else {
            return self.show_menu();
        };
        let result = (|| -> anyhow::Result<std::path::PathBuf> {
            // Keep what's on the cartridge now, in case this was a mistake.
            busy("Backing up the cartridge save...", 0.0);
            let (backup, _) = cart::backup_save(link, info, "(before restore)", false, &mut |p| {
                busy("Backing up the cartridge save...", p)
            })?;
            busy("Restoring save...", 0.0);
            cart::write_save(link, info, info.save, &save.path, &mut |p| busy("Restoring save...", p))?;
            Ok(backup)
        })();
        match result {
            Ok(backup) => send(Update::Result {
                ok: true,
                message: format!(
                    "Save restored and verified.\n\nThe cartridge's previous save was kept as\n{}",
                    display_path(&backup)
                ),
            }),
            Err(e) => self.fail(e),
        }
    }
}

impl Worker {
    fn set_clock(&mut self) {
        let (Some(link), Some(info)) = (self.link.as_mut(), self.info.as_ref()) else {
            return self.identify();
        };
        let result = (|| -> anyhow::Result<(clock::Clock, Option<std::path::PathBuf>)> {
            // The MBC3 clock is written through the save RAM area: keep a copy of the save in
            // case a write goes astray.
            let backup = if info.mode == link::CartMode::Dmg && info.has_save() {
                busy("Backing up the save...", 0.0);
                let (backup, _) = cart::backup_save(link, info, "(before clock set)", false, &mut |p| {
                    busy("Backing up the save...", p)
                })?;
                Some(backup)
            } else {
                None
            };
            busy("Setting the cartridge clock...", -1.0);
            let now = crate::device::Device::lock().get_local_datetime();
            Ok((clock::set(link, info, now)?, backup))
        })();
        match result {
            Ok((clock, backup)) => {
                let mut message = format!("Cartridge clock set.\n{}", clock.describe());
                if matches!(clock, clock::Clock::Mbc3 { .. }) {
                    message += "\n\nGame Boy clocks count days, not dates: the game keeps its own offset, \
                        so its in-game clock may still need setting in the game.";
                }
                if let Some(backup) = backup {
                    message += &format!("\n\nSave backed up first to\n{}", display_path(&backup));
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
fn display_path(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    s.strip_prefix("/sdcard").unwrap_or(&s).to_string()
}
