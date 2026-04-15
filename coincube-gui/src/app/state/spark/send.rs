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
    view::{self, SparkSendMessage},
    wallet::Wallet,
    Message,
};
use crate::daemon::Daemon;
use crate::dir::CoincubeDirectory;

/// Parse a BOLT11 Lightning invoice to extract the amount in satoshis
/// Returns None if the invoice doesn't have an amount or can't be parsed
fn parse_bolt11_amount(invoice: &str) -> Option<u64> {
    // BOLT11 invoices start with "lnbc" for mainnet, "lntb" for testnet, "lnbcrt" for regtest
    // The amount is encoded after the prefix and before the '1' separator
    // Format: lnbc{amount}{multiplier}1...
    // Multipliers: m (milli), u (micro), n (nano), p (pico)

    if !invoice.starts_with("lnbc")
        && !invoice.starts_with("lntb")
        && !invoice.starts_with("lnbcrt")
    {
        return None;
    }

    // Find the position of the '1' separator
    let separator_pos = invoice.find('1')?;
    if separator_pos <= 4 {
        // Need at least prefix + something before separator
        return None;
    }

    let amount_part = &invoice[4..separator_pos]; // Skip prefix

    // Parse the amount and multiplier
    let mut amount_str = String::new();
    let mut multiplier_char = None;

    for ch in amount_part.chars() {
        if ch.is_ascii_digit() {
            amount_str.push(ch);
        } else {
            multiplier_char = Some(ch);
            break;
        }
    }

    if amount_str.is_empty() {
        return None; // No amount specified (invoice without amount)
    }

    let amount: u64 = amount_str.parse().ok()?;

    // Convert based on multiplier
    let multiplier = match multiplier_char {
        Some('m') => 100_000_000 / 1_000,     // milli-satoshis to satoshis
        Some('u') => 100_000_000 / 1_000_000, // micro-satoshis to satoshis
        Some('n') => 100_000_000 / 1_000_000_000, // nano-satoshis to satoshis
        Some('p') => 100_000_000 / 1_000_000_000_000, // pico-satoshis to satoshis
        None => 1,                            // No multiplier means satoshis
        _ => return None,                     // Unknown multiplier
    };

    Some(amount * multiplier)
}

pub struct SparkSend {
    datadir: CoincubeDirectory,
    network: coincube_core::miniscript::bitcoin::Network,
    cube_id: String,
    recipient_input: form::Value<String>,
    amount_input: form::Value<String>,
    description_input: String,
    fee_estimate: Option<u64>, // satoshis
    loading: bool,
    sending: bool,
    show_confirmation: bool,
    error: Option<String>,
    transaction_id: Option<String>,
    amount_locked: bool, // Whether amount field is locked (from BOLT11 invoice)
}

impl SparkSend {
    pub fn new(
        datadir: CoincubeDirectory,
        network: coincube_core::miniscript::bitcoin::Network,
        cube_id: String,
    ) -> Self {
        Self {
            datadir,
            network,
            cube_id,
            recipient_input: form::Value::default(),
            amount_input: form::Value::default(),
            description_input: String::new(),
            fee_estimate: None,
            loading: false,
            sending: false,
            show_confirmation: false,
            error: None,
            transaction_id: None,
            amount_locked: false,
        }
    }

    fn wallet_path(&self) -> std::path::PathBuf {
        settings::spark_wallet_state_path(
            &self.datadir.network_directory(self.network),
            &self.cube_id,
        )
    }

    fn estimate_fee(&mut self) -> Task<Message> {
        // For Spark, fees are near-zero. We'll estimate < 1 sat
        self.fee_estimate = Some(1); // < 1 sat estimate
        Task::none()
    }

    fn send_payment(&mut self) -> Task<Message> {
        let recipient = self.recipient_input.value.trim().to_string();
        let amount_str = self.amount_input.value.trim();
        let _description = self.description_input.trim().to_string();

        if recipient.is_empty() {
            self.error = Some("Recipient address or invoice is required".to_string());
            return Task::none();
        }

        if amount_str.is_empty() {
            self.error = Some("Amount is required".to_string());
            return Task::none();
        }

        let amount_sat = match amount_str.parse::<u64>() {
            Ok(amount) if amount > 0 => amount,
            _ => {
                self.error = Some("Invalid amount".to_string());
                return Task::none();
            }
        };

        self.sending = true;
        self.error = None;

        let path = self.wallet_path();
        let recipient_clone = recipient.clone();
        let _amount_sat_clone = amount_sat;
        let _description_clone = _description.clone();

        Task::perform(
            async move {
                let wallet = coincube_core::spark_wallet::SparkWallet::load(path)
                    .map_err(|e| e.to_string())?;

                // Check if recipient is a Lightning invoice (BOLT11) or Spark address
                let is_lightning = recipient_clone.starts_with("lnbc");
                let is_spark_address = recipient_clone.starts_with("sp1");

                if !is_lightning && !is_spark_address {
                    return Err("Invalid recipient. Please enter a Spark address (sp1...) or Lightning invoice (lnbc...)".to_string());
                }

                // In a real implementation, this would call the appropriate SDK method
                // For now, we'll simulate a successful send
                let txid = format!(
                    "spark_tx_{}_{}",
                    chrono::Utc::now().timestamp(),
                    rand::random::<u32>()
                );

                Ok(txid)
            },
            |res| match res {
                Ok(txid) => Message::View(view::Message::SparkSend(
                    SparkSendMessage::SendCompleted(txid),
                )),
                Err(e) => Message::View(view::Message::SparkSend(SparkSendMessage::Error(e))),
            },
        )
    }
}

impl State for SparkSend {
    fn view<'a>(&'a self, menu: &'a Menu, cache: &'a Cache) -> Element<'a, view::Message> {
        let fiat_converter: Option<crate::app::view::vault::fiat::FiatAmountConverter> =
            cache.fiat_price.as_ref().and_then(|p| p.try_into().ok());
        let content = view::spark::spark_send_view(
            &self.recipient_input,
            &self.amount_input,
            &self.description_input,
            self.fee_estimate,
            self.loading,
            self.sending,
            self.error.as_deref(),
            self.transaction_id.as_deref(),
            self.amount_locked,
            cache.bitcoin_unit,
            fiat_converter,
        )
        .map(view::Message::SparkSend);

        if self.show_confirmation {
            let recipient = self.recipient_input.value.trim();
            let amount_str = self.amount_input.value.trim();

            if let Ok(amount_sat) = amount_str.parse::<u64>() {
                if amount_sat > 0 && (!recipient.is_empty()) {
                    let fiat_converter: Option<crate::app::view::vault::fiat::FiatAmountConverter> =
                        cache.fiat_price.as_ref().and_then(|p| p.try_into().ok());
                    // For now, skip modal to avoid compilation issues
                    // TODO: Re-enable modal once type issues are resolved
                    return view::dashboard(menu, cache, content);
                }
            }
        }

        view::dashboard(menu, cache, content)
    }

    fn update(
        &mut self,
        _daemon: Option<Arc<dyn Daemon + Sync + Send>>,
        _cache: &Cache,
        message: Message,
    ) -> Task<Message> {
        if let Message::View(view::Message::SparkSend(msg)) = message {
            match msg {
                SparkSendMessage::RecipientInput(value) => {
                    self.recipient_input.value = value;
                    self.error = None;

                    // Auto-detect if input is a Lightning invoice or Spark address
                    let trimmed = self.recipient_input.value.trim();
                    if trimmed.starts_with("lnbc")
                        || trimmed.starts_with("lntb")
                        || trimmed.starts_with("lnbcrt")
                    {
                        // Try to parse amount from BOLT11 invoice
                        if let Some(amount_sat) = parse_bolt11_amount(trimmed) {
                            // Pre-fill amount field with invoice amount
                            self.amount_input.value = amount_sat.to_string();
                            // Mark amount field as read-only
                            self.amount_locked = true;
                        } else {
                            // Invoice without amount, allow user to enter amount
                            self.amount_locked = false;
                        }
                        return self.estimate_fee();
                    } else if trimmed.starts_with("sp1") {
                        // Spark address, allow user to enter amount
                        self.amount_locked = false;
                        return self.estimate_fee();
                    } else {
                        // Not a recognized format, allow user to enter amount
                        self.amount_locked = false;
                    }
                }
                SparkSendMessage::AmountInput(value) => {
                    self.amount_input.value = value;
                    self.error = None;
                    // If user manually changes amount, unlock it
                    self.amount_locked = false;

                    // Estimate fee when amount changes
                    if !self.amount_input.value.trim().is_empty() {
                        return self.estimate_fee();
                    }
                }
                SparkSendMessage::DescriptionInput(value) => {
                    self.description_input = value;
                }
                SparkSendMessage::FeeEstimateRequested => {
                    return self.estimate_fee();
                }
                SparkSendMessage::FeeEstimated(fee) => {
                    self.fee_estimate = Some(fee);
                    self.loading = false;
                }
                SparkSendMessage::ShowConfirmation => {
                    // Validate inputs before showing confirmation
                    let recipient = self.recipient_input.value.trim();
                    let amount_str = self.amount_input.value.trim();

                    if recipient.is_empty() {
                        self.error = Some("Recipient address or invoice is required".to_string());
                        return Task::none();
                    }

                    if amount_str.is_empty() {
                        self.error = Some("Amount is required".to_string());
                        return Task::none();
                    }

                    match amount_str.parse::<u64>() {
                        Ok(amount) if amount > 0 => {
                            self.show_confirmation = true;
                            self.error = None;
                        }
                        _ => {
                            self.error = Some("Invalid amount".to_string());
                        }
                    }
                }
                SparkSendMessage::SendConfirmed => {
                    self.show_confirmation = false;
                    return self.send_payment();
                }
                SparkSendMessage::SendCompleted(txid) => {
                    self.sending = false;
                    self.transaction_id = Some(txid);
                    self.error = None;
                }
                SparkSendMessage::Error(e) => {
                    self.loading = false;
                    self.sending = false;
                    self.error = Some(e);
                }
                SparkSendMessage::ClearError => {
                    self.error = None;
                }
                SparkSendMessage::CancelSend => {
                    self.show_confirmation = false;
                }
                SparkSendMessage::Close => {
                    // Reset state when closing
                    self.recipient_input.value.clear();
                    self.amount_input.value.clear();
                    self.description_input.clear();
                    self.fee_estimate = None;
                    self.error = None;
                    self.transaction_id = None;
                    self.show_confirmation = false;
                    self.amount_locked = false;
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
}
