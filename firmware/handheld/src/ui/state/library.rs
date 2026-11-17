use std::{cell::RefCell, rc::Rc, sync::atomic::Ordering};

use super::super::slint::Backend;
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};

use crate::{
    device::Device,
    ui::FRAMES_DRAWN,
    kvs,
    library::{self, Game, System},
    worker,
};

use super::UiState;

/// State of the "Run ROM" screen.
#[derive(Default)]
pub struct LibraryState {
    /// Entries of the system list: Favorites (None), then the systems that have a ROM folder
    systems: Vec<Option<System>>,
    /// Showing the favorites (of every system) rather than one system's games
    favorites: bool,
    /// Games shown
    games: Vec<Game>,
    /// Box art buffers, reused from game to game
    strip_pool: library::StripPool,
    /// Frame count when a full redraw was requested to release the box art (see
    /// library_focus_game)
    boxart_redraw_at: Option<u32>,
}

fn strings(v: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    ModelRc::from(Rc::new(VecModel::from(
        v.into_iter().map(SharedString::from).collect::<Vec<_>>(),
    )))
}

impl UiState {
    /// Set up the "Run ROM" screen.
    pub(super) fn setup_library(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        let state_ = state.clone();
        backend.on_library_open(move || state_.borrow_mut().library_open());

        let state_ = state.clone();
        backend.on_library_open_system(move |i| state_.borrow_mut().library_open_system(i as usize));

        let state_ = state.clone();
        backend.on_library_focus_game(move |i| state_.borrow_mut().library_focus_game(i as usize));

        let state_ = state.clone();
        backend.on_library_run_game(move |i| state_.borrow_mut().library_run_game(i as usize));

        let state_ = state.clone();
        backend.on_library_close(move || state_.borrow_mut().library_close());

        let state_ = state.clone();
        backend.on_library_clear_boxart(move || state_.borrow_mut().library_clear_boxart());

        let state_ = state.clone();
        backend.on_library_toggle_favorite(move |i| state_.borrow_mut().library_toggle_favorite(i as usize));
        let state_ = state.clone();
        backend.on_library_play_music(move |i| state_.borrow_mut().library_play_music(i as usize));
    }

    /// Release the box art. Its memory is only freed after the next redraw, so the next image
    /// is loaded later (from a timer), never right after this.
    fn library_clear_boxart(&mut self) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_library_boxart(ModelRc::default());
        backend.set_library_has_boxart(false);
        backend.set_library_boxart_note("".into());
    }

    /// Release the game list and box art (when leaving the screen or starting a game).
    fn library_close(&mut self) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_library_boxart(ModelRc::default());
        backend.set_library_has_boxart(false);
        backend.set_library_games(ModelRc::default());
        self.library.games = vec![];
        self.library.strip_pool.clear();
    }

    /// Show the list of systems.
    fn library_open(&mut self) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_library_boxart(ModelRc::default());
        backend.set_library_has_boxart(false);
        // Systems in alphabetical order (after Favorites).
        let mut systems: Vec<(System, usize)> = library::SYSTEMS
            .iter()
            .filter(|s| s.rom_dir().is_dir())
            .map(|s| {
                crate::crash_handler::note(&format_args!("Run ROM: counting {}", s.folder));
                (*s, s.games().len())
            })
            .collect();
        systems.sort_by(|a, b| crate::core::browse::natural_cmp(a.0.folder, b.0.folder));
        let favorites = library::favorites().len();
        let empty = systems.is_empty() && favorites == 0;
        backend.set_library_systems(strings(
            std::iter::once(format!("{FAVORITES} ({favorites})"))
                .chain(systems.iter().map(|(s, n)| format!("{} ({n})", s.folder)))
                .filter(|_| !empty),
        ));
        backend.set_library_empty(empty);
        self.library.systems = std::iter::once(None)
            .chain(systems.into_iter().map(|(s, _)| Some(s)))
            .filter(|_| !empty)
            .collect();
        self.library.favorites = false;
        self.library.games.clear();
        backend.set_library_view(0);

        // Return to the last system viewed.
        let last = kvs::keys::LIBRARY_LAST_SYSTEM.get().unwrap_or_default();
        let index = self
            .library
            .systems
            .iter()
            .position(|s| s.map_or(FAVORITES, |s| s.folder) == last)
            .unwrap_or(0);
        backend.set_library_system_index(index as i32);
    }

    /// Show the games of a system, or the favorites.
    fn library_open_system(&mut self, index: usize) {
        let Some(system) = self.library.systems.get(index).copied() else {
            return;
        };
        let name = system.map_or(FAVORITES, |s| s.folder);
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_library_has_music(false);
        // Each system's box art buffers are sized for it.
        backend.set_library_boxart(ModelRc::default());
        backend.set_library_has_boxart(false);
        self.library.strip_pool.clear();
        self.library.games = system.map_or_else(library::favorites, |s| s.games());
        crate::crash_handler::note(&format_args!("Run ROM: {name} ({} games)", self.library.games.len()));
        self.library.favorites = system.is_none();
        kvs::keys::LIBRARY_LAST_SYSTEM.set(&name.to_string());

        backend.set_library_system_name(name.into());
        backend.set_library_games(strings(library::System::titles(&self.library.games).into_iter()));

        // Select the last game played from this system, if it's still there.
        let last = kvs::keys::LIBRARY_LAST_GAME.get().unwrap_or_default();
        let game_index = self
            .library
            .games
            .iter()
            .position(|g| g.path.to_string_lossy() == last)
            .unwrap_or(0);
        backend.set_library_game_index(game_index as i32);
        backend.set_library_view(1);
        self.library_focus_game(game_index);
    }

    /// Show the box art of the highlighted game.
    fn library_focus_game(&mut self, index: usize) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_library_has_music(self.library.games.get(index).is_some_and(|g| {
            g.system.core_id == "Game-Bub.GB" && {
                let mut header = [0u8; 0x150];
                std::fs::File::open(&g.path)
                    .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut header))
                    .is_ok_and(|_| crate::cart_tool::music::has_music(&header))
            }
        }));
        // The box art buffers are shared with the image on screen, which is only released when a
        // redraw visits the box art (redraws of just the scrolling name don't); refilling them
        // before that would copy them, using twice the memory. So once the highlight has stopped,
        // clear the image, ask for a full redraw, and load after it.
        if !self.library.strip_pool.is_empty() {
            let frames = FRAMES_DRAWN.load(Ordering::Relaxed);
            let wait = match self.library.boxart_redraw_at {
                None => {
                    backend.set_library_boxart(ModelRc::default());
                    backend.set_library_has_boxart(false);
                    crate::ui::send(crate::ui::Message::Redraw);
                    self.library.boxart_redraw_at = Some(frames);
                    true
                }
                // A partial redraw may come before the full one.
                Some(at) => frames < at.wrapping_add(2),
            };
            if wait {
                let root_ = self.root.clone();
                slint::Timer::single_shot(std::time::Duration::from_millis(30), move || {
                    if let Some(root) = root_.upgrade() {
                        // The highlighted game, which may have changed since.
                        let backend = root.global::<Backend>();
                        backend.invoke_library_focus_game(backend.get_library_game_index());
                    }
                });
                return;
            }
            self.library.boxart_redraw_at = None;
        }
        // Release the previous image first, so two are never in memory at once.
        backend.set_library_boxart(ModelRc::default());
        backend.set_library_has_boxart(false);
        let thumbnail = match self.library.games.get(index) {
            Some(game) => library::load_thumbnail(
                &game.thumbnail_path(),
                &mut self.library.strip_pool,
                library::BOXART_SIZE,
                library::BOXART_SIZE,
            ),
            _ => None,
        };
        match thumbnail {
            Some(Ok(thumbnail)) => {
                crate::crash_handler::note(&format_args!("box art {}x{} loaded", thumbnail.width, thumbnail.height));
                let strips: Vec<Image> = thumbnail.strips.into_iter().map(Image::from_rgb8).collect();
                backend.set_library_boxart_width(thumbnail.width as i32);
                backend.set_library_boxart_height(thumbnail.height as i32);
                backend.set_library_boxart_reduced(thumbnail.reduced);
                backend.set_library_boxart(ModelRc::from(Rc::new(VecModel::from(strips))));
                backend.set_library_has_boxart(true);
            }
            Some(Err(low)) => backend.set_library_boxart_note(
                format!("Not enough memory for box art ({} KB free)", low.largest_free / 1024).into(),
            ),
            None if self.library.games.is_empty() && self.library.favorites => backend
                .set_library_boxart_note("No favorites yet. Press Select on a game to add it.".into()),
            None => backend.set_library_boxart_note("No box art".into()),
        }
    }

    /// Run the selected game.
    fn library_run_game(&mut self, index: usize) {
        let Some(game) = self.library.games.get(index).cloned() else {
            return;
        };
        let system = game.system;
        kvs::keys::LIBRARY_LAST_GAME.set(&game.path.to_string_lossy().into_owned());
        // Free the box art and game list before the core loads.
        self.library_close();
        kvs::keys::LAST_CORE.set(&system.core_id.to_string());
        let root = self.root.unwrap();
        // The in-game "Load ROM" opens the core's file browser.
        root.global::<Backend>().set_game_can_load_rom(true);
        worker::send(worker::Message::RunRom {
            core_id: system.core_id.to_string(),
            path: game.path.clone(),
        });
    }

    /// Play the game's music (built-in cartridge music data) instead of the game.
    fn library_play_music(&mut self, index: usize) {
        let Some(game) = self.library.games.get(index).cloned() else {
            return;
        };
        self.library_close();
        let root = self.root.unwrap();
        root.global::<Backend>().set_game_can_load_rom(false);
        crate::cart_tool::play_rom_music(game.path.clone());
    }

    /// Add the game to its core's favorites, or remove it.
    fn library_toggle_favorite(&mut self, index: usize) {
        let Some(game) = self.library.games.get(index).cloned() else {
            return;
        };
        let result = crate::core::toggle_core_favorite(game.system.core_id, &game.path);
        let text = match &result {
            Ok(true) => "Added to Favorites".to_string(),
            Ok(false) => "Removed from Favorites".to_string(),
            Err(e) => {
                log::error!("Couldn't save favorites: {e}");
                "Couldn't save Favorites".to_string()
            }
        };
        crate::ui::send(crate::ui::Message::Notification(crate::ui::Notification::new_short(text)));

        // In the favorites, a removed game leaves the list.
        if self.library.favorites && result == Ok(false) {
            self.library.games.remove(index);
            let root = self.root.unwrap();
            let backend = root.global::<Backend>();
            backend.set_library_games(strings(library::System::titles(&self.library.games).into_iter()));
            let index = index.min(self.library.games.len().saturating_sub(1));
            backend.set_library_game_index(index as i32);
            self.library_clear_boxart();
            self.library_focus_game(index);
        }
    }
}

/// Name of the favorites entry in the system list.
const FAVORITES: &str = "Favorites";
