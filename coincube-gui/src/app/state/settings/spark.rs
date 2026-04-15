use std::sync::Arc;

use coincube_ui::widget::Element;
use iced::Task;

use crate::{
    app::{
        cache::Cache,
        menu::Menu,
        state::State,
        view::{self},
        wallet::Wallet,
        Message,
    },
    daemon::Daemon,
};

pub struct SparkSettingsState {
    pub ssp_url: Option<String>,
}

impl Default for SparkSettingsState {
    fn default() -> Self {
        Self::new()
    }
}

impl SparkSettingsState {
    pub fn new() -> Self {
        SparkSettingsState { ssp_url: None }
    }
}

impl State for SparkSettingsState {
    fn view<'a>(&'a self, menu: &'a Menu, cache: &'a Cache) -> Element<'a, view::Message> {
        crate::app::view::settings::spark::spark_section(menu, cache, self)
    }

    fn reload(
        &mut self,
        _daemon: Option<Arc<dyn Daemon + Sync + Send>>,
        _wallet: Option<Arc<Wallet>>,
    ) -> Task<Message> {
        Task::none()
    }

    fn update(
        &mut self,
        _daemon: Option<Arc<dyn Daemon + Sync + Send>>,
        _cache: &Cache,
        message: Message,
    ) -> Task<Message> {
        match message {
            Message::View(view::Message::Settings(view::SettingsMessage::SparkSspUrlEdited(
                url,
            ))) => {
                self.ssp_url = url;
                Task::none()
            }
            Message::View(view::Message::Settings(view::SettingsMessage::GeneralSection)) => {
                self.reload(None, None)
            }
            _ => Task::none(),
        }
    }
}

impl From<SparkSettingsState> for Box<dyn State> {
    fn from(s: SparkSettingsState) -> Box<dyn State> {
        Box::new(s)
    }
}
