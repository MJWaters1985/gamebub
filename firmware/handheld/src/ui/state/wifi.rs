use std::{cell::RefCell, rc::Rc};

use super::super::slint::Backend;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{device::Device, net, wifi_setup};

use super::UiState;

/// State of the Wi-Fi screen.
#[derive(Default)]
pub struct WifiState {
    networks: Vec<net::Network>,
    ssid: String,
    password: String,
}

impl UiState {
    /// Set up the Wi-Fi screen (Settings > Wi-Fi > Network).
    pub(super) fn setup_wifi(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        let state_ = state.clone();
        backend.on_wifi_open(move || state_.borrow_mut().wifi_show_menu());
        backend.on_wifi_scan(|| wifi_setup::scan());
        backend.on_wifi_test(|| wifi_setup::test());
        backend.on_wifi_forget(|| wifi_setup::forget());

        let state_ = state.clone();
        backend.on_wifi_select_network(move |i| state_.borrow_mut().wifi_select_network(i as usize));

        let state_ = state.clone();
        backend.on_wifi_typed(move |text| {
            let mut state = state_.borrow_mut();
            if state.wifi.password.chars().count() < 63 {
                state.wifi.password.push_str(&text);
            }
            state.wifi_update_password();
        });

        let state_ = state.clone();
        backend.on_wifi_deleted(move || {
            let mut state = state_.borrow_mut();
            state.wifi.password.pop();
            state.wifi_update_password();
        });

        let state_ = state.clone();
        backend.on_wifi_join(move || {
            let state = state_.borrow();
            wifi_setup::join(state.wifi.ssid.clone(), state.wifi.password.clone());
        });

        backend.on_boxart_download_all(|| wifi_setup::download_library_boxart());
        backend.on_sd_setup(|erase| crate::sd_setup::start(erase));
        backend.on_sd_check(|| crate::sd_setup::start_check());
    }

    fn wifi_show_menu(&mut self) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_wifi_status(wifi_setup::status().into());
        backend.set_wifi_has_network(net::saved_network().is_some());
        backend.set_wifi_view(0);
    }

    fn wifi_select_network(&mut self, index: usize) {
        let Some(network) = self.wifi.networks.get(index).cloned() else {
            return;
        };
        self.wifi.ssid = network.ssid.clone();
        self.wifi.password.clear();
        if network.secured {
            let root = self.root.unwrap();
            let backend = root.global::<Backend>();
            backend.set_wifi_selected_ssid(network.ssid.into());
                self.wifi_update_password();
            backend.set_wifi_view(3);
        } else {
            wifi_setup::join(network.ssid, String::new());
        }
    }

    fn wifi_update_password(&mut self) {
        let root = self.root.unwrap();
        root.global::<Backend>().set_wifi_password(self.wifi.password.clone().into());
    }

    /// Show an update from the Wi-Fi background work.
    pub fn wifi_update(&mut self, update: wifi_setup::WifiUpdate) {
        use wifi_setup::WifiUpdate;
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        match update {
            WifiUpdate::Busy(label) => {
                backend.set_wifi_busy_label(label.into());
                backend.set_wifi_view(1);
            }
            WifiUpdate::Networks(networks) => {
                let labels: Vec<SharedString> = networks
                    .iter()
                    .map(|n| {
                        let bars = match n.rssi {
                            -60..=0 => "strong",
                            -72..=-61 => "good",
                            _ => "weak",
                        };
                        let lock = if n.secured { "" } else { ", open" };
                        format!("{}  ({bars}{lock})", n.ssid).into()
                    })
                    .collect();
                self.wifi.networks = networks;
                backend.set_wifi_networks(ModelRc::from(Rc::new(VecModel::from(labels))));
                backend.set_wifi_view(2);
            }
            WifiUpdate::Result { ok, message } => {
                // Don't keep the typed password around.
                self.wifi.password.clear();
                self.wifi_update_password();
                backend.set_wifi_status(wifi_setup::status().into());
                backend.set_wifi_has_network(net::saved_network().is_some());
                backend.set_wifi_result(message.into());
                backend.set_wifi_result_ok(ok);
                backend.set_wifi_view(4);
            }
        }
    }

    /// Show an update from a Tools task.
    pub fn task_update(&mut self, update: wifi_setup::TaskUpdate) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        match update.result {
            Some((ok, message)) => {
                // Set Up SD Card can mount a card that wasn't usable before.
                backend.set_sdcard_mounted(std::fs::exists("/sdcard").unwrap_or(false));
                backend.set_sdcard_status(crate::sd_setup::status().into());
                backend.set_task_result(message.into());
                backend.set_task_result_ok(ok);
                backend.set_task_done(true);
            }
            None => {
                backend.set_task_label(update.label.into());
                backend.set_task_progress(if update.progress < 0.0 { -1.0 } else { update.progress * 100.0 });
                backend.set_task_done(false);
            }
        }
    }
}
