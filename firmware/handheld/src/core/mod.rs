use std::{
    fs::File,
    io::{BufReader, BufWriter, ErrorKind, Read, Write},
    ops::DerefMut as _,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use esp_idf_svc::hal::units::Hertz;
use thiserror::Error;

use self::CoreError::*;
use crate::{
    bitstream,
    device::{
        drivers::fpga::{self, SpiCommand, MAX_SPI_READ_CLOCK},
        Device,
    },
    kvs, ui,
};
pub use info::CoreFile;
pub use info::CoreInfo;
pub use info::CoreListEntry;
use browse::{Anchor, EntryKey, Target};
use settings::CoreSettings;

pub mod browse;
mod info;
mod settings;

static CORE_MANAGER: LazyLock<Mutex<CoreManager>> =
    LazyLock::new(|| Mutex::new(CoreManager::new()));

/// Maximum number of files to show at once (larger directories are paged).
const FILE_LIST_MAX: usize = 100;

/// Directory for save files, when they're not stored next to the ROM.
const DIR_SAVES: &str = "/sdcard/saves/";

const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_millis(250);
const NOTIFY_TIMEOUT: Duration = Duration::from_millis(10);
const SETUP_TIMEOUT: Duration = Duration::from_millis(100);
/// Only perform background I/O when reading files at least this large.
const BG_IO_THRESHOLD: usize = 512 * 1024;

#[allow(unused)]
mod command {
    // Commands
    pub const GET_STATUS: u32 = 0x0000;
    pub const CORE_RUN: u32 = 0x0100;
    pub const CORE_HALT: u32 = 0x0101;
    pub const SETUP_COMPLETE: u32 = 0x0102;
    pub const NOTIFY_FOCUS: u32 = 0x0200;
    pub const FILE_WRITE_START: u32 = 0x0300;
    pub const FILE_WRITE_END: u32 = 0x0301;
    pub const FILE_READ_START: u32 = 0x0302;
    pub const FILE_READ_END: u32 = 0x0303;

    // Status
    pub const STATUS_UNKNOWN: u32 = 0;
    pub const STATUS_INITIALIZE: u32 = 1;
    pub const STATUS_SETUP: u32 = 2;
    pub const STATUS_CORE_HALT: u32 = 3;
    pub const STATUS_CORE_RUN: u32 = 4;
}

pub const DIR_SDCARD: &str = "/sdcard/";
pub const DIR_SETTINGS: &str = "/sdcard/settings/";
pub const CORE_POLL_INTERVAL: Duration = Duration::from_millis(5);

pub struct CoreManager {
    stage: Stage,
    listed_cores: bool,
    core_info: Option<info::CoreInfo>,
    core_handler: CoreHandlerImpl,
    core_settings: Option<CoreSettings>,
    selected_files: Vec<Option<PathBuf>>,
    /// ROM to load instead of showing the file browser (Run ROM).
    preset_rom: Option<PathBuf>,
    run_cartridge: bool,

    /// Directory shown in the file browser (`None` for favorites).
    browse_dir: Option<PathBuf>,
    /// Entries currently shown in the file browser.
    browse_entries: Vec<browse::Entry>,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum Stage {
    Idle,
    LoadInit,
    LoadSelectFile(usize),
    LoadBitstream,
    Running,
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Cannot open file {0}")]
    CannotOpenFile(String),
    #[error("Failed to load file {0}:\n{1}")]
    FailedLoadFile(String, String),
    #[error("Failed to save file {0}:\n{1}")]
    FailedSaveFile(String, String),
    #[error("File {0} wrong size:\nExpected {1} bytes\nActually {2} bytes")]
    FileWrongSize(String, u32, u32),
    #[error("File {0} too big:\nMaximum {1} bytes\nActually {2} bytes")]
    FileTooBig(String, u32, u32),
    #[error("Core command {0:04X} timed out")]
    CommandTimeout(u32),
    #[error("Core command {0:04X} failed")]
    CommandFailed(u32),
    #[error("Timed out waiting for core to be ready")]
    CoreReadyTimeout,
    #[error("{0}")]
    Other(String),
}

/// Core-specific lifecycle callbacks.
///
/// The main purpose is to add core-specific customizations for built-in cores
/// while generic core functionality is being built.
pub trait CoreHandler {
    /// Called after the bitstream is programmed.
    fn on_after_program(&mut self);

    /// Called before loading a file, returns the path override.
    fn get_file_path_override(&mut self, id: u16) -> Option<PathBuf>;

    /// Called before a file is loaded, for any additional stuff.
    fn on_before_file_load(&mut self, id: u16, file: &mut File) -> Result<(), String>;

    /// Called for each chunk of a file loaded
    fn on_during_file_load(&mut self, _id: u16, _data: &[u8]) {}

    /// Called after a file is loaded, for any additional stuff.
    fn on_after_file_load(&mut self, _id: u16) {}

    fn on_before_run(&mut self) -> Result<(), String>;

    fn on_focus_changed(&mut self, has_focus: bool);

    /// Called before saving a file. May override the core's file size.
    fn get_file_size(&mut self, id: u16) -> Option<u32>;

    /// Called after file is written, to write any additional data.
    fn on_after_file_save(&mut self, id: u16, file: &mut File) -> Result<(), String>;

    /// Temporary: as Bitstream trait
    fn as_legacy_bitstream(&mut self) -> &mut dyn bitstream::Bitstream;
}

enum CoreHandlerImpl {
    None,
    Gameboy(crate::bitstream::gameboy::Gameboy),
    Gba(crate::bitstream::gba::Gba),
    Snes(crate::bitstream::snes::Snes),
}

impl CoreHandlerImpl {
    fn get_mut(&mut self) -> Option<&mut dyn CoreHandler> {
        match self {
            CoreHandlerImpl::None => None,
            CoreHandlerImpl::Gameboy(gameboy) => Some(gameboy),
            CoreHandlerImpl::Gba(gba) => Some(gba),
            CoreHandlerImpl::Snes(snes) => Some(snes),
        }
    }
}

/// # CoreManager
///
/// Manages the lifecycle of cores.
impl CoreManager {
    fn new() -> Self {
        CoreManager {
            core_info: None,
            listed_cores: false,
            stage: Stage::Idle,
            core_handler: CoreHandlerImpl::None,
            core_settings: None,
            selected_files: Vec::new(),
            preset_rom: None,
            run_cartridge: false,
            browse_dir: None,
            browse_entries: Vec::new(),
        }
    }

    pub fn lock() -> MutexGuard<'static, Self> {
        CORE_MANAGER.lock().unwrap()
    }

    /// Find and list all cores (UI)
    pub fn list_cores(&mut self) {
        if self.listed_cores {
            // Already sent the list to the UI, ignore duplicate request.
            return;
        }

        let list = info::list_cores();
        ui::send(ui::Message::CoreList(list));
    }

    /// Start the process of running a specific core (by ID).
    pub fn run_core(&mut self, id: &str, run_cartridge: bool) {
        log::info!("Run core={id} cart={run_cartridge}");
        assert!(self.core_info.is_none());
        assert!(self.stage == Stage::Idle);
        let core = match info::get_core(id) {
            Ok(core) => {
                self.core_info = Some(core);
                self.core_info.as_ref().unwrap()
            }
            Err(e) => {
                // TODO show error to user?
                log::error!("Error loading core '{id}': {e}");
                return;
            }
        };

        self.core_handler = match core.id.as_str() {
            "Game-Bub.GB" => CoreHandlerImpl::Gameboy(crate::bitstream::gameboy::Gameboy::new()),
            "Game-Bub.GBA" => CoreHandlerImpl::Gba(crate::bitstream::gba::Gba::new()),
            "Game-Bub.SNES" => CoreHandlerImpl::Snes(crate::bitstream::snes::Snes::new()),
            _ => CoreHandlerImpl::None,
        };

        self.stage = Stage::LoadInit;
        self.run_cartridge = run_cartridge;
        self.core_settings = Self::load_settings(core);
        self.core_settings.get_or_insert_default();

        self.selected_files = vec![None; core.files.len()];
        self.next_file_select();
    }

    /// Run a core with a ROM file, without showing the file browser.
    pub fn run_core_with_rom(&mut self, id: &str, rom: PathBuf) {
        self.preset_rom = Some(rom);
        self.run_core(id, false);
        // If the core didn't take it (e.g. it failed to load), don't leave it for later.
        self.preset_rom = None;
    }

    fn load_settings(core: &CoreInfo) -> Option<CoreSettings> {
        let file = match File::open(core.get_settings_path()) {
            Ok(file) => file,
            Err(_) => {
                log::warn!("Failed to open settings file");
                return None;
            }
        };
        let reader = BufReader::with_capacity(256, file);
        let mut settings: CoreSettings = match serde_json::from_reader(reader) {
            Ok(x) => x,
            Err(_) => {
                log::warn!("Failed to parse settings file");
                return None;
            }
        };

        // Only keep file paths that belong to user-selected files.
        settings
            .file_paths
            .retain(|(i, _)| core.files.iter().any(|f| f.id == *i && f.user_selected));
        // TODO: validate non-path settings
        settings.settings.clear();

        Some(settings)
    }

    /// Temporary transitional method
    /// TODO: remove
    pub fn current_bitstream(&mut self) -> Option<&mut dyn bitstream::Bitstream> {
        self.core_handler.get_mut().map(|c| c.as_legacy_bitstream())
    }

    /// Legacy method, should be core-specific (setting?)
    pub fn reset_core(&mut self) {
        let _ = self.run_core_command(&[command::CORE_HALT], &mut [], NOTIFY_TIMEOUT);
        let _ = self.run_core_command(&[command::CORE_RUN], &mut [], NOTIFY_TIMEOUT);
    }

    pub fn prepare_for_power_off(&mut self) {
        if self.stage == Stage::Running {
            if let Err(e) = self.persist_files() {
                log::error!("Error saving: {}", e);
            }
            if let Err(e) = self.persist_settings() {
                log::error!("Error saving settings: {}", e);
            }
        }
        self.reset_state();
    }

    pub fn exit_core(&mut self) {
        if self.stage == Stage::Running {
            let _ = self.run_core_command(&[command::CORE_HALT], &mut [], NOTIFY_TIMEOUT);

            if let Err(e) = self.persist_files() {
                log::error!("Error saving: {}", e);
            }
            if let Err(e) = self.persist_settings() {
                log::error!("Error saving settings: {}", e);
            }
        }

        self.reset_state();

        // Power off the DAC
        Device::lock()
            .dac
            .set_power_down(0, true)
            .expect("DAC power");

        // And go back to the boot bitstream
        bitstream::program_boot();
    }

    /// Exit the current core and run it again, to select a different file.
    pub fn reload_core(&mut self) {
        let id = self.core_info.as_ref().map(|c| c.id);
        let run_cartridge = self.run_cartridge;
        self.exit_core();
        match id {
            Some(id) if !run_cartridge => self.run_core(&id, false),
            _ => log::warn!("No core to reload"),
        }
    }

    fn reset_state(&mut self) {
        self.core_info = None;
        self.core_handler = CoreHandlerImpl::None;
        self.core_settings = None;
        self.stage = Stage::Idle;
        self.selected_files.clear();
        self.browse_dir = None;
        self.browse_entries.clear();
        ui::GAME_FOCUSED.store(false, std::sync::atomic::Ordering::Relaxed);

        Device::lock().set_cart_power(false);
    }

    fn poll_core_status(&mut self, expected: u32, timeout: Duration) -> Result<(), CoreError> {
        let start = Instant::now();
        loop {
            if start.elapsed() > timeout {
                return Err(CoreError::CoreReadyTimeout);
            }
            let mut status = [0u32];
            self.run_core_command(&[command::GET_STATUS], &mut status, timeout)
                .map_err(|_| CoreError::CommandFailed(command::GET_STATUS))?;

            if status[0] == expected {
                return Ok(());
            }
            std::thread::sleep(CORE_POLL_INTERVAL);
        }
    }

    fn run_core_command(
        &self,
        request: &[u32],
        response: &mut [u32],
        timeout: Duration,
    ) -> Result<(), CoreError> {
        assert!(request.len() > 0);
        // Write command and arguments
        {
            let mut device = Device::lock();
            for (i, &x) in request.iter().enumerate() {
                let _ = device
                    .fpga
                    .write_u32(fpga::REG_CMD_HOST_BASE + (4 * i as u32), x);
            }
            let _ = device.fpga.write_u32(fpga::REG_CTRL_CMD_HOST, 0b1000);
        }

        // Poll for command completion.
        let start = Instant::now();
        let status = loop {
            if start.elapsed() > timeout {
                log::error!("Command {:08X} timed out", request[0]);
                return Err(CoreError::CommandTimeout(request[0]));
            }
            let mut device = Device::lock();
            let status = device.fpga.read_u32(fpga::REG_CTRL_CMD_HOST).unwrap();
            if (status & 0b0011) != 0 {
                break status;
            }
            drop(device);
            std::thread::sleep(CORE_POLL_INTERVAL);
        };

        // Read result values and end command
        {
            let mut device = Device::lock();
            for (i, x) in response.iter_mut().enumerate() {
                *x = device
                    .fpga
                    .read_u32(fpga::REG_CMD_HOST_BASE + (4 * i as u32))
                    .unwrap();
            }

            let _ = device.fpga.write_u32(fpga::REG_CTRL_CMD_HOST, 0);
        }

        // Check status.
        if (status & 0b0001) == 0 {
            Ok(())
        } else {
            Err(CoreError::CommandFailed(request[0]))
        }
    }

    pub fn focus_changed(&mut self, has_focus: bool) {
        ui::GAME_FOCUSED.store(has_focus, std::sync::atomic::Ordering::Relaxed);
        {
            let mut device = Device::lock();
            device.dac.set_power_down(0, !has_focus).expect("DAC power");
            let _ = device
                .fpga
                .write_u32(fpga::REG_CTRL_FOCUS, has_focus as u32);
        }

        let _ = self.run_core_command(
            &[command::NOTIFY_FOCUS, has_focus as u32],
            &mut [],
            NOTIFY_TIMEOUT,
        );

        if let Some(core_handler) = self.core_handler.get_mut() {
            core_handler.on_focus_changed(has_focus);
        }
    }

    fn next_file_select(&mut self) {
        let core = self.core_info.as_ref().unwrap();
        let index = loop {
            // Find the index of the next file to load.
            let index = match self.stage {
                Stage::LoadInit => 0,
                Stage::LoadSelectFile(i) => i + 1,
                _ => panic!(),
            };
            if index >= core.files.len() {
                log::info!("File selection complete");
                self.stage = Stage::LoadBitstream;
                self.finish_loading();
                return;
            }
            self.stage = Stage::LoadSelectFile(index);
            let file = &core.files[index];
            if !file.user_selected {
                continue;
            }
            if self.run_cartridge && (file.id == 0 || file.dependent_on_0) {
                // TODO: validate and make sure there's a file 0
                continue;
            }
            break index;
        };
        let file = &core.files[index];
        if file.id == 0 {
            if let Some(rom) = self.preset_rom.take() {
                return self.file_selected(rom);
            }
        }

        // Get the starting path for the file browser.
        let last_path = self
            .core_settings
            .as_ref()
            .and_then(|x| x.file_paths.iter().find(|(id, _)| *id == file.id))
            .map(|(_, path)| path.as_path());
        let last_dir = last_path.and_then(|p| p.parent());

        // Use the last path, or the last dir, or finally the root path.
        let (initial_dir, anchor) = match (last_path, last_dir) {
            (Some(file), Some(parent)) if file.is_file() => {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                (parent, Anchor::Around(EntryKey::new(&name, true)))
            }
            (_, Some(parent)) if parent.is_dir() => (parent, Anchor::Start),
            _ => (Path::new(DIR_SDCARD), Anchor::Start),
        };
        let initial_dir = initial_dir.to_path_buf();

        ui::send(ui::Message::CoreFileSelectBegin {
            label: file.label.to_string(),
        });
        self.browse(Some(initial_dir), anchor);
    }

    fn finish_loading(&mut self) {
        if let Err(e) = self.finish_loading_inner() {
            self.exit_core();
            ui::send(ui::Message::CoreLoadError(e.to_string()));
        }
    }

    fn finish_loading_inner(&mut self) -> Result<(), CoreError> {
        self.load_bitstream();

        // Wait for the core to be ready for setup.
        self.poll_core_status(command::STATUS_SETUP, SETUP_TIMEOUT)?;
        ui::send(ui::Message::EnterGame);

        if self.run_cartridge {
            let mut device = Device::lock();
            // Power turned off in reset_state.
            device.set_cart_power(true);
        }

        self.load_files()?;

        if let Some(core_handler) = self.core_handler.get_mut() {
            core_handler
                .on_before_run()
                .map_err(|err| CoreError::Other(err))?;
        }

        // Tell the core we're finished setting up.
        self.run_core_command(&[command::SETUP_COMPLETE], &mut [], NOTIFY_TIMEOUT)?;

        // Wait for the core to be ready for run.
        self.poll_core_status(command::STATUS_CORE_HALT, SETUP_TIMEOUT)?;
        log::info!("Core ready to run");

        // Clear loading bar
        ui::send(ui::Message::EnterGame);
        self.stage = Stage::Running;

        // Start core.
        let _ = Device::lock().fpga.write_u32(fpga::REG_CTRL_VIBRATE, 1);
        let _ = self.run_core_command(&[command::CORE_RUN], &mut [], NOTIFY_TIMEOUT);
        self.focus_changed(true);
        Ok(())
    }

    fn load_bitstream(&mut self) {
        assert!(self.stage == Stage::LoadBitstream);

        bitstream::program_fpga(&self.core_info.as_ref().unwrap().bitstream);
        if let Some(core_handler) = self.core_handler.get_mut() {
            core_handler.on_after_program();
        }
    }

    fn load_files(&mut self) -> Result<(), CoreError> {
        // TODO: Sum of size of files to load (rather than doing it file-by-file).
        let mut overall_transferred = 0u64;
        let mut overall_total = 0u64;
        let mut last_progress_update = Instant::now();

        let mut scratch = crate::bitstream::SCRATCH.take().expect("scratch buffer");

        let core = self.core_info.as_ref().unwrap();
        let file_0_index = core.files.iter().position(|f| f.id == 0);
        for (i, info) in core.files.iter().enumerate() {
            if self.run_cartridge && (info.id == 0 || info.dependent_on_0) {
                continue;
            }

            // Get the file path
            let mut path = self.selected_files[i].clone();

            // TODO if asset path is not none, construct path based on that

            // Path to load from, if different from the path to save to.
            let mut load_path = None;
            if info.dependent_on_0 {
                // Construct a new path based on file 0's path
                assert!(path.is_none());
                let extension = &info.extensions[0][1..]; // Remove the dot
                let file_0_index = file_0_index.unwrap();
                let rom_path = self.selected_files[file_0_index].as_ref();
                let beside_rom = rom_path.map(|p| p.with_extension(extension));
                if !info.read_only && kvs::keys::SAVE_LOCATION.get().unwrap_or(0) == 1 {
                    let in_saves_dir = rom_path.map(|p| saves_dir_path(p, extension));
                    // If there's no file in the saves directory yet, load the one
                    // next to the ROM (it'll be saved to the saves directory).
                    if let (Some(new), Some(old)) = (&in_saves_dir, &beside_rom) {
                        if !new.exists() && old.exists() {
                            load_path = Some(old.clone());
                        }
                    }
                    path = in_saves_dir;
                } else {
                    path = beside_rom;
                }
            }

            // Possibly override the path
            if let Some(core_handler) = self.core_handler.get_mut() {
                path = path.or(core_handler.get_file_path_override(info.id));
            }

            let path = match path {
                Some(path) => path,
                // Use an empty path. Open will fail and we'll clear it.
                None if info.optional => PathBuf::new(),
                // It shouldn't be possible to have no path for a required file.
                None => panic!("Missing path for required file {}", info.label),
            };

            let mut load_path = load_path.unwrap_or_else(|| path.clone());
            if !info.read_only && !load_path.exists() {
                // Recover from an interrupted save, if possible.
                let backup = backup_path(&load_path);
                if backup.is_file() {
                    log::warn!("Loading backup file {}", backup.display());
                    load_path = backup;
                }
            }
            log::info!("Load file {} from {}", info.label, load_path.display());

            let file = File::open(&load_path);
            self.selected_files[i] = Some(path);
            let mut file = match file {
                Ok(file) => file,
                Err(_) if info.optional && info.initialize => {
                    log::info!("Failed to open file, clearing");
                    self.clear_file_slot(info, &mut scratch)?;
                    continue;
                }
                Err(_) if info.optional => {
                    log::info!("Failed to open file, skipping");
                    continue;
                }
                Err(_) => {
                    return Err(CannotOpenFile(info.label.to_string()));
                }
            };

            self.run_core_command(
                &[command::FILE_WRITE_START, info.id as u32],
                &mut [],
                SETUP_TIMEOUT,
            )?;

            if let Some(core_handler) = self.core_handler.get_mut() {
                core_handler
                    .on_before_file_load(info.id, &mut file)
                    .map_err(|e| FailedLoadFile(info.label.to_string(), e))?;
            }
            let file_size = file.metadata().unwrap().len();
            overall_total += file_size;

            if info.exact_size != 0 && file_size != (info.exact_size as u64) {
                return Err(FileWrongSize(
                    info.label.to_string(),
                    info.exact_size,
                    file_size as u32,
                ));
            }
            if info.max_size != 0 && file_size > (info.max_size as u64) {
                return Err(FileTooBig(
                    info.label.to_string(),
                    info.max_size,
                    file_size as u32,
                ));
            }

            let start_time = Instant::now();
            let mut transfer_duration = Duration::ZERO;
            let mut handler_duration = Duration::ZERO;
            let mut transferred = 0;
            let mut on_chunk_read = |chunk: &[u8]| {
                let transfer_start = Instant::now();
                let max_clock = Some(Hertz(info.max_transfer_speed * 1000 * 2));
                let command = SpiCommand::new(info.transfer_word_size);
                Device::lock()
                    .fpga
                    .spi_write(max_clock, command, info.address + transferred, chunk)
                    .unwrap();
                transferred += chunk.len() as u32;
                overall_transferred += chunk.len() as u64;
                transfer_duration += transfer_start.elapsed();

                let handler_start = Instant::now();

                if let Some(core_handler) = self.core_handler.get_mut() {
                    core_handler.on_during_file_load(info.id, chunk);
                }
                handler_duration += handler_start.elapsed();

                // Update UI progress bar.
                if last_progress_update.elapsed() > PROGRESS_UPDATE_INTERVAL {
                    let progress = (overall_transferred as f32) / (overall_total as f32);
                    ui::send(ui::Message::CoreLoadProgress(progress));
                    last_progress_update = Instant::now();
                }
            };

            // Only do background I/O for large files.
            let result = if file_size >= BG_IO_THRESHOLD as u64 {
                crate::util::background_io::iter_chunks(file, &mut scratch, on_chunk_read)
            } else {
                let mut duration = Duration::ZERO;
                loop {
                    let read_start = Instant::now();
                    let result = file.read(&mut scratch);
                    duration += read_start.elapsed();

                    match result {
                        Ok(0) => break Ok(duration),
                        Ok(n) => on_chunk_read(&scratch[..n]),
                        Err(err) => break Err(err),
                    }
                }
            };

            let read_duration = result
                .map_err(|_| FailedLoadFile(info.label.to_string(), "I/O error".to_string()))?;

            let duration = start_time.elapsed();
            self.run_core_command(
                &[
                    // Command
                    command::FILE_WRITE_END,
                    // Arg 1: file ID
                    info.id as u32,
                    // Arg 2: file size (bytes)
                    transferred as u32,
                    // Arg 3: reserved (0)
                    0,
                ],
                &mut [],
                NOTIFY_TIMEOUT,
            )?;
            if let Some(core_handler) = self.core_handler.get_mut() {
                core_handler.on_after_file_load(info.id);
            }

            log::info!(
                "Loaded {} bytes in {} ms ({}/{}/{} ms read/transfer/handler)",
                transferred,
                duration.as_millis(),
                read_duration.as_millis(),
                transfer_duration.as_millis(),
                handler_duration.as_millis(),
            );
        }

        Ok(())
    }

    fn persist_files(&mut self) -> Result<(), CoreError> {
        assert!(self.stage == Stage::Running);
        let core = self.core_info.as_ref().unwrap();
        for (i, info) in core.files.iter().enumerate() {
            if info.read_only {
                continue;
            }
            if self.run_cartridge && (info.id == 0 || info.dependent_on_0) {
                continue;
            }

            let path = self.selected_files[i].clone().unwrap();
            log::info!("Saving file {} to {}", info.label, path.display());

            let mut file_size = 0u32;
            self.run_core_command(
                &[command::FILE_READ_START, info.id as u32],
                std::slice::from_mut(&mut file_size),
                SETUP_TIMEOUT,
            )?;
            if let Some(override_size) = self
                .core_handler
                .get_mut()
                .and_then(|x| x.get_file_size(info.id))
            {
                file_size = override_size;
            }

            // Write to a temporary file first, so that the previous save is
            // kept intact if writing fails or is interrupted.
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let temp_path = temp_path(&path);
            let mut file = File::create(&temp_path)
                .map_err(|_| FailedSaveFile(info.label.to_string(), "Open failed".to_string()))?;
            let mut scratch = crate::bitstream::SCRATCH.take().expect("scratch buffer");
            let buf = scratch.deref_mut();
            let mut address: u32 = info.address;
            let mut bytes_left = file_size as usize;

            let start_time = Instant::now();
            while bytes_left > 0 {
                let to_read = bytes_left.min(buf.len());
                let data = &mut buf[0..to_read];

                let max_clock =
                    Some(Hertz(info.max_transfer_speed * 1000 * 2).min(MAX_SPI_READ_CLOCK));
                let command = SpiCommand::new(info.transfer_word_size);
                let _ = Device::lock()
                    .fpga
                    .spi_read(max_clock, command, address, data);
                file.write_all(data).map_err(|_| {
                    FailedSaveFile(info.label.to_string(), "Write failed".to_string())
                })?;
                address += to_read as u32;
                bytes_left -= to_read;
            }
            log::info!(
                "Saved {} bytes in {}ms",
                file_size,
                start_time.elapsed().as_millis() as u32
            );

            self.run_core_command(
                &[command::FILE_READ_END, info.id as u32],
                &mut [],
                NOTIFY_TIMEOUT,
            )?;
            if let Some(core_handler) = self.core_handler.get_mut() {
                core_handler
                    .on_after_file_save(info.id, &mut file)
                    .map_err(|e| FailedSaveFile(info.label.to_string(), e))?;
            }
            file.sync_all()
                .map_err(|_| FailedSaveFile(info.label.to_string(), "Sync failed".to_string()))?;
            drop(file);

            replace_file(&temp_path, &path)
                .map_err(|_| FailedSaveFile(info.label.to_string(), "Rename failed".to_string()))?;
        }
        Ok(())
    }

    /// Called to persist core settings to the core settings JSON file
    pub fn persist_settings(&mut self) -> Result<(), std::io::Error> {
        let core = self.core_info.as_ref().unwrap();
        let settings = self.core_settings.as_ref().unwrap();

        if !std::fs::exists(DIR_SETTINGS)? {
            std::fs::create_dir(DIR_SETTINGS)?;
        }
        let file = File::create(core.get_settings_path())?;
        let writer = BufWriter::with_capacity(256, file);
        serde_json::to_writer(writer, settings)
            .map_err(|e| std::io::Error::new(ErrorKind::Other, e))
    }

    /// Called when a file browser entry is chosen.
    pub fn file_select_choose(&mut self, index: usize) {
        let Some(entry) = self.browse_entries.get(index) else {
            log::warn!("Invalid file browser entry {index}");
            return;
        };
        match entry.target.clone() {
            Target::Path(path) if entry.is_dir => self.browse(Some(path), Anchor::Start),
            Target::Path(path) => self.file_selected(path),
            Target::Favorites => self.browse(None, Anchor::Start),
            Target::PrevPage => {
                let first = self.first_key();
                let anchor = first.map_or(Anchor::Start, Anchor::Before);
                self.browse(self.browse_dir.clone(), anchor);
            }
            Target::NextPage => {
                let last = self.last_key();
                let anchor = last.map_or(Anchor::Start, Anchor::After);
                self.browse(self.browse_dir.clone(), anchor);
            }
        }
    }

    /// Called to go to the parent directory in the file browser.
    pub fn file_select_up(&mut self) {
        match self.browse_dir.clone() {
            // Favorites: go back to the root directory.
            None => self.browse(Some(PathBuf::from(DIR_SDCARD)), Anchor::Start),
            Some(dir) if dir == Path::new(DIR_SDCARD) => {
                log::warn!("No parent directory");
            }
            Some(dir) => {
                // Highlight the directory we're just leaving.
                let name = dir.file_name().unwrap_or_default().to_string_lossy();
                let anchor = Anchor::Around(EntryKey::new(&name, false));
                let parent = dir.parent().map(Path::to_path_buf);
                self.browse(parent, anchor);
            }
        }
    }

    /// Add or remove a file browser entry from the favorites.
    pub fn file_select_toggle_favorite(&mut self, index: usize) {
        let Some(entry) = self.browse_entries.get(index) else {
            return;
        };
        let Target::Path(path) = &entry.target else {
            return;
        };
        if entry.is_dir {
            return;
        }
        let path = path.clone();
        let Some(settings) = self.core_settings.as_mut() else {
            return;
        };

        let added = match settings.favorites.iter().position(|p| *p == path) {
            Some(i) => {
                settings.favorites.remove(i);
                false
            }
            None => {
                settings.favorites.push(path);
                true
            }
        };
        if let Err(e) = self.persist_settings() {
            log::error!("Error saving settings: {}", e);
        }

        let text = if added {
            "Added to Favorites"
        } else {
            "Removed from Favorites"
        };
        ui::send(ui::Message::Notification(ui::Notification::new_short(
            text.to_string(),
        )));

        // Refresh the favorites list if we're viewing it.
        if self.browse_dir.is_none() {
            let favorites = self.favorites_entries();
            let highlight = index.min(favorites.len().saturating_sub(1));
            self.show_entries(favorites, highlight);
        }
    }

    fn file_selected(&mut self, path: PathBuf) {
        log::info!("File selected: {}", path.display());
        let Stage::LoadSelectFile(file_index) = self.stage else {
            panic!()
        };

        // Update settings
        let settings = self.core_settings.as_mut().unwrap();
        let file_id = self.core_info.as_ref().unwrap().files[file_index].id;
        settings.file_paths.retain(|x| x.0 != file_id);
        settings.file_paths.push((file_id, path.clone()));

        self.selected_files[file_index] = Some(path);
        self.browse_entries.clear();
        self.next_file_select();
    }

    /// Called if a file selection is cancelled.
    pub fn cancel_file_select(&mut self) {
        // TODO support multiple file select (go back to previous file)
        log::info!("File select cancelled");
        if self.core_info.is_some() && self.stage != Stage::Running {
            // Keep favorites changes.
            let _ = self.persist_settings();
        }
        self.core_info = None;
        self.core_settings = None;
        self.stage = Stage::Idle;
        self.browse_dir = None;
        self.browse_entries.clear();
    }

    fn first_key(&self) -> Option<EntryKey> {
        self.browse_entries.iter().find_map(Self::entry_key)
    }

    fn last_key(&self) -> Option<EntryKey> {
        self.browse_entries.iter().rev().find_map(Self::entry_key)
    }

    fn entry_key(entry: &browse::Entry) -> Option<EntryKey> {
        match &entry.target {
            Target::Path(path) => {
                let name = path.file_name()?.to_string_lossy();
                Some(EntryKey::new(&name, !entry.is_dir))
            }
            _ => None,
        }
    }

    /// Show a directory (or the favorites, if `None`) in the file browser.
    fn browse(&mut self, dir: Option<PathBuf>, anchor: Anchor) {
        self.browse_dir = dir.clone();
        let Some(dir) = dir else {
            let favorites = self.favorites_entries();
            self.show_entries(favorites, 0);
            return;
        };

        let file_index = match self.stage {
            Stage::LoadSelectFile(i) => i,
            _ => panic!(),
        };
        let extensions = &self.core_info.as_ref().unwrap().files[file_index].extensions;
        let extensions: Vec<&str> = extensions.iter().map(|e| e.as_str()).collect();

        let window = match browse::read_dir_keys(&dir, &extensions) {
            Ok(keys) => browse::window(keys, &anchor, FILE_LIST_MAX),
            Err(e) => {
                log::warn!("Error listing directory: {:?}", e);
                ui::send(ui::Message::CoreFileSelectError(format!(
                    "Error listing directory:\n{}",
                    e,
                )));
                browse::window(std::iter::empty(), &anchor, FILE_LIST_MAX)
            }
        };

        let mut entries = Vec::with_capacity(window.entries.len() + 3);
        let mut highlight = window.highlight;
        if window.has_prev {
            entries.push(browse::Entry {
                label: "<< Previous page".to_string(),
                is_dir: false,
                target: Target::PrevPage,
            });
            highlight += 1;
        } else if dir == Path::new(DIR_SDCARD)
            && self
                .core_settings
                .as_ref()
                .map_or(false, |s| !s.favorites.is_empty())
        {
            entries.push(browse::Entry {
                label: "Favorites".to_string(),
                is_dir: true,
                target: Target::Favorites,
            });
            if matches!(anchor, Anchor::Around(_)) {
                highlight += 1;
            }
        }
        for key in window.entries {
            entries.push(browse::Entry {
                target: Target::Path(dir.join(&key.name)),
                label: key.name,
                is_dir: !key.is_file,
            });
        }
        if window.has_next {
            entries.push(browse::Entry {
                label: "Next page >>".to_string(),
                is_dir: false,
                target: Target::NextPage,
            });
        }
        // When paging forward, highlight the first real entry.
        if matches!(anchor, Anchor::After(_)) && window.has_prev {
            highlight = 1;
        }
        self.show_entries(entries, highlight);
    }

    /// Build the list of favorites for the file browser.
    fn favorites_entries(&self) -> Vec<browse::Entry> {
        let Some(settings) = self.core_settings.as_ref() else {
            return Vec::new();
        };
        let mut favorites: Vec<(EntryKey, &PathBuf)> = settings
            .favorites
            .iter()
            .filter(|p| p.is_file())
            .filter_map(|p| Some((EntryKey::new(&p.file_name()?.to_string_lossy(), true), p)))
            .take(FILE_LIST_MAX)
            .collect();
        favorites.sort_by(|a, b| a.0.cmp(&b.0));
        favorites
            .into_iter()
            .map(|(key, path)| browse::Entry {
                label: key.name,
                is_dir: false,
                target: Target::Path(path.clone()),
            })
            .collect()
    }

    fn show_entries(&mut self, entries: Vec<browse::Entry>, highlight: usize) {
        let path = match &self.browse_dir {
            None => "/Favorites".to_string(),
            Some(dir) => {
                let mut path = dir
                    .strip_prefix(DIR_SDCARD)
                    .unwrap_or(dir)
                    .to_string_lossy()
                    .into_owned();
                if !path.starts_with('/') {
                    path.insert(0, '/');
                }
                path
            }
        };
        let files = entries
            .iter()
            .map(|e| (e.label.clone(), e.is_dir))
            .collect();
        self.browse_entries = entries;
        ui::send(ui::Message::CoreFileSelectList {
            path,
            files,
            selected: highlight,
        });
    }

    /// Clear a file slot to 0xFF
    fn clear_file_slot(&self, info: &CoreFile, buf: &mut [u8]) -> Result<(), CoreError> {
        self.run_core_command(
            &[command::FILE_WRITE_START, info.id as u32],
            &mut [],
            SETUP_TIMEOUT,
        )?;

        buf.fill(0xFF);
        let mut pos = 0u32;
        let len = info.max_size.max(info.exact_size);
        while pos < len {
            let n = ((len - pos) as usize).min(buf.len());
            let max_clock = Some(Hertz(info.max_transfer_speed * 1000 * 2));
            let command = SpiCommand::new(info.transfer_word_size);
            let _ =
                Device::lock()
                    .fpga
                    .spi_write(max_clock, command, info.address + pos, &buf[..n]);
            pos += n as u32;
        }

        self.run_core_command(
            &[
                // Command
                command::FILE_WRITE_END,
                // Arg 1: file ID
                info.id as u32,
                // Arg 2: file size (bytes)
                len,
                // Arg 3: reserved (0)
                0,
            ],
            &mut [],
            NOTIFY_TIMEOUT,
        )?;

        Ok(())
    }
}

/// Point a core's remembered file paths and favorites at a ROM's new path, after it has been
/// renamed (only while no core is running, which keeps its settings in memory).
pub fn rename_rom_in_settings(core_id: &str, old: &Path, new: &Path) {
    let mut path = PathBuf::from(DIR_SETTINGS);
    path.push(core_id);
    path.add_extension("json");
    let Ok(file) = File::open(&path) else {
        return;
    };
    let Ok(mut settings) =
        serde_json::from_reader::<_, CoreSettings>(BufReader::with_capacity(256, file))
    else {
        return;
    };
    let mut changed = false;
    let paths = settings.file_paths.iter_mut().map(|(_, p)| p);
    for p in paths.chain(settings.favorites.iter_mut()) {
        if p == old {
            *p = new.to_path_buf();
            changed = true;
        }
    }
    if changed {
        let result = File::create(&path).map_err(|e| e.to_string()).and_then(|file| {
            serde_json::to_writer(BufWriter::with_capacity(256, file), &settings)
                .map_err(|e| e.to_string())
        });
        if let Err(e) = result {
            log::warn!("Couldn't update {}: {e}", path.display());
        }
    }
}

/// Path for a save file in the saves directory, e.g. `/sdcard/saves/gba/Game.sav`.
pub(crate) fn saves_dir_path(rom_path: &Path, extension: &str) -> PathBuf {
    let system = rom_path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "other".to_string());
    let mut path = PathBuf::from(DIR_SAVES);
    path.push(system);
    path.push(rom_path.file_name().unwrap_or_default());
    path.set_extension(extension);
    path
}

/// Where a ROM's save file is kept, following the "Save Files" setting.
pub fn save_path_for_rom(rom_path: &Path) -> PathBuf {
    if kvs::keys::SAVE_LOCATION.get().unwrap_or(0) == 1 {
        saves_dir_path(rom_path, "sav")
    } else {
        rom_path.with_extension("sav")
    }
}

/// Append a suffix to a file name (`Game.sav` -> `Game.sav.bak`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    match path.file_name() {
        Some(name) => {
            let mut name = name.to_os_string();
            name.push(suffix);
            path.with_file_name(name)
        }
        None => PathBuf::new(),
    }
}

/// Path of the previous version of a save file.
fn backup_path(path: &Path) -> PathBuf {
    with_suffix(path, ".bak")
}

/// Path of a save file while it's being written.
pub fn temp_path(path: &Path) -> PathBuf {
    with_suffix(path, ".tmp")
}

/// Replace `path` with `new`, keeping the previous version as a backup.
pub fn replace_file(new: &Path, path: &Path) -> std::io::Result<()> {
    if path.exists() {
        let backup = backup_path(path);
        // FAT can't rename over an existing file.
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(path, &backup)?;
    }
    std::fs::rename(new, path)
}
