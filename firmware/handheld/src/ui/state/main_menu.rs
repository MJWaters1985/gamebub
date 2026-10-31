use std::{cell::RefCell, rc::Rc};

use super::super::slint::Backend;
use slint::{ComponentHandle, Global};

use crate::{device::Device, worker};

use super::UiState;

impl UiState {
    /// Set up the "Main Menu" screen.
    pub(super) fn setup_main_menu(&mut self, _state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        let root_ = self.root.clone();
        backend.on_main_menu_run_cartridge(move || {
            Backend::get(&root_.unwrap()).set_game_can_load_rom(false);
            worker::send(worker::Message::RunCartridge);
        });
    }
}
