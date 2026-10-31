use std::{cell::RefCell, rc::Rc, time::Duration};

use super::super::slint::Backend;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::{
    device::Device,
    kvs,
    ui::slint::{CoreSubscreen, FileIcon, ScreenId},
    worker,
};

use super::UiState;

struct FileListModel {
    data: Vec<(String, bool)>,
}

impl Model for FileListModel {
    type Data = crate::ui::slint::FileListEntry;

    fn row_count(&self) -> usize {
        self.data.len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        if row >= self.data.len() {
            return None;
        }
        let (name, is_dir) = &self.data[row];
        Some(crate::ui::slint::FileListEntry {
            name: SharedString::from(name),
            icon: if *is_dir {
                FileIcon::Folder
            } else {
                FileIcon::Blank
            },
        })
    }

    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        &()
    }
}

impl UiState {
    /// Set up the "Cores" screen.
    pub(super) fn setup_cores(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        backend.on_core_list_fetch(move || {
            worker::send(worker::Message::FetchCoreList);
        });

        let state_ = state.clone();
        backend.on_core_run(move |core_id| {
            let mut state = state_.borrow_mut();
            state.cores_handle_run(core_id);
        });

        backend.on_core_file_select_selected(move |index| {
            worker::send(worker::Message::CoreFileChoose(index as usize));
        });

        backend.on_core_file_select_up(move || {
            worker::send(worker::Message::CoreFileUp);
        });

        backend.on_core_file_select_favorite(move |index| {
            worker::send(worker::Message::CoreFileToggleFavorite(index as usize));
        });

        let state_ = state.clone();
        backend.on_core_file_select_cancel(move || {
            let state = state_.borrow();
            let root = state.root.unwrap();
            root.invoke_set_title("Cores".into());
            root.global::<Backend>().set_core_file_select_active(false);
            worker::send(worker::Message::CoreFileCancelled);
        });

        let state_ = state.clone();
        backend.on_core_file_select_settings(move || {
            let core_id = kvs::keys::LAST_CORE.get().unwrap_or_default();
            let root = {
                let mut state = state_.borrow_mut();
                state.settings_open_for_core(&core_id);
                state.root.unwrap()
            };
            root.invoke_set_screen(ScreenId::Settings);
        });
    }

    pub fn cores_handle_run(&mut self, core_id: SharedString) {
        let core_id = core_id.to_string();
        kvs::keys::LAST_CORE.set(&core_id);
        let root = self.root.unwrap();
        root.global::<Backend>().set_game_can_load_rom(true);
        worker::send(worker::Message::RunCore(core_id));
    }

    pub fn cores_list(&mut self, list: Vec<crate::core::CoreListEntry>) {
        let last_core = kvs::keys::LAST_CORE.get();
        let selected_index = last_core
            .and_then(|last_core| list.iter().position(|x| x.id.as_str() == last_core))
            .unwrap_or(0);

        let list = ModelRc::from(Rc::new(VecModel::from(
            list.into_iter()
                .map(|item| crate::ui::slint::CoreListEntry {
                    core_id: item.id.as_str().into(),
                    core_name: item.name.as_str().into(),
                    core_author: item.author.as_str().into(),
                })
                .collect::<Vec<_>>(),
        )));
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_core_list_index(selected_index as i32);
        backend.set_core_list(list);
    }

    pub fn cores_file_select_begin(&mut self, label: String) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        root.invoke_set_title(format!("Select {label}").into());
        backend.set_core_file_select_label(label.into());
        backend.set_core_file_select_list(slint::ModelRc::default());
        backend.set_core_file_select_index(-1);
        backend.set_core_file_select_is_loading(true);
        backend.set_core_file_select_active(true);
        backend.set_core_subscreen(CoreSubscreen::FileSelect);
    }

    pub fn cores_file_select_list(
        &mut self,
        path: String,
        files: Vec<(String, bool)>,
        selected: usize,
    ) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_core_file_select_path(path.into());
        let files = ModelRc::new(FileListModel { data: files });
        backend.set_core_file_select_list(files);

        {
            // Defer setting the index and unsetting loading, because
            // the FileListView component can't handle shifting view to
            // a newly selected element until the first time it renders.
            let root = self.root.clone();
            self.core_file_select_timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(1),
                move || {
                    let root = root.unwrap();
                    let backend = root.global::<Backend>();
                    // Force a change notification, even if the index is the same.
                    backend.set_core_file_select_index(-1);
                    backend.set_core_file_select_index(selected as i32);
                    backend.set_core_file_select_is_loading(false);
                },
            );
        }
    }

    pub fn cores_file_select_set_error(&mut self, error: String) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_core_file_select_is_loading(false);
        backend.set_core_file_select_error(error.into());
    }

    pub fn cores_set_error(&mut self, error: String) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_core_error(error.into());
        backend.set_core_file_select_active(false);
        backend.set_core_subscreen(CoreSubscreen::Error);
        root.invoke_set_screen(ScreenId::Cores);
    }
}
