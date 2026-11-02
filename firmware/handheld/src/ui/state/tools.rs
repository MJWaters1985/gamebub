use std::{cell::RefCell, rc::Rc};

use super::super::slint::{Backend, BatteryInfo};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::device::{drivers::usb, Device};

use super::UiState;

impl UiState {
    /// Set up the "Tools" screen.
    pub(super) fn setup_tools(&mut self, _state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        backend.on_tools_start_usb_drive(move || {
            if let Err(e) = usb::configure_usb(usb::UsbMode::ConsoleAndMassStorage) {
                // This has been seen to fail right after a firmware update.
                // Reset the USB stack and try once more before giving up.
                log::error!("USB drive setup failed ({e}), retrying");
                let retry = usb::configure_usb(usb::UsbMode::SerialJtag).and_then(|_| {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    usb::configure_usb(usb::UsbMode::ConsoleAndMassStorage)
                });
                if let Err(e) = retry {
                    log::error!("USB drive setup failed again: {e}");
                    crate::ui::send(crate::ui::Message::Notification(
                        crate::ui::Notification::new_alert(
                            "USB setup failed:\nplease reboot and try again".into(),
                        ),
                    ));
                }
            }
        });
        backend.on_tools_end_usb_drive(move || {
            usb::configure_usb(usb::UsbMode::ConsoleOnly).expect("USB teardown failed");
            Device::lock().reboot();
        });

        backend.on_tools_start_cart_reader(move || {
            usb::configure_usb(usb::UsbMode::ConsoleAndSerial).expect("USB setup failed");
            crate::cart_backup::start_task(1);
        });
        backend.on_tools_end_cart_reader(move || {
            usb::configure_usb(usb::UsbMode::ConsoleOnly).expect("USB teardown failed");
            Device::lock().reboot();
        });

        backend.on_cart_tool_start(|| crate::cart_tool::start());
        backend.on_cart_tool_action(|i| {
            crate::cart_tool::request(crate::cart_tool::Request::Action(i.max(0) as usize))
        });
        backend.on_cart_tool_confirm(|| crate::cart_tool::request(crate::cart_tool::Request::Confirm));
        backend.on_cart_tool_exit(|| crate::cart_tool::request(crate::cart_tool::Request::Exit));

        backend.on_tools_get_battery_info(move || {
            let mut device = Device::lock();
            BatteryInfo {
                is_charging: device.get_battery_is_charging(),
                level: device.fuel_gauge.get_battery_level().unwrap_or(f32::NAN),
                vbus_pgood: device.get_vbus_pgood(),
                voltage: device.fuel_gauge.get_battery_voltage().unwrap_or(f32::NAN),
                current: device.fuel_gauge.get_battery_current().unwrap_or(f32::NAN),
            }
        })
    }
}

impl UiState {
    /// Show an update from the Cartridge Manager.
    pub fn cart_tool_update(&mut self, update: crate::cart_tool::Update) {
        use crate::cart_tool::Update;
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        let strings = |v: Vec<String>| {
            ModelRc::from(Rc::new(VecModel::from(
                v.into_iter().map(SharedString::from).collect::<Vec<_>>(),
            )))
        };
        match update {
            Update::Busy { label, progress } => {
                backend.set_cart_tool_busy_label(label.into());
                backend.set_cart_tool_progress(if progress < 0.0 { -1.0 } else { progress * 100.0 });
                backend.set_cart_tool_view(0);
            }
            Update::Menu { title, details, actions } => {
                backend.set_cart_tool_title(title.into());
                backend.set_cart_tool_details(details.into());
                backend.set_cart_tool_actions(strings(actions));
                backend.set_cart_tool_view(1);
            }
            Update::Confirm { title, text } => {
                backend.set_cart_tool_confirm_title(title.into());
                backend.set_cart_tool_confirm_text(text.into());
                backend.set_cart_tool_view(2);
            }
            Update::Result { ok, message } => {
                backend.set_cart_tool_result(message.into());
                backend.set_cart_tool_result_ok(ok);
                backend.set_cart_tool_view(3);
            }
        }
    }
}
