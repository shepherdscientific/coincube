use std::sync::Arc;

use coincube_core::{
    miniscript::bitcoin::Amount,
    spark_wallet::{SparkTransaction, SparkWallet, SparkWalletInfo},
};
use coincube_ui::widget::Element;
use iced::{Subscription, Task};

use crate::{
    app::{
        cache::Cache,
        menu::Menu,
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
    balance: Amount,
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
            balance: Amount::ZERO,
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

    fn load_wallet_snapshot(path: std::path::PathBuf) -> Result<(SparkWalletInfo, Amount, Vec<SparkTransaction>), String> {
        let wallet = SparkWallet::load(path).map_err(|e| e.to_string())?;
        let info = wallet.wallet_info().map_err(|e| e.to_string())?;
        let balance = wallet.get_balance().map_err(|e| e.to_string())?;
        let transactions = wallet.get_transactions().map_err(|e| e.to_string())?;
        Ok((info, Amount::from_sat(balance.sats), transactions))
    }
}

impl State for SparkOverview {
    fn view<'a>(&'a self, menu: &'a Menu, cache: &'a Cache) -> Element<'a, view::Message> {
        view::dashboard(
            menu,
            cache,
            view::spark::spark_overview_view(
                self.wallet_id.as_deref(),
                self.balance,
                &self.transactions,
                self.error.as_deref(),
                cache.bitcoin_unit,
                cache.fiat_price.as_ref().map(|price| price.converter()),
            )
            .map(view::Message::SparkOverview),
        )
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
                            Ok((info, balance, transactions)) => Message::View(view::Message::SparkOverview(
                                SparkOverviewMessage::Loaded {
                                    wallet_id: info.wallet_id,
                                    balance,
                                    transactions,
                                },
                            )),
                            Err(e) => Message::View(view::Message::SparkOverview(
                                SparkOverviewMessage::LoadFailed(e),
                            )),
                        }
                    });
                }
                SparkOverviewMessage::Loaded { wallet_id, balance, transactions } => {
                    self.wallet_id = Some(wallet_id);
                    self.balance = balance;
                    self.transactions = transactions;
                    self.error = None;
                }
                SparkOverviewMessage::LoadFailed(e) => {
                    self.error = Some(e);
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
