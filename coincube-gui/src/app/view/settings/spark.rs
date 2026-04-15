use iced::widget::{Column, Container, Row};
use iced::{Alignment, Length};

use coincube_ui::component::{card, text::*};
use coincube_ui::theme;
use coincube_ui::widget::{Element, TextInput};

use crate::app::cache::Cache;
use crate::app::menu::Menu;
use crate::app::state::settings::spark::SparkSettingsState;
use crate::app::view::{dashboard, message::*};

pub fn spark_section<'a>(
    menu: &'a Menu,
    cache: &'a Cache,
    state: &'a SparkSettingsState,
) -> Element<'a, Message> {
    let ssp_url_value = state.ssp_url.as_deref().unwrap_or("");
    dashboard(
        menu,
        cache,
        Column::new()
            .spacing(20)
            .push(super::header("Spark", SettingsMessage::SparkSection))
            .push(
                Container::new(
                    Column::new()
                        .spacing(20)
                        .push(text("Spark Service Provider (SSP) Configuration").bold())
                        .push(
                            text("Configure your Spark Service Provider URL. The default Lightspark SSP will be used if not specified.")
                                .small(),
                        )
                        .push(
                            card::simple(
                                Column::new()
                                    .spacing(10)
                                    .push(text("Default SSP URL: https://api.lightspark.com").small())
                                    .push(
                                        Column::new()
                                            .spacing(5)
                                            .push(text("Custom SSP URL:").bold())
                                            .push(
                                                Row::new()
                                                    .spacing(10)
                                                    .align_y(Alignment::Center)
                                                    .push(text("SSP URL:").width(Length::Fill))
                                                    .push(
                                                        TextInput::new(
                                                            "Enter Spark SSP URL (e.g., https://api.lightspark.com)",
                                                            ssp_url_value,
                                                        )
                                                        .on_input(move |input: String| {
                                                            SettingsMessage::SparkSspUrlEdited(if input.is_empty() {
                                                                None
                                                            } else {
                                                                Some(input)
                                                            })
                                                            .into()
                                                        })
                                                        .width(Length::Fill),
                                                    ),
                                            ),
                                    )
                            ).width(Length::Fill)
                        ),
                )
                .width(Length::Fill)
                .style(theme::card::simple),
            ),
    )
}
