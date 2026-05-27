use iced::widget::{Column, Row};
use iced::{Alignment, Length};

use coincube_ui::component::{badge, button, card, separation, text::*};
use coincube_ui::{
    icon,
    theme,
    widget::Element,
};

use crate::app::cache;
use crate::app::menu::Menu;
use crate::app::state::settings::coin_cube_firmware::CoinCubeFirmwareState;
use crate::app::view::dashboard;
use crate::app::view::message::*;

pub fn coin_cube_firmware_section<'a>(
    menu: &'a Menu,
    cache: &'a cache::Cache,
    state: &'a CoinCubeFirmwareState,
) -> Element<'a, Message> {
    let mut col = Column::new()
        .spacing(20)
        .push(super::header("CoinCube Firmware", SettingsMessage::CoinCubeFirmwareSection));

    if state.loading {
        col = col.push(loading_card());
    } else if let Some(info) = &state.info {
        if let Some(device) = &info.device {
            col = col.push(device_info_card(device));
            col = col.push(update_card(device, info));
        } else {
            col = col.push(no_device_card(info.device_error.as_deref()));
        }
    } else {
        col = col.push(loading_card());
    }

    dashboard(menu, cache, col)
}

fn loading_card<'a>() -> Element<'a, Message> {
    card::simple(
        Row::new()
            .spacing(20)
            .align_y(Alignment::Center)
            .padding(40)
            .push(
                text("Checking device and fetching latest firmware…")
                    .style(theme::text::secondary),
            ),
    )
    .width(Length::Fill)
    .into()
}

fn no_device_card<'a>(error: Option<&str>) -> Element<'a, Message> {
    let mut col = Column::new()
        .spacing(10)
        .push(
            Row::new()
                .push(badge::badge(icon::chip_icon()))
                .push(text("No CoinCube Connected").bold())
                .padding(10)
                .spacing(20)
                .align_y(Alignment::Center)
                .width(Length::Fill),
        )
        .push(separation().width(Length::Fill))
        .push(
            text("Connect your CoinCube hardware wallet via USB to manage firmware.")
                .style(theme::text::secondary),
        );

    if let Some(e) = error {
        col = col.push(
            text(format!("Details: {}", e))
                .size(12)
                .style(theme::text::secondary),
        );
    }

    col = col.push(
        Row::new().padding(10).push(
            button::secondary(None, "Retry")
                .on_press(Message::Settings(SettingsMessage::CoinCubeFirmwareEnter)),
        ),
    );

    card::simple(col).width(Length::Fill).into()
}

fn device_info_card<'a>(
    device: &'a crate::app::state::settings::coin_cube_firmware::CoinCubeInfo,
) -> Element<'a, Message> {
    let mut col = Column::new()
        .spacing(8)
        .push(
            Row::new()
                .push(badge::badge(icon::chip_icon()))
                .push(text("Connected CoinCube").bold())
                .padding(10)
                .spacing(20)
                .align_y(Alignment::Center)
                .width(Length::Fill),
        )
        .push(separation().width(Length::Fill));

    col = col
        .push(info_row("Firmware Version", &device.version))
        .push(info_row("SE Fingerprint (Serial)", &device.fingerprint));

    if device.is_dev {
        col = col.push(
            Row::new()
                .padding(10)
                .push(
                    badge::badge(
                        text("DEV BUILD")
                            .size(11)
                            .style(theme::text::warning),
                    )
                )
                .push(
                    text("This is a development build with WiFi OTA enabled.")
                        .size(13)
                        .style(theme::text::secondary),
                )
                .spacing(10)
                .align_y(Alignment::Center),
        );
    }

    card::simple(col).width(Length::Fill).into()
}

fn update_card<'a>(
    device: &'a crate::app::state::settings::coin_cube_firmware::CoinCubeInfo,
    info: &'a crate::app::state::settings::coin_cube_firmware::CoinCubeFirmwareInfo,
) -> Element<'a, Message> {
    let mut col = Column::new()
        .spacing(8)
        .push(
            Row::new()
                .push(badge::badge(icon::reload_icon()))
                .push(text("Firmware Update").bold())
                .padding(10)
                .spacing(20)
                .align_y(Alignment::Center)
                .width(Length::Fill),
        )
        .push(separation().width(Length::Fill));

    match &info.latest {
        Some(latest) => {
            let update_available = device.version != latest.version;

            if update_available {
                col = col.push(
                    Row::new()
                        .padding(10)
                        .spacing(10)
                        .align_y(Alignment::Center)
                        .push(
                            badge::badge(
                                text("UPDATE AVAILABLE")
                                    .size(11)
                                    .style(theme::text::warning),
                            )
                        )
                        .push(
                            text(format!(
                                "New firmware v{} is available (you are on v{}).",
                                latest.version, device.version
                            ))
                            .size(14),
                        ),
                );
            } else {
                col = col.push(
                    Row::new()
                        .padding(10)
                        .push(
                            text("You are running the latest firmware.")
                                .size(14)
                                .style(theme::text::secondary),
                        ),
                );
            }

            col = col.push(info_row("Latest Version", &latest.version));

            if let Some(changelog) = &latest.changelog_url {
                col = col.push(
                    Row::new().padding(10).push(
                        button::secondary(None, "View Changelog")
                            .on_press(Message::OpenUrl(changelog.clone())),
                    ),
                );
            }

            if update_available {
                col = col.push(separation().width(Length::Fill));
                col = col.push(
                    Row::new()
                        .padding(10)
                        .push(text("Upgrade Instructions").bold()),
                );

                if device.is_dev {
                    col = col.push(dev_upgrade_instructions(&latest.url));
                } else {
                    col = col.push(prod_upgrade_instructions(&latest.url, &latest.sha256));
                }
            }
        }
        None => {
            if let Some(err) = &info.network_error {
                col = col.push(
                    text(format!(
                        "Could not check for updates: {}",
                        err
                    ))
                    .size(14)
                    .style(theme::text::secondary),
                );
            } else {
                col = col.push(
                    text("Fetching latest firmware version…")
                        .size(14)
                        .style(theme::text::secondary),
                );
            }
        }
    }

    col = col.push(
        Row::new().padding(10).push(
            button::secondary(None, "Refresh")
                .on_press(Message::Settings(SettingsMessage::CoinCubeFirmwareEnter)),
        ),
    );

    card::simple(col).width(Length::Fill).into()
}

fn dev_upgrade_instructions<'a>(url: &str) -> Element<'a, Message> {
    Column::new()
        .spacing(8)
        .padding(10)
        .push(
            text("Development build detected — WiFi OTA is enabled.")
                .size(14)
                .style(theme::text::secondary),
        )
        .push(
            text("Update via the PlatformIO OTA task, or reflash using:")
                .size(13)
                .style(theme::text::secondary),
        )
        .push(
            text("pio run -t upload --upload-port /dev/cu.usbmodem*")
                .size(12)
                .style(theme::text::secondary),
        )
        .push(
            Row::new()
                .spacing(10)
                .push(
                    button::secondary(None, "Download Firmware .bin")
                        .on_press(Message::OpenUrl(url.to_string())),
                ),
        )
        .into()
}

fn prod_upgrade_instructions<'a>(url: &str, sha256: &'a str) -> Element<'a, Message> {
    Column::new()
        .spacing(8)
        .padding(10)
        .push(
            text("Production build — no over-the-air update available.")
                .size(14)
                .style(theme::text::secondary),
        )
        .push(
            text("To update your CoinCube firmware, download the .bin file below and flash it using PlatformIO:")
                .size(13)
                .style(theme::text::secondary),
        )
        .push(
            text("1. Install PlatformIO on your computer")
                .size(12)
                .style(theme::text::secondary),
        )
        .push(
            text("2. Connect your CoinCube via USB-C")
                .size(12)
                .style(theme::text::secondary),
        )
        .push(
            text("3. Run: pio run -t upload --upload-port /dev/cu.usbmodem*")
                .size(12)
                .style(theme::text::secondary),
        )
        .push(info_row("SHA-256", sha256))
        .push(
            Row::new()
                .spacing(10)
                .push(
                    button::secondary(None, "Download Firmware .bin")
                        .on_press(Message::OpenUrl(url.to_string())),
                ),
        )
        .into()
}

fn info_row<'a>(label: &'a str, value: &'a str) -> Element<'a, Message> {
    Row::new()
        .push(
            Column::new()
                .push(text(label).bold().size(13))
                .width(Length::FillPortion(1)),
        )
        .push(
            text(value)
                .size(13)
                .style(theme::text::secondary)
                .width(Length::FillPortion(2)),
        )
        .padding([6, 10])
        .spacing(20)
        .into()
}
