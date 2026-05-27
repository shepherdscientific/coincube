use std::fmt;
use std::sync::Arc;

use coincube_ui::widget::Element;
use iced::Task;
use serde::Deserialize;

use crate::app::cache::Cache;
use crate::app::menu::Menu;
use crate::app::message::Message;
use crate::app::state::State;
use crate::app::view;
use crate::coincube_hw::CoinCubeDevice;
use crate::daemon::Daemon;
use crate::app::wallet::Wallet;
use crate::services::coincube_api_base_url;

#[derive(Debug, Clone, Deserialize)]
pub struct LatestFirmware {
    pub version: String,
    pub sha256: String,
    pub url: String,
    /// Optional changelog / release-notes URL.
    pub changelog_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CoinCubeInfo {
    /// 8-hex master fingerprint.
    pub fingerprint: String,
    /// Firmware version reported by the device.
    pub version: String,
    /// Whether this is a dev build (has a prerelease suffix like -dev, -alpha).
    pub is_dev: bool,
}

#[derive(Debug, Clone)]
pub struct CoinCubeFirmwareInfo {
    /// Info from the locally connected device, if any.
    pub device: Option<CoinCubeInfo>,
    /// Latest firmware metadata fetched from the API, if available.
    pub latest: Option<LatestFirmware>,
    /// Fetch error for the device scan.
    pub device_error: Option<String>,
    /// Fetch error for the API call.
    pub network_error: Option<String>,
}

pub struct CoinCubeFirmwareState {
    pub info: Option<CoinCubeFirmwareInfo>,
    pub loading: bool,
    /// True after the initial load completes — distinguishes "never loaded"
    /// from "loaded with no device found".
    pub loaded: bool,
}

impl fmt::Debug for CoinCubeFirmwareState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoinCubeFirmwareState")
            .field("loading", &self.loading)
            .field("loaded", &self.loaded)
            .finish()
    }
}

impl Default for CoinCubeFirmwareState {
    fn default() -> Self {
        Self {
            info: None,
            loading: false,
            loaded: false,
        }
    }
}

impl CoinCubeFirmwareState {
    /// Spawns the two async fetches: device scan and latest-firmware API call.
    fn start_load(&mut self) -> Task<Message> {
        if self.loading {
            return Task::none();
        }
        self.loading = true;
        Task::perform(
            async move {
                let device_fut = query_connected_device();
                let api_fut = fetch_latest_firmware();
                let (device_result, api_result) = tokio::join!(device_fut, api_fut);

                let (device, device_error) = match device_result {
                    Ok(info) => (Some(info), None),
                    Err(e) => (None, Some(e)),
                };
                let (latest, network_error) = match api_result {
                    Ok(fw) => (Some(fw), None),
                    Err(e) => (None, Some(e)),
                };

                CoinCubeFirmwareInfo {
                    device,
                    latest,
                    device_error,
                    network_error,
                }
            },
            |info| {
                Message::View(view::Message::Settings(
                    view::SettingsMessage::CoinCubeFirmwareLoaded(Ok(info)),
                ))
            },
        )
    }
}

impl State for CoinCubeFirmwareState {
    fn view<'a>(&'a self, menu: &'a Menu, cache: &'a Cache) -> Element<'a, view::Message> {
        crate::app::view::settings::coin_cube_firmware::coin_cube_firmware_section(
            menu,
            cache,
            self,
        )
    }

    fn update(
        &mut self,
        _daemon: Option<Arc<dyn Daemon + Sync + Send>>,
        _cache: &Cache,
        message: Message,
    ) -> Task<Message> {
        match message {
            Message::View(view::Message::Settings(
                view::SettingsMessage::CoinCubeFirmwareEnter,
            )) => self.start_load(),
            Message::View(view::Message::Settings(
                view::SettingsMessage::CoinCubeFirmwareLoaded(res),
            )) => {
                self.loading = false;
                self.loaded = true;
                match res {
                    Ok(info) => self.info = Some(info),
                    Err(e) => {
                        self.info = Some(CoinCubeFirmwareInfo {
                            device: None,
                            latest: None,
                            device_error: Some(e),
                            network_error: None,
                        });
                    }
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    fn reload(
        &mut self,
        _daemon: Option<Arc<dyn Daemon + Sync + Send>>,
        _wallet: Option<Arc<Wallet>>,
    ) -> Task<Message> {
        self.info = None;
        self.loaded = false;
        self.loading = false;
        self.start_load()
    }
}

impl From<CoinCubeFirmwareState> for Box<dyn State> {
    fn from(s: CoinCubeFirmwareState) -> Box<dyn State> {
        Box::new(s)
    }
}

/// Query the locally connected CoinCube device to get its firmware version.
async fn query_connected_device() -> Result<CoinCubeInfo, String> {
    let ports = CoinCubeDevice::enumerate_ports().map_err(|e| format!("Port enumeration failed: {}", e))?;

    if ports.is_empty() {
        return Err("No CoinCube device found. Connect your CoinCube via USB and try again.".to_string());
    }

    // Try each port; take the first that responds.
    for port in ports {
        // Use a short timeout — the panel is interactive, don't block long.
        let device = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            CoinCubeDevice::new(&port),
        )
        .await
        .map_err(|_| "Timeout connecting to device".to_string())?
        .map_err(|e| format!("Failed to open device on {}: {}", port, e))?;

        let version = device.version();
        let fingerprint = hex::encode(device.fingerprint().to_bytes());
        let version_str = version.to_string();

        // Dev builds carry a prerelease suffix like "-dev" or "-alpha".
        let is_dev = version.prerelease.as_ref().map_or(false, |pre| {
            pre.contains("dev") || pre.contains("alpha") || pre.contains("beta")
        });

        return Ok(CoinCubeInfo {
            fingerprint,
            version: version_str,
            is_dev,
        });
    }

    Err("No CoinCube device responded. Check the USB connection.".to_string())
}

/// Fetch the latest published firmware version from the Coincube API.
async fn fetch_latest_firmware() -> Result<LatestFirmware, String> {
    let base = coincube_api_base_url();
    let url = format!("{}/api/v1/firmware/coincube/latest", base);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to reach firmware endpoint: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Firmware endpoint returned {}: {}",
            resp.status().as_u16(),
            resp.text().await.unwrap_or_default(),
        ));
    }

    let fw: LatestFirmware = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse firmware response: {}", e))?;

    Ok(fw)
}
