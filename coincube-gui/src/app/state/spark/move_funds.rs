use std::convert::TryInto;
use std::sync::Arc;

use crate::app::settings::unit::BitcoinDisplayUnit;
use coincube_ui::component::form;
use coincube_ui::widget::Element;
use iced::Task;

use crate::app::{
    cache::Cache,
    menu::Menu,
    settings,
    state::State,
    view::{self, MoveFundsDirection, SparkMoveFundsMessage},
    wallet::Wallet,
    Message,
};
use crate::daemon::Daemon;
use crate::dir::CoincubeDirectory;

pub struct SparkMoveFunds {
    datadir: CoincubeDirectory,
    network: coincube_core::miniscript::bitcoin::Network,
    cube_id: String,
    direction: MoveFundsDirection,
    amount_input: form::Value<String>,
    fee_estimate: Option<u64>,
    time_estimate: Option<u64>,
    loading: bool,
    error: Option<String>,
}

impl SparkMoveFunds {
    pub fn new(
        datadir: CoincubeDirectory,
        network: coincube_core::miniscript::bitcoin::Network,
        cube_id: String,
    ) -> Self {
        Self {
            datadir,
            network,
            cube_id,
            direction: MoveFundsDirection::VaultToSpark,
            amount_input: form::Value {
                value: String::new(),
                warning: None,
                valid: true,
            },
            fee_estimate: None,
            time_estimate: None,
            loading: false,
            error: None,
        }
    }

    fn wallet_path(&self) -> std::path::PathBuf {
        settings::spark_wallet_state_path(
            &self.datadir.network_directory(self.network),
            &self.cube_id,
        )
    }

    fn parse_amount(&self) -> Result<u64, String> {
        let amount_str = self.amount_input.value.trim();
        if amount_str.is_empty() {
            return Err("Amount is required".to_string());
        }

        amount_str
            .parse::<u64>()
            .map_err(|_| "Invalid amount. Enter a number in satoshis".to_string())
    }

    fn estimate_fee(&self, amount_sat: u64) -> u64 {
        // Mock fee estimation: 1 sat/byte * typical tx size
        // For a typical 1-input 2-output transaction
        let tx_size_bytes = 200;
        let fee_rate_sat_per_byte = 1;
        tx_size_bytes * fee_rate_sat_per_byte
    }

    fn estimate_time(&self, _amount_sat: u64) -> u64 {
        // Mock time estimation for cooperative exit: 10 minutes
        10 * 60 // 10 minutes in seconds
    }
}

impl State for SparkMoveFunds {
    fn view<'a>(&'a self, menu: &'a Menu, cache: &'a Cache) -> Element<'a, view::Message> {
        let fiat_converter: Option<crate::app::view::vault::fiat::FiatAmountConverter> =
            cache.fiat_price.as_ref().and_then(|p| p.try_into().ok());

        // Show fee estimate for Vault→Spark, time estimate for Spark→Vault
        let fee_estimate = match self.direction {
            MoveFundsDirection::VaultToSpark => self.fee_estimate,
            MoveFundsDirection::SparkToVault => None,
        };
        let time_estimate = match self.direction {
            MoveFundsDirection::VaultToSpark => None,
            MoveFundsDirection::SparkToVault => self.time_estimate,
        };

        let content = view::spark::spark_move_funds_view(
            self.direction,
            &self.amount_input,
            fee_estimate,
            time_estimate,
            self.loading,
            self.error.as_deref(),
            cache.bitcoin_unit,
            fiat_converter,
        )
        .map(view::Message::SparkMoveFunds);

        view::dashboard(menu, cache, content)
    }

    fn update(
        &mut self,
        daemon: Option<Arc<dyn Daemon + Sync + Send>>,
        cache: &Cache,
        message: Message,
    ) -> Task<Message> {
        if let Message::View(view::Message::SparkMoveFunds(msg)) = message {
            match msg {
                SparkMoveFundsMessage::ToggleDirection(direction) => {
                    self.direction = direction;
                    // Clear estimates when direction changes
                    self.fee_estimate = None;
                    self.time_estimate = None;
                }
                SparkMoveFundsMessage::AmountInput(amount) => {
                    self.amount_input.value = amount;
                    self.amount_input.valid = true;
                    self.amount_input.warning = None;
                    // Clear estimates when amount changes
                    self.fee_estimate = None;
                    self.time_estimate = None;
                }
                SparkMoveFundsMessage::EstimateRequested => match self.parse_amount() {
                    Ok(amount_sat) => match self.direction {
                        MoveFundsDirection::VaultToSpark => {
                            let fee = self.estimate_fee(amount_sat);
                            return Task::done(Message::View(view::Message::SparkMoveFunds(
                                SparkMoveFundsMessage::FeeEstimated(fee),
                            )));
                        }
                        MoveFundsDirection::SparkToVault => {
                            let time = self.estimate_time(amount_sat);
                            return Task::done(Message::View(view::Message::SparkMoveFunds(
                                SparkMoveFundsMessage::TimeEstimated(time),
                            )));
                        }
                    },
                    Err(e) => {
                        self.error = Some(e);
                    }
                },
                SparkMoveFundsMessage::FeeEstimated(fee) => {
                    self.fee_estimate = Some(fee);
                }
                SparkMoveFundsMessage::TimeEstimated(time) => {
                    self.time_estimate = Some(time);
                }
                SparkMoveFundsMessage::MoveRequested => {
                    match self.parse_amount() {
                        Ok(amount_sat) => {
                            self.loading = true;
                            self.error = None;

                            let direction = self.direction;
                            let datadir = self.datadir.clone();
                            let network = self.network;
                            let cube_id = self.cube_id.clone();
                            let amount = amount_sat;

                            return Task::perform(
                                async move {
                                    // In a real implementation, this would call the daemon RPC
                                    // For now, simulate a successful move
                                    std::thread::sleep(std::time::Duration::from_secs(2));
                                    Ok(format!("move-{:x}", rand::random::<u64>()))
                                },
                                move |result| match result {
                                    Ok(txid) => Message::View(view::Message::SparkMoveFunds(
                                        SparkMoveFundsMessage::MoveCompleted(txid),
                                    )),
                                    Err(e) => Message::View(view::Message::SparkMoveFunds(
                                        SparkMoveFundsMessage::Error(e),
                                    )),
                                },
                            );
                        }
                        Err(e) => {
                            self.error = Some(e);
                        }
                    }
                }
                SparkMoveFundsMessage::MoveCompleted(txid) => {
                    self.loading = false;
                    // Show success message and reset form
                    self.error = Some(format!(
                        "Successfully moved funds! Transaction ID: {}",
                        txid
                    ));
                    self.amount_input.value = String::new();
                    self.fee_estimate = None;
                    self.time_estimate = None;
                }
                SparkMoveFundsMessage::Error(e) => {
                    self.loading = false;
                    self.error = Some(e);
                }
                SparkMoveFundsMessage::ClearError => {
                    self.error = None;
                }
                SparkMoveFundsMessage::Close => {
                    return Task::done(Message::View(view::Message::Menu(Menu::Spark(
                        crate::app::menu::SparkSubMenu::Overview,
                    ))));
                }
            }
        }
        Task::none()
    }

    fn reload(
        &mut self,
        _daemon: Option<Arc<dyn Daemon + Sync + Send>>,
        _wallet: Option<Arc<Wallet>>,
    ) -> Task<Message> {
        Task::none()
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        iced::Subscription::none()
    }
}
