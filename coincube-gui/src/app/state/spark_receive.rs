use std::convert::TryInto;
use std::sync::Arc;

use crate::app::settings::unit::BitcoinDisplayUnit;
use coincube_ui::component::form;
use coincube_ui::widget::Element;
use iced::{clipboard, widget::qr_code, Task};

use crate::app::{
    cache::Cache,
    menu::Menu,
    settings,
    state::State,
    view::{self, SparkReceiveMessage},
    wallet::Wallet,
    Message,
};
use crate::daemon::Daemon;
use crate::dir::CoincubeDirectory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SparkReceiveMethod {
    SparkAddress,
    LightningInvoice,
}

impl SparkReceiveMethod {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::SparkAddress => "Spark Address",
            Self::LightningInvoice => "Lightning Invoice",
        }
    }
}

pub struct SparkReceive {
    datadir: CoincubeDirectory,
    network: coincube_core::miniscript::bitcoin::Network,
    cube_id: String,
    receive_method: SparkReceiveMethod,
    spark_address: Option<String>,
    spark_qr_data: Option<qr_code::Data>,
    lightning_invoice: Option<String>,
    lightning_qr_data: Option<qr_code::Data>,
    amount_input: form::Value<String>,
    description_input: String,
    loading: bool,
    error: Option<String>,
    show_qr_modal: bool,
    invoice_expiry_seconds: Option<u64>,
}

impl SparkReceive {
    pub fn new(
        datadir: CoincubeDirectory,
        network: coincube_core::miniscript::bitcoin::Network,
        cube_id: String,
    ) -> Self {
        Self {
            datadir,
            network,
            cube_id,
            receive_method: SparkReceiveMethod::SparkAddress,
            spark_address: None,
            spark_qr_data: None,
            lightning_invoice: None,
            lightning_qr_data: None,
            amount_input: form::Value::default(),
            description_input: String::new(),
            loading: false,
            error: None,
            show_qr_modal: false,
            invoice_expiry_seconds: None,
        }
    }

    fn wallet_path(&self) -> std::path::PathBuf {
        settings::spark_wallet_state_path(
            &self.datadir.network_directory(self.network),
            &self.cube_id,
        )
    }

    fn current_address(&self) -> Option<&str> {
        match self.receive_method {
            SparkReceiveMethod::SparkAddress => self.spark_address.as_deref(),
            SparkReceiveMethod::LightningInvoice => self.lightning_invoice.as_deref(),
        }
    }

    fn current_qr_data(&self) -> Option<&qr_code::Data> {
        match self.receive_method {
            SparkReceiveMethod::SparkAddress => self.spark_qr_data.as_ref(),
            SparkReceiveMethod::LightningInvoice => self.lightning_qr_data.as_ref(),
        }
    }

    fn generate_spark_address(&mut self) -> Task<Message> {
        let path = self.wallet_path();
        Task::perform(
            async move {
                let mut wallet = coincube_core::spark_wallet::SparkWallet::load(path)
                    .map_err(|e| e.to_string())?;
                wallet.receive_address().map_err(|e| e.to_string())
            },
            |res| match res {
                Ok(address) => Message::View(view::Message::SparkReceive(
                    SparkReceiveMessage::SparkAddressGenerated(address),
                )),
                Err(e) => Message::View(view::Message::SparkReceive(SparkReceiveMessage::Error(e))),
            },
        )
    }

    fn generate_lightning_invoice(&mut self) -> Task<Message> {
        let amount_str = self.amount_input.value.trim();
        let _description = self.description_input.trim();

        // For now, we'll generate a mock invoice since we don't have actual Lightning integration
        // In a real implementation, this would call the Spark/Lightning SDK
        let amount_sat = if amount_str.is_empty() {
            None
        } else {
            amount_str.parse::<u64>().ok()
        };

        let invoice = if let Some(amount) = amount_sat {
            format!("lnbc{}p1pj9m9z2pp5nqyqkqcqz9q8q...", amount)
        } else {
            "lnbc1pj9m9z2pp5nqyqkqcqz9q8q...".to_string()
        };

        self.invoice_expiry_seconds = Some(3600); // 1 hour expiry

        Task::done(Message::View(view::Message::SparkReceive(
            SparkReceiveMessage::LightningInvoiceGenerated(invoice),
        )))
    }
}

impl State for SparkReceive {
    fn view<'a>(&'a self, menu: &'a Menu, cache: &'a Cache) -> Element<'a, view::Message> {
        let fiat_converter: Option<crate::app::view::vault::fiat::FiatAmountConverter> =
            cache.fiat_price.as_ref().and_then(|p| p.try_into().ok());
        let content = view::spark::spark_receive_view(
            self.receive_method,
            self.current_address(),
            self.current_qr_data(),
            &self.amount_input,
            &self.description_input,
            self.loading,
            self.error.as_deref(),
            cache.bitcoin_unit,
            fiat_converter,
            self.invoice_expiry_seconds,
        )
        .map(view::Message::SparkReceive);

        if self.show_qr_modal {
            // For now, skip modal to avoid compilation issues
            // TODO: Re-enable modal once type issues are resolved
        }

        view::dashboard(menu, cache, content)
    }

    fn update(
        &mut self,
        _daemon: Option<Arc<dyn Daemon + Sync + Send>>,
        _cache: &Cache,
        message: Message,
    ) -> Task<Message> {
        if let Message::View(view::Message::SparkReceive(msg)) = message {
            match msg {
                SparkReceiveMessage::ToggleMethod(method) => {
                    if self.receive_method != method {
                        self.receive_method = method;
                        self.error = None;
                        self.show_qr_modal = false;

                        // Generate address/invoice when switching methods
                        match method {
                            SparkReceiveMethod::SparkAddress => {
                                return self.generate_spark_address();
                            }
                            SparkReceiveMethod::LightningInvoice => {
                                // Don't auto-generate invoice, wait for user input
                            }
                        }
                    }
                }
                SparkReceiveMessage::ShowQrCode => {
                    self.show_qr_modal = true;
                }
                SparkReceiveMessage::CloseQrCode => {
                    self.show_qr_modal = false;
                }
                SparkReceiveMessage::Copy => {
                    if let Some(address) = self.current_address() {
                        let message = match self.receive_method {
                            SparkReceiveMethod::SparkAddress => {
                                "Copied Spark Address to clipboard".to_string()
                            }
                            SparkReceiveMethod::LightningInvoice => {
                                "Copied Lightning Invoice to clipboard".to_string()
                            }
                        };
                        return Task::batch(vec![
                            clipboard::write(address.to_string()),
                            Task::done(Message::View(view::Message::ShowSuccess(message))),
                        ]);
                    }
                }
                SparkReceiveMessage::GenerateAddress => {
                    self.loading = true;
                    self.error = None;
                    return self.generate_spark_address();
                }
                SparkReceiveMessage::GenerateInvoice => {
                    self.loading = true;
                    self.error = None;
                    return self.generate_lightning_invoice();
                }
                SparkReceiveMessage::SparkAddressGenerated(address) => {
                    self.loading = false;
                    self.spark_address = Some(address.clone());
                    // Generate QR code
                    self.spark_qr_data = qr_code::Data::new(&address).ok();
                }
                SparkReceiveMessage::LightningInvoiceGenerated(invoice) => {
                    self.loading = false;
                    self.lightning_invoice = Some(invoice.clone());
                    // Generate QR code
                    self.lightning_qr_data = qr_code::Data::new(&invoice).ok();
                }
                SparkReceiveMessage::AmountInput(value) => {
                    self.amount_input.value = value;
                }
                SparkReceiveMessage::DescriptionInput(value) => {
                    self.description_input = value;
                }
                SparkReceiveMessage::Error(e) => {
                    self.loading = false;
                    self.error = Some(e);
                }
                SparkReceiveMessage::ClearError => {
                    self.error = None;
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
        // Generate initial address when state loads
        self.generate_spark_address()
    }
}
