//! Background work for Settings > Wi-Fi > Network (scan, join, test) and Tools > Download Box
//! Art. Each runs on its own short-lived thread and reports to the UI.

use crate::{boxart, kvs, net, ui};

/// Updates for the Wi-Fi screen.
#[derive(Debug, Clone)]
pub enum WifiUpdate {
    Busy(String),
    Networks(Vec<net::Network>),
    Result { ok: bool, message: String },
}

/// Updates for a Tools task (box art download).
#[derive(Debug, Clone)]
pub struct TaskUpdate {
    pub label: String,
    /// 0-1, or negative if unknown
    pub progress: f32,
    /// Set when the task has finished
    pub result: Option<(bool, String)>,
}

fn send(update: WifiUpdate) {
    let update = match update {
        // Include memory figures with errors: memory is the most likely problem.
        WifiUpdate::Result { ok: false, message } => {
            WifiUpdate::Result { ok: false, message: format!("{message}{}", memory_note()) }
        }
        other => other,
    };
    ui::send(ui::Message::Wifi(update));
}

fn heap() -> (usize, usize) {
    unsafe {
        use esp_idf_svc::sys::{heap_caps_get_free_size, heap_caps_get_largest_free_block, MALLOC_CAP_8BIT};
        (heap_caps_get_free_size(MALLOC_CAP_8BIT), heap_caps_get_largest_free_block(MALLOC_CAP_8BIT))
    }
}

fn memory_note() -> String {
    let (free, largest) = heap();
    format!("\n\nFree memory: {} KB (largest block {} KB)", free / 1024, largest / 1024)
}

fn spawn(_name: &str, f: impl FnOnce() + Send + 'static) {
    crate::jobs::run(f);
}

/// Status line for the Wi-Fi screen.
pub fn status() -> String {
    match net::saved_network() {
        Some((ssid, _)) => format!("Network: {ssid}"),
        None => "No network set up".to_string(),
    }
}

pub fn scan() {
    send(WifiUpdate::Busy("Looking for networks...".into()));
    spawn("wifi_scan", || {
        match net::Wifi::start().and_then(|mut wifi| wifi.scan()) {
            Ok(networks) if networks.is_empty() => send(WifiUpdate::Result {
                ok: false,
                message: "No networks found.\nGame Bub can only use 2.4 GHz networks.".into(),
            }),
            Ok(networks) => send(WifiUpdate::Networks(networks)),
            Err(e) => send(WifiUpdate::Result { ok: false, message: format!("{e:#}") }),
        }
    });
}

/// Connect to a network; if it works, save it.
pub fn join(ssid: String, password: String) {
    send(WifiUpdate::Busy(format!("Connecting to {ssid}...")));
    spawn("wifi_join", move || {
        let result = (|| -> anyhow::Result<String> {
            let mut wifi = net::Wifi::start()?;
            wifi.connect(&ssid, &password)?;
            kvs::keys::WIFI_SSID.set(&ssid);
            kvs::keys::WIFI_PASSWORD.set(&password);
            kvs::keys::flush_all();
            Ok(clock_note(&mut wifi))
        })();
        match result {
            Ok(note) => send(WifiUpdate::Result {
                ok: true,
                message: format!("Connected to {ssid}. The network is saved.{note}"),
            }),
            Err(e) => send(WifiUpdate::Result { ok: false, message: format!("{e:#}") }),
        }
    });
}

/// Connect to the saved network to check that it works.
pub fn test() {
    send(WifiUpdate::Busy("Connecting...".into()));
    spawn("wifi_test", || {
        // connect_saved also sets the clock if that's enabled; report it.
        let result = (|| -> anyhow::Result<String> {
            let Some((ssid, password)) = net::saved_network() else {
                anyhow::bail!("No network is set up yet.");
            };
            let mut wifi = net::Wifi::start()?;
            wifi.connect(&ssid, &password)?;
            Ok(format!("Connected to {ssid}.{}", clock_note(&mut wifi)))
        })();
        match result {
            Ok(message) => send(WifiUpdate::Result { ok: true, message }),
            Err(e) => send(WifiUpdate::Result { ok: false, message: format!("{e:#}") }),
        }
    });
}

/// Sync the clock if that setting is on, and describe the result.
fn clock_note(wifi: &mut net::Wifi) -> String {
    if !kvs::keys::WIFI_SYNC_CLOCK.get().unwrap_or(true) {
        return String::new();
    }
    match wifi.sync_clock() {
        Ok(now) => format!(
            "\n\nClock set to {:04}-{:02}-{:02} {:02}:{:02} UTC.",
            now.year(),
            now.month() as u8,
            now.day(),
            now.hour(),
            now.minute()
        ),
        Err(e) => format!("\n\nThe clock wasn't set: {e:#}"),
    }
}

pub fn forget() {
    kvs::keys::WIFI_SSID.set(&String::new());
    kvs::keys::WIFI_PASSWORD.set(&String::new());
    kvs::keys::flush_all();
    send(WifiUpdate::Result { ok: true, message: "The Wi-Fi network was forgotten.".into() });
}

/// Download box art for every game in /roms that doesn't have it.
pub fn download_library_boxart() {
    let task = |label: &str, progress: f32, result: Option<(bool, String)>| {
        ui::send(ui::Message::Task(TaskUpdate { label: label.to_string(), progress, result }));
    };
    task("Looking for games without box art...", -1.0, None);
    spawn("boxart", move || {
        let games = boxart::missing_in_library();
        if games.is_empty() {
            return task("", 1.0, Some((true, "Every game in /roms already has box art.".into())));
        }
        match boxart::fetch(&games, &mut |label, p| task(label, p, None)) {
            Ok(r) => {
                let mut message = format!("Box art added for {} of {} games.", r.found, games.len());
                if r.not_found > 0 {
                    message += &format!("\n{} not found in the box art library.", r.not_found);
                }
                if r.failed > 0 {
                    message += &format!("\n{} failed (try again later).", r.failed);
                }
                task("", 1.0, Some((r.failed == 0, message)));
            }
            Err(e) => task("", 1.0, Some((false, format!("{e:#}")))),
        }
    });
}
