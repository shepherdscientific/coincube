use iced::Length;

use coincube_ui::{
    component::hw,
    image,
    theme,
    widget::{Button, Element, Row},
};

use crate::{
    app::view::message::*,
    hw::{HardwareWallet, UnsupportedReason},
};
use async_hwi::DeviceKind;

fn is_coin_cube(hw: &HardwareWallet) -> bool {
    hw.id().starts_with("coincube-")
}

fn wrap_coin_cube_icon<'a>(
    hw: &HardwareWallet,
    content: impl Into<Element<'a, Message>>,
) -> Row<'a, Message> {
    if is_coin_cube(hw) {
        Row::new()
            .push(
                image::coin_cube_hw_icon()
                    .width(Length::Fixed(32.0))
                    .height(Length::Fixed(32.0)),
            )
            .push(iced::widget::Space::new().width(Length::Fixed(8.0)).height(Length::Fixed(1.0)))
            .push(content)
    } else {
        Row::new().push(content)
    }
}

pub fn hw_list_view(
    i: usize,
    hw: &HardwareWallet,
    signed: bool,
    signing: bool,
    can_sign: bool,
) -> Element<'_, Message> {
    let kind_label = hw.display_name();
    let content = match hw {
        HardwareWallet::Supported {
            version,
            fingerprint,
            alias,
            registered,
            ..
        } => {
            if signing {
                hw::processing_hardware_wallet(&kind_label, version.as_ref(), fingerprint, alias.as_ref())
            } else if signed {
                hw::sign_success_hardware_wallet(
                    &kind_label,
                    version.as_ref(),
                    fingerprint,
                    alias.as_ref(),
                )
            } else if *registered == Some(false) {
                hw::warning_hardware_wallet(
                    &kind_label,
                    version.as_ref(),
                    fingerprint,
                    alias.as_ref(),
                    "The wallet descriptor is not registered on the device.\n You can register it in the settings.",
                )
            } else if !can_sign {
                hw::disabled_hardware_wallet(&kind_label, version.as_ref(), fingerprint, "This signing device is not part of this spending path.")
            } else {
                hw::supported_hardware_wallet(&kind_label, version.as_ref(), fingerprint, alias.as_ref())
            }
        }
        HardwareWallet::Unsupported {
            version,
            reason,
            ..
        } => match reason {
            UnsupportedReason::NotPartOfWallet(fg) => {
                hw::unrelated_hardware_wallet(kind_label.clone(), version.as_ref(), fg)
            }
            UnsupportedReason::WrongNetwork => {
                hw::wrong_network_hardware_wallet(kind_label.clone(), version.as_ref())
            }
            UnsupportedReason::Version {
                minimal_supported_version,
            } => hw::unsupported_version_hardware_wallet(
                kind_label.clone(),
                version.as_ref(),
                minimal_supported_version,
            ),
            _ => hw::unsupported_hardware_wallet(kind_label.clone(), version.as_ref()),
        },
        HardwareWallet::Locked {
            pairing_code, ..
        } => hw::locked_hardware_wallet(kind_label.clone(), pairing_code.as_ref()),
    };
    let mut bttn = Button::new(wrap_coin_cube_icon(hw, content))
        .style(theme::button::secondary)
        .width(Length::Fill);
    if can_sign && !signing {
        if let HardwareWallet::Supported { registered, .. } = hw {
            if *registered != Some(false) {
                bttn = bttn.on_press(Message::SelectHardwareWallet(i));
            }
        }
    }
    bttn.into()
}

pub fn hw_list_view_for_registration(
    i: usize,
    hw: &HardwareWallet,
    chosen: bool,
    processing: bool,
    registered: bool,
) -> Element<'_, Message> {
    let kind_label = hw.display_name();
    let content = match hw {
        HardwareWallet::Supported {
            version,
            fingerprint,
            alias,
            ..
        } => {
            if chosen && processing {
                hw::processing_hardware_wallet(&kind_label, version.as_ref(), fingerprint, alias.as_ref())
            } else if registered {
                hw::registration_success_hardware_wallet(
                    &kind_label,
                    version.as_ref(),
                    fingerprint,
                    alias.as_ref(),
                )
            } else {
                hw::supported_hardware_wallet(&kind_label, version.as_ref(), fingerprint, alias.as_ref())
            }
        }
        HardwareWallet::Unsupported {
            version,
            reason,
            ..
        } => match reason {
            UnsupportedReason::NotPartOfWallet(fg) => {
                hw::unrelated_hardware_wallet(kind_label.clone(), version.as_ref(), fg)
            }
            UnsupportedReason::WrongNetwork => {
                hw::wrong_network_hardware_wallet(kind_label.clone(), version.as_ref())
            }
            UnsupportedReason::Version {
                minimal_supported_version,
            } => hw::unsupported_version_hardware_wallet(
                kind_label.clone(),
                version.as_ref(),
                minimal_supported_version,
            ),
            _ => hw::unsupported_hardware_wallet(kind_label.clone(), version.as_ref()),
        },
        HardwareWallet::Locked {
            pairing_code, ..
        } => hw::locked_hardware_wallet(kind_label.clone(), pairing_code.as_ref()),
    };
    let mut bttn = Button::new(wrap_coin_cube_icon(hw, content))
        .style(theme::button::secondary)
        .width(Length::Fill);
    if !processing && hw.is_supported() {
        bttn = bttn.on_press(Message::SelectHardwareWallet(i));
    }
    bttn.into()
}

pub fn hw_list_view_verify_address(
    i: usize,
    hw: &HardwareWallet,
    chosen: bool,
) -> Element<'_, Message> {
    let kind_label = hw.display_name();
    let (content, selectable) = match hw {
        HardwareWallet::Supported {
            kind,
            version,
            fingerprint,
            alias,
            ..
        } => {
            if chosen {
                (
                    hw::processing_hardware_wallet(
                        &kind_label,
                        version.as_ref(),
                        fingerprint,
                        alias.as_ref(),
                    ),
                    false,
                )
            } else {
                match kind {
                    DeviceKind::Specter | DeviceKind::SpecterSimulator => {
                        if is_coin_cube(hw) {
                            (hw::supported_hardware_wallet(
                                &kind_label,
                                version.as_ref(),
                                fingerprint,
                                alias.as_ref(),
                            ), true)
                        } else {
                            (hw::unimplemented_method_hardware_wallet(
                                kind_label.clone(),
                                version.as_ref(),
                                fingerprint,
                                "Coincube cannot request the device to display the address. \n The verification must be done manually with the device control."
                            ), false)
                        }
                    }
                    _ => (hw::supported_hardware_wallet(
                        &kind_label,
                        version.as_ref(),
                        fingerprint,
                        alias.as_ref(),
                    ), true),
                }
            }
        }
        HardwareWallet::Unsupported {
            version,
            reason,
            ..
        } => (
            match reason {
                UnsupportedReason::NotPartOfWallet(fg) => {
                    hw::unrelated_hardware_wallet(kind_label.clone(), version.as_ref(), fg)
                }
                UnsupportedReason::WrongNetwork => {
                    hw::wrong_network_hardware_wallet(kind_label.clone(), version.as_ref())
                }
                UnsupportedReason::Version {
                    minimal_supported_version,
                } => hw::unsupported_version_hardware_wallet(
                    kind_label.clone(),
                    version.as_ref(),
                    minimal_supported_version,
                ),
                _ => hw::unsupported_hardware_wallet(kind_label.clone(), version.as_ref()),
            },
            false,
        ),
        HardwareWallet::Locked {
            pairing_code, ..
        } => (
            hw::locked_hardware_wallet(kind_label.clone(), pairing_code.as_ref()),
            false,
        ),
    };
    let mut bttn = Button::new(wrap_coin_cube_icon(hw, content))
        .style(theme::button::secondary)
        .width(Length::Fill);
    if selectable && hw.is_supported() {
        bttn = bttn.on_press(Message::SelectHardwareWallet(i));
    }
    bttn.into()
}
