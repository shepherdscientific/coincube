pub mod move_funds;
pub mod send;

use std::convert::TryInto;
use std::sync::Arc;

use coincube_core::{
    miniscript::bitcoin::Amount,
    spark_wallet::{SparkTransaction, SparkWallet, SparkWalletInfo, SparkBalance},
};
use coincube_ui::widget::Element;
use iced::{Subscription, Task};

use crate::{
    app::{
        cache::Cache,
        menu::{Menu, SparkSubMenu},
        settings,
        state::State,
        view::{self, SparkOverviewMessage},
        wallet::Wallet,
        Message,
    },
    daemon::Daemon,
    dir::CoincubeDirectory,
};

pub struct SparkOverview {
    datadir: CoincubeDirectory,
    network: coincube_core::miniscript::bitcoin::Network,
    cube_id: String,
    wallet_id: Option<String>,
    btc_balance: Amount,
    btkn_balance: Amount,
    transactions: Vec<SparkTransaction>,
    error: Option<String>,
}

impl SparkOverview {
    pub fn new(
        datadir: CoincubeDirectory,
        network: coincube_core::miniscript::bitcoin::Network,
        cube_id: String,
    ) -> Self {
        Self {
            datadir,
            network,
            cube_id,
            wallet_id: None,
            btc_balance: Amount::ZERO,
            btkn_balance: Amount::ZERO,
            transactions: Vec::new(),
            error: None,
        }
    }

    fn wallet_path(&self) -> std::path::PathBuf {
        settings::spark_wallet_state_path(
            &self.datadir.network_directory(self.network),
            &self.cube_id,
        )
    }

    fn load_wallet_snapshot(
        path: std::path::PathBuf,
    ) -> Result<(SparkWalletInfo, SparkBalance, Vec<SparkTransaction>), String> {
        let wallet = SparkWallet::load(path).map_err(|e| e.to_string())?;
        let info = wallet.wallet_info().map_err(|e| e.to_string())?;
        let balance = wallet.get_balance().map_err(|e| e.to_string())?;
        let transactions = wallet.get_transactions().map_err(|e| e.to_string())?;
        Ok((info, balance, transactions))
    }
}

impl State for SparkOverview {
    fn view<'a>(&'a self, menu: &'a Menu, cache: &'a Cache) -> Element<'a, view::Message> {
        let fiat_converter: Option<crate::app::view::vault::fiat::FiatAmountConverter> =
            cache.fiat_price.as_ref().and_then(|p| p.try_into().ok());
        let content = view::spark::spark_overview_view(
            self.wallet_id.as_deref(),
            self.btc_balance,
            self.btkn_balance,
            &self.transactions,
            self.error.as_deref(),
            cache.bitcoin_unit,
            fiat_converter,
        )
        .map(view::Message::SparkOverview);

        view::dashboard(menu, cache, content)
    }

    fn update(
        &mut self,
        _daemon: Option<Arc<dyn Daemon + Sync + Send>>,
        _cache: &Cache,
        message: Message,
    ) -> Task<Message> {
        if let Message::View(view::Message::SparkOverview(msg)) = message {
            match msg {
                SparkOverviewMessage::RefreshRequested => {
                    let path = self.wallet_path();
                    return Task::perform(async move { Self::load_wallet_snapshot(path) }, |res| {
                        match res {
                            Ok((info, balance, transactions)) => Message::View(
                                view::Message::SparkOverview(SparkOverviewMessage::Loaded {
                                    wallet_id: info.wallet_id,
                                    balance,
                                    transactions,
                                }),
                            ),
                            Err(e) => Message::View(view::Message::SparkOverview(
                                SparkOverviewMessage::LoadFailed(e),
                            )),
                        }
                    });
                }
                SparkOverviewMessage::Loaded {
                    wallet_id,
                    balance,
                    transactions,
                } => {
                    self.wallet_id = Some(wallet_id);
                    self.btc_balance = Amount::from_sat(balance.bitcoin_sats);
                    self.btkn_balance = Amount::from_sat(balance.btkn_sats);
                    self.transactions = transactions;
                    self.error = None;
                }
                SparkOverviewMessage::LoadFailed(e) => {
                    self.error = Some(e);
                }
                SparkOverviewMessage::Send => {
                    return Task::done(Message::View(view::Message::Menu(Menu::Spark(
                        SparkSubMenu::Send,
                    ))));
                }
                SparkOverviewMessage::Receive => {
                    return Task::done(Message::View(view::Message::Menu(Menu::Spark(
                        SparkSubMenu::Receive,
                    ))));
                }
                SparkOverviewMessage::MoveFunds => {
                    return Task::done(Message::View(view::Message::Menu(Menu::Spark(
                        SparkSubMenu::MoveFunds,
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
        Task::done(Message::View(view::Message::SparkOverview(
            SparkOverviewMessage::RefreshRequested,
        )))
    }

    fn subscription(&self) -> Subscription<Message> {
        // Auto-refresh every 30 seconds
        iced::time::every(std::time::Duration::from_secs(30)).map(|_| {
            Message::View(view::Message::SparkOverview(
                SparkOverviewMessage::RefreshRequested,
            ))
        })
    }
}
