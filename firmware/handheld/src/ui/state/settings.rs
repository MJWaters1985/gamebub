use std::cell::RefCell;
use std::rc::Rc;

use super::super::slint::Backend;
use crate::device::Device;
use crate::ui::slint::ScreenId;
use crate::ui::state::{SettingDatetime, SettingEntry, SettingType, SettingValue};
use settings::Page;
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker, ToSharedString};
use time::OffsetDateTime;

use super::UiState;

mod settings {
    use crate::display_color;
    use crate::kvs::{keys, KvsKey};
    use crate::ui::state::ScreenId;

    pub struct Page {
        pub name: &'static str,
        pub entries: &'static [Entry],
    }

    pub enum Entry {
        Checkbox {
            name: &'static str,
            key: &'static KvsKey<bool>,
        },
        List {
            name: &'static str,
            key: &'static KvsKey<i32>,
            choices: &'static [&'static str],
        },
        SystemDatetime {
            name: &'static str,
        },
        Subpage {
            name: &'static str,
            page: &'static Page,
        },
        Screen {
            name: &'static str,
            screen: ScreenId,
        },
        Action {
            name: &'static str,
            action: fn(),
        },
    }

    pub static PAGE_ROOT: Page = Page {
        name: "",
        entries: &[
            Entry::Screen {
                name: "About",
                screen: ScreenId::About,
            },
            Entry::Subpage {
                name: "General",
                page: &PAGE_GENERAL,
            },
            Entry::Subpage {
                name: "Display",
                page: &PAGE_DISPLAY,
            },
            Entry::Subpage {
                name: "Wi-Fi",
                page: &PAGE_WIFI,
            },
            Entry::Subpage {
                name: "Dock",
                page: &PAGE_DOCK,
            },
            Entry::Subpage {
                name: "Core: GB / GBC",
                page: &PAGE_CORE_GB,
            },
            Entry::Subpage {
                name: "Core: GBA",
                page: &PAGE_CORE_GBA,
            },
        ],
    };

    pub static PAGE_GENERAL: Page = Page {
        name: "General",
        entries: &[
            Entry::SystemDatetime {
                name: "Date and Time",
            },
            Entry::List {
                name: "Time Zone",
                key: &keys::TIME_ZONE,
                choices: &crate::timezone::ZONE_NAMES,
            },
            Entry::Checkbox {
                name: "Show Clock",
                key: &keys::SHOW_CLOCK,
            },
            Entry::List {
                name: "Clock Format",
                key: &keys::CLOCK_FORMAT,
                choices: &["12-hour", "24-hour"],
            },
            Entry::List {
                name: "Startup Action",
                key: &keys::STARTUP_ACTION,
                choices: &["Main Menu", "Run Cartridge"],
            },
            Entry::List {
                name: "Save Files",
                key: &keys::SAVE_LOCATION,
                choices: &["Next to ROM", "/saves folder"],
            },
            Entry::List {
                name: "Auto Power Off",
                key: &keys::AUTO_POWER_OFF,
                choices: &["Never", "3 min", "5 min", "10 min", "30 min"],
            },
            Entry::Checkbox {
                name: "X/Y as Select/Start",
                key: &keys::XY_SELECT_START,
            },
        ],
    };

    pub static PAGE_DISPLAY: Page = Page {
        name: "Display",
        entries: &[
            Entry::List {
                name: "Color Temperature",
                key: &keys::DISPLAY_TEMPERATURE,
                choices: &display_color::TEMPERATURE_CHOICES,
            },
            Entry::List {
                name: "Saturation",
                key: &keys::DISPLAY_SATURATION,
                choices: &display_color::SATURATION_CHOICES,
            },
            Entry::List {
                name: "Contrast",
                key: &keys::DISPLAY_CONTRAST,
                choices: &display_color::CONTRAST_CHOICES,
            },
            Entry::List {
                name: "Gamma",
                key: &keys::DISPLAY_GAMMA,
                choices: &display_color::GAMMA_CHOICES,
            },
            Entry::List {
                name: "Red",
                key: &keys::DISPLAY_RED,
                choices: &display_color::GAIN_CHOICES,
            },
            Entry::List {
                name: "Green",
                key: &keys::DISPLAY_GREEN,
                choices: &display_color::GAIN_CHOICES,
            },
            Entry::List {
                name: "Blue",
                key: &keys::DISPLAY_BLUE,
                choices: &display_color::GAIN_CHOICES,
            },
            Entry::Action {
                name: "Reset to Defaults",
                action: display_color::reset,
            },
        ],
    };

    pub static PAGE_WIFI: Page = Page {
        name: "Wi-Fi",
        entries: &[
            Entry::Screen {
                name: "Network",
                screen: ScreenId::Wifi,
            },
            Entry::Checkbox {
                name: "Sync Clock on Connect",
                key: &keys::WIFI_SYNC_CLOCK,
            },
            Entry::Checkbox {
                name: "Box Art After Dump",
                key: &keys::BOXART_AFTER_DUMP,
            },
        ],
    };

    pub static PAGE_DOCK: Page = Page {
        name: "Dock",
        entries: &[Entry::Checkbox {
            name: "Swap A/B and X/Y",
            key: &keys::DOCK_SWAP_ABXY,
        }],
    };

    pub static PAGE_CORE_GB: Page = Page {
        name: "Core: GB / GBC",
        entries: &[
            Entry::Checkbox {
                name: "Enable GB mode",
                key: &keys::GB_IS_DMG,
            },
            // Entry::Checkbox {
            //     name: "Skip Boot Animation",
            //     key: &keys::GB_SKIP_BOOT_ANIM,
            // },
            Entry::List {
                name: "GBC Color Corrections",
                key: &keys::CGB_COLOR_PROFILE,
                choices: &["None", "GBC", "GBA", "GBA SP"],
            },
            Entry::List {
                name: "GB Color Palette",
                key: &keys::DMG_COLOR_PALETTE,
                choices: &["Grayscale", "DMG Green", "GB Pocket"],
            },
            Entry::Checkbox {
                name: "Integer Overscale (crop)",
                key: &keys::GB_OVERSCALE,
            },
            Entry::List {
                name: "Tilt Sensitivity",
                key: &keys::TILT_SENSITIVITY,
                choices: &["Lowest", "Low", "Normal", "High"],
            },
        ],
    };

    pub static PAGE_CORE_GBA: Page = Page {
        name: "Core: GBA",
        entries: &[
            // Entry::Checkbox {
            //     name: "Skip Boot Animation",
            //     key: &keys::GBA_SKIP_BOOT_ANIM,
            // },
            Entry::List {
                name: "Color Corrections",
                key: &keys::GBA_COLOR_PROFILE,
                choices: &["None", "GBA", "GBA SP", "NDS", "NDS Lite", "NSO GBA"],
            },
            Entry::Checkbox {
                name: "Enable Game Boy Player",
                key: &keys::GBA_ENABLE_GBP,
            },
            Entry::Checkbox {
                name: "Warn about missing BIOS",
                key: &keys::GBA_BIOS_WARNING,
            },
            Entry::List {
                name: "Tilt Sensitivity",
                key: &keys::TILT_SENSITIVITY,
                choices: &["Lowest", "Low", "Normal", "High"],
            },
        ],
    };
}

pub struct SettingsState {
    model: Option<Rc<SettingsModel>>,
    page: &'static Page,
    stack: Vec<(&'static Page, usize)>,
    /// Page to open when the settings screen is next entered.
    pending_page: Option<&'static Page>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            model: None,
            page: &settings::PAGE_ROOT,
            stack: Vec::new(),
            pending_page: None,
        }
    }
}

impl UiState {
    /// Set up the "Settings" screen.
    pub(super) fn setup_settings(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        let state_ = state.clone();
        backend.on_settings_changed(move |i, value| {
            let mut state = state_.borrow_mut();
            let Some(model) = state.settings.model.as_ref() else {
                return;
            };

            let action = model.changed(i as usize, value);
            match action {
                SettingsAction::None => {}
                SettingsAction::Subpage(page) => {
                    let entry = (state.settings.page, i as usize);
                    state.settings.stack.push(entry);
                    state.set_settings_page(page, 0);
                }
                SettingsAction::Screen(screen_id) => {
                    let root = state.root.unwrap();
                    std::mem::drop(state);
                    root.invoke_set_screen(screen_id);

                    // Workaround for issue where the title doesn't get updated
                    // (it's updated, but it doesn't take effect until the next render)
                    crate::ui::send(crate::ui::Message::Redraw);
                }
            }
        });

        let state_ = state.clone();
        backend.on_settings_back(move || {
            let mut state = state_.borrow_mut();
            match state.settings.stack.pop() {
                Some(previous) => {
                    state.set_settings_page(previous.0, previous.1);
                    true
                }
                None => false,
            }
        })
    }

    fn set_settings_page(&mut self, page: &'static Page, selected_item: usize) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        let model = Rc::new(SettingsModel::new(page));
        self.settings.model = Some(model.clone());
        self.settings.page = page;
        backend.set_settings(ModelRc::from(model));
        backend.set_settings_index(selected_item as i32);

        // Should be in Slint, but there isn't a good way to do it
        let mut title = "Settings".to_shared_string();
        if !page.name.is_empty() {
            title.push_str(": ");
            title.push_str(page.name);
        }
        root.invoke_set_title(title);
    }

    pub(super) fn on_settings_enter(&mut self) {
        self.settings.stack.clear();
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        match self.settings.pending_page.take() {
            Some(page) => {
                backend.set_settings_return_screen(ScreenId::Cores);
                self.set_settings_page(page, 0);
            }
            None => {
                backend.set_settings_return_screen(ScreenId::MainMenu);
                self.set_settings_page(&settings::PAGE_ROOT, 0);
            }
        }
    }

    /// Open the settings for a core next time the settings screen is entered.
    /// Leaving the settings will return to the "Cores" screen.
    pub(super) fn settings_open_for_core(&mut self, core_id: &str) {
        self.settings.pending_page = match core_id {
            "Game-Bub.GB" => Some(&settings::PAGE_CORE_GB),
            "Game-Bub.GBA" => Some(&settings::PAGE_CORE_GBA),
            _ => Some(&settings::PAGE_ROOT),
        };
    }
}

pub struct SettingsModel {
    page: &'static settings::Page,
    notify: ModelNotify,
}

impl Model for SettingsModel {
    type Data = SettingEntry;

    fn row_count(&self) -> usize {
        self.page.entries.len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        let entry = self.page.entries.get(row)?;
        let row = match entry {
            settings::Entry::Checkbox { name, key } => SettingEntry {
                name: (*name).into(),
                r#type: SettingType::Checkbox,
                value: SettingValue {
                    bool_value: key.get().unwrap(),
                    ..SettingValue::default()
                },
                ..Default::default()
            },
            settings::Entry::List { name, key, choices } => SettingEntry {
                name: (*name).into(),
                r#type: SettingType::List,
                value: SettingValue {
                    int_value: key.get().unwrap(),
                    ..SettingValue::default()
                },
                choices: ModelRc::new(
                    choices
                        .iter()
                        .map(|x| slint::SharedString::from(*x))
                        .collect::<slint::VecModel<_>>(),
                ),
            },
            settings::Entry::SystemDatetime { name } => SettingEntry {
                name: (*name).into(),
                r#type: SettingType::Datetime,
                value: SettingValue {
                    datetime_value: {
                        let dt = crate::timezone::to_local(OffsetDateTime::now_utc());
                        SettingDatetime {
                            year: dt.year(),
                            month: dt.month() as i32,
                            day: dt.day() as i32,
                            hour: dt.hour() as i32,
                            min: dt.minute() as i32,
                            sec: dt.second() as i32,
                        }
                    },
                    ..SettingValue::default()
                },
                ..Default::default()
            },
            settings::Entry::Subpage { name, .. } => SettingEntry {
                name: (*name).into(),
                r#type: SettingType::Subpage,
                ..Default::default()
            },
            settings::Entry::Screen { name, .. } => SettingEntry {
                name: (*name).into(),
                r#type: SettingType::Subpage,
                ..Default::default()
            },
            settings::Entry::Action { name, .. } => SettingEntry {
                name: (*name).into(),
                r#type: SettingType::Subpage,
                ..Default::default()
            },
        };
        Some(row)
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }

    fn as_any(&self) -> &dyn core::any::Any {
        // a typical implementation just return `self`
        self
    }
}

impl SettingsModel {
    fn new(page: &'static settings::Page) -> Self {
        SettingsModel {
            page,
            notify: ModelNotify::default(),
        }
    }

    /// Notify that a setting has changed. Returns whether we navigated to a new page.
    fn changed(&self, index: usize, value: SettingValue) -> SettingsAction {
        let Some(entry) = self.page.entries.get(index) else {
            log::info!("Unknown setting changed: {} -> {:?}", index, value);
            return SettingsAction::None;
        };

        match entry {
            settings::Entry::Checkbox { key, .. } => key.set(&value.bool_value),
            settings::Entry::List { key, .. } => {
                key.set(&value.int_value);
                if std::ptr::eq(self.page, &settings::PAGE_DISPLAY) {
                    apply_display_color();
                }
                if std::ptr::eq(*key, &crate::kvs::keys::TIME_ZONE) {
                    // The date and time row shows local time.
                    crate::timezone::apply();
                    self.notify.reset();
                    return SettingsAction::None;
                }
            }
            settings::Entry::SystemDatetime { .. } => {
                let dt = convert_settings_datetime(&value.datetime_value).unwrap();
                let dt = dt.replace_second(0).unwrap();
                let dt = crate::timezone::from_local(dt);
                Device::lock().set_datetime(dt);
            }
            settings::Entry::Subpage { page, .. } => {
                return SettingsAction::Subpage(*page);
            }
            settings::Entry::Screen { screen, .. } => {
                return SettingsAction::Screen(*screen);
            }
            settings::Entry::Action { action, .. } => {
                action();
                if std::ptr::eq(self.page, &settings::PAGE_DISPLAY) {
                    apply_display_color();
                }
                self.notify.reset();
                return SettingsAction::None;
            }
        }
        self.notify.row_changed(index);
        SettingsAction::None
    }
}

/// Apply changed display color settings to the menus (games pick them up when they start).
fn apply_display_color() {
    crate::display_color::apply_to_menus();
    // The boot bitstream (logo and background behind the menus) runs whenever no core does.
    if crate::core::CoreManager::lock().current_bitstream().is_none() {
        crate::bitstream::boot::apply_display_color(&mut Device::lock());
    }
    crate::ui::send(crate::ui::Message::Redraw);
}

enum SettingsAction {
    None,
    Subpage(&'static Page),
    Screen(ScreenId),
}

/// Helper function used by the UI to be able to correctly modify individual datetime components.
pub fn settings_datetime_add(source: SettingDatetime, delta: SettingDatetime) -> SettingDatetime {
    fn inner(
        source: &SettingDatetime,
        delta: SettingDatetime,
    ) -> time::Result<time::PrimitiveDateTime> {
        let mut dt = convert_settings_datetime(source)?;
        dt = dt.replace_day(1)?; // The day will be re-added later.
        dt = dt.replace_year(((dt.year() as i32) + delta.year).min(2100).max(2000))?;
        if delta.month < 0 {
            dt = dt.replace_month(dt.month().nth_prev((-delta.month) as u8))?;
        } else {
            dt = dt.replace_month(dt.month().nth_next(delta.month as u8))?;
        }
        dt = dt.replace_hour(((dt.hour() as i32) + delta.hour).rem_euclid(24) as u8)?;
        dt = dt.replace_minute(((dt.minute() as i32) + delta.min).rem_euclid(60) as u8)?;
        dt = dt.replace_second(((dt.second() as i32) + delta.sec).rem_euclid(60) as u8)?;
        let day_max = time::util::days_in_month(dt.month(), dt.year()) as i32;
        if delta.day == 0 {
            // If we aren't changing the day, clamp it to the maximum days in the month.
            dt = dt.replace_day(source.day.min(day_max) as u8)?;
        } else {
            dt = dt.replace_day((source.day + delta.day - 1).rem_euclid(day_max) as u8 + 1)?;
        }
        Ok(dt)
    }
    match inner(&source, delta) {
        Ok(dt) => SettingDatetime {
            year: dt.year(),
            month: dt.month() as i32,
            day: dt.day() as i32,
            hour: dt.hour() as i32,
            min: dt.minute() as i32,
            sec: dt.second() as i32,
        },
        Err(_) => {
            log::warn!("Invalid date");
            source
        }
    }
}

fn convert_settings_datetime(source: &SettingDatetime) -> time::Result<time::PrimitiveDateTime> {
    let date = time::Date::from_calendar_date(
        source.year,
        (source.month as u8).try_into()?,
        source.day as u8,
    )?;
    let time = time::Time::from_hms(source.hour as u8, source.min as u8, source.sec as u8)?;
    Ok(time::PrimitiveDateTime::new(date, time))
}
