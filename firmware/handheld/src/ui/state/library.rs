use std::{cell::RefCell, rc::Rc};

use super::super::slint::Backend;
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};

use crate::{
    device::Device,
    kvs,
    library::{self, Game, System},
    worker,
};

use super::UiState;

/// State of the "Run ROM" screen.
#[derive(Default)]
pub struct LibraryState {
    /// Systems that have a ROM folder
    systems: Vec<System>,
    /// Selected system and its games
    system: Option<System>,
    games: Vec<Game>,
    /// Box art buffers, reused from game to game
    strip_pool: library::StripPool,
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
        let systems: Vec<(System, usize)> = library::SYSTEMS
            .iter()
            .filter(|s| s.rom_dir().is_dir())
            .map(|s| (*s, s.games().len()))
            .collect();
        backend.set_library_systems(strings(
            systems.iter().map(|(s, n)| format!("{} ({n})", s.folder)),
        ));
        backend.set_library_empty(systems.is_empty());
        self.library.systems = systems.into_iter().map(|(s, _)| s).collect();
        self.library.system = None;
        self.library.games.clear();
        backend.set_library_view(0);

        // Return to the last system viewed.
        let last = kvs::keys::LIBRARY_LAST_SYSTEM.get().unwrap_or_default();
        let index = self.library.systems.iter().position(|s| s.folder == last).unwrap_or(0);
        backend.set_library_system_index(index as i32);
    }

    /// Show the games of a system.
    fn library_open_system(&mut self, index: usize) {
        let Some(system) = self.library.systems.get(index).copied() else {
            return;
        };
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        // Each system's box art buffers are sized for it.
        backend.set_library_boxart(ModelRc::default());
        backend.set_library_has_boxart(false);
        self.library.strip_pool.clear();
        self.library.games = system.games();
        self.library.system = Some(system);
        kvs::keys::LIBRARY_LAST_SYSTEM.set(&system.folder.to_string());

        backend.set_library_system_name(system.folder.into());
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
        // Release the previous image first, so two are never in memory at once.
        backend.set_library_boxart(ModelRc::default());
        backend.set_library_has_boxart(false);
        let thumbnail = match (self.library.system, self.library.games.get(index)) {
            (Some(system), Some(game)) => library::load_thumbnail(
                &system.thumbnail_path(&game.name),
                &mut self.library.strip_pool,
                // Game Boy box art is square (the largest), and those lists are often long:
                // full-size art and a screen full of names don't fit in memory together, so it
                // is shown at half resolution (scaled up to the same size).
                if system.folder == "Game Boy Advance" { 1 } else { 2 },
            ),
            _ => None,
        };
        match thumbnail {
            Some(Ok(thumbnail)) => {
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
            None => backend.set_library_boxart_note("No box art".into()),
        }
    }

    /// Run the selected game.
    fn library_run_game(&mut self, index: usize) {
        let (Some(system), Some(game)) = (self.library.system, self.library.games.get(index).cloned())
        else {
            return;
        };
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
}
