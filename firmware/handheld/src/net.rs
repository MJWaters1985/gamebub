//! Wi-Fi: connect for short tasks (box art downloads, clock sync), then turn the radio off.
//!
//! Memory is tight (about 80 KB in the largest free block), so Wi-Fi is only started when
//! needed, uses plain HTTP (no TLS), and is shut down before any other memory-hungry work.

use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use embedded_svc::http::client::Client;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};
use esp_idf_svc::sntp::{EspSntp, SyncStatus};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};

/// A network found by a scan.
#[derive(Clone, Debug)]
pub struct Network {
    pub ssid: String,
    /// Signal strength (dBm)
    pub rssi: i8,
    pub secured: bool,
}

/// Only one task uses Wi-Fi at a time.
static IN_USE: Mutex<()> = Mutex::new(());

/// The saved network (name, password), if one is set up.
pub fn saved_network() -> Option<(String, String)> {
    let ssid = crate::kvs::keys::WIFI_SSID.get().filter(|s| !s.is_empty())?;
    Some((ssid, crate::kvs::keys::WIFI_PASSWORD.get().unwrap_or_default()))
}

/// Connect to the saved network, and set the clock if that setting is on.
pub fn connect_saved() -> anyhow::Result<Wifi> {
    let Some((ssid, password)) = saved_network() else {
        bail!("Wi-Fi isn't set up yet.\nChoose a network in Settings → Wi-Fi.");
    };
    let mut wifi = Wifi::start()?;
    wifi.connect(&ssid, &password)?;
    if crate::kvs::keys::WIFI_SYNC_CLOCK.get().unwrap_or(true) {
        if let Err(e) = wifi.sync_clock() {
            log::warn!("Clock sync failed: {e:#}");
        }
    }
    Ok(wifi)
}

/// Wi-Fi, started in station mode. Stopped (and its memory freed) when dropped.
pub struct Wifi {
    wifi: BlockingWifi<EspWifi<'static>>,
    _in_use: MutexGuard<'static, ()>,
}

impl Wifi {
    /// Start Wi-Fi. Callers usually come from a screen change, and the old screen's memory is
    /// only freed once the UI has redrawn: wait for that first, and retry if Wi-Fi still can't
    /// get enough memory.
    pub fn start() -> anyhow::Result<Self> {
        std::thread::sleep(Duration::from_millis(300));
        let mut attempt = 0;
        loop {
            match Self::try_start() {
                Ok(wifi) => return Ok(wifi),
                Err(e) if attempt < 2 && format!("{e:#}").contains("NO_MEM") => {
                    log::warn!("Wi-Fi start: {e:#}, retrying");
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn try_start() -> anyhow::Result<Self> {
        let in_use = IN_USE.try_lock().map_err(|_| anyhow::anyhow!("Wi-Fi is busy"))?;
        let sysloop = EspSystemEventLoop::take().context("event loop")?;
        // The modem isn't used by anything else; Wi-Fi is only ever started from one place.
        let modem = unsafe { Modem::new() };
        // No NVS: the "nvs" partition belongs to the settings store.
        let esp_wifi = EspWifi::new(modem, sysloop.clone(), None).context("Wi-Fi init")?;
        let mut wifi = BlockingWifi::wrap(esp_wifi, sysloop).context("Wi-Fi init")?;
        wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
        wifi.start().context("Wi-Fi start")?;
        Ok(Wifi { wifi, _in_use: in_use })
    }

    /// Scan for networks, strongest first (one entry per network name).
    pub fn scan(&mut self) -> anyhow::Result<Vec<Network>> {
        let mut networks: Vec<Network> = vec![];
        for ap in self.wifi.scan().context("Wi-Fi scan")? {
            let ssid = ap.ssid.to_string();
            if ssid.is_empty() || networks.iter().any(|n| n.ssid == ssid) {
                continue;
            }
            networks.push(Network {
                ssid,
                rssi: ap.signal_strength,
                secured: ap.auth_method.map_or(false, |m| m != AuthMethod::None),
            });
        }
        networks.sort_by(|a, b| b.rssi.cmp(&a.rssi));
        Ok(networks)
    }

    /// Connect to a network and wait for an IP address.
    pub fn connect(&mut self, ssid: &str, password: &str) -> anyhow::Result<()> {
        let config = ClientConfiguration {
            ssid: ssid.try_into().map_err(|_| anyhow::anyhow!("Network name is too long"))?,
            password: password.try_into().map_err(|_| anyhow::anyhow!("Password is too long"))?,
            auth_method: if password.is_empty() { AuthMethod::None } else { AuthMethod::WPA2Personal },
            ..Default::default()
        };
        self.wifi.set_configuration(&Configuration::Client(config))?;
        self.wifi.connect().context("Couldn't connect (check the network name and password)")?;
        self.wifi.wait_netif_up().context("Couldn't get a network address")?;
        Ok(())
    }

    /// Set the clock from the internet (SNTP). Returns the time set.
    pub fn sync_clock(&mut self) -> anyhow::Result<time::OffsetDateTime> {
        let sntp = EspSntp::new_default().context("SNTP")?;
        let start = Instant::now();
        while sntp.get_sync_status() != SyncStatus::Completed {
            if start.elapsed() > Duration::from_secs(15) {
                bail!("Couldn't get the time from the internet");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let now = time::OffsetDateTime::from(std::time::SystemTime::now());
        crate::device::Device::lock().set_datetime(now);
        Ok(now)
    }

    /// Download a URL (plain HTTP) to a file. Returns the size, or None if not found (404).
    pub fn download(&mut self, url: &str, path: &Path) -> anyhow::Result<Option<u64>> {
        let connection = EspHttpConnection::new(&HttpConfiguration {
            buffer_size: Some(1024),
            buffer_size_tx: Some(512),
            timeout: Some(Duration::from_secs(15)),
            ..Default::default()
        })
        .context("HTTP client")?;
        let mut client = Client::wrap(connection);
        let request = client.get(url).context("HTTP request")?;
        let mut response = request.submit().context("HTTP request")?;
        match response.status() {
            200 => {}
            404 => return Ok(None),
            status => bail!("The server answered {status}"),
        }
        let temp = crate::core::temp_path(path);
        let mut file = std::fs::File::create(&temp).with_context(|| format!("Couldn't create {}", temp.display()))?;
        let mut buf = [0u8; 1024];
        let mut total = 0u64;
        loop {
            let n = response.read(&mut buf).map_err(|e| anyhow::anyhow!("Download failed: {e:?}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).context("Couldn't write the download")?;
            total += n as u64;
        }
        file.sync_all()?;
        drop(file);
        let _ = std::fs::remove_file(path);
        std::fs::rename(&temp, path)?;
        Ok(Some(total))
    }
}

impl Drop for Wifi {
    fn drop(&mut self) {
        let _ = self.wifi.disconnect();
        let _ = self.wifi.stop();
    }
}
