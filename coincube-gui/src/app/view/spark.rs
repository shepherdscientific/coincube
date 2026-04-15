use coincube_core::spark_wallet::SparkTransactionDirection;
use coincube_core::{
    miniscript::bitcoin::Amount, spark_wallet::SparkAssetType, spark_wallet::SparkTransaction,
};
use coincube_ui::{
    color,
    component::{
        amount::*,
        badge, button, card, form,
        text::{self, h3_bold, h4_bold, p1_regular, p2_regular, H2_SIZE, H4_SIZE},
        transaction::{TransactionDirection, TransactionListItem},
    },
    icon::{arrow_down_up_icon, check_circle_icon, lightning_icon, receive_icon, send_icon},
    theme,
    widget::*,
};
use iced::widget::{qr_code, text as iced_text, Space};
use iced::{Alignment, Length};

use crate::utils::format_time_ago;

use crate::app::state::spark_receive::SparkReceiveMethod;
use crate::app::view::{
    FiatAmountConverter, MoveFundsDirection, SparkMoveFundsMessage, SparkOverviewMessage,
    SparkReceiveMessage, SparkSendMessage,
};

pub fn spark_overview_view<'a>(
    wallet_id: Option<&'a str>,
    btc_balance: Amount,
    btkn_balance: Amount,
    transactions: &'a [SparkTransaction],
    error: Option<&'a str>,
    bitcoin_unit: BitcoinDisplayUnit,
    fiat_converter: Option<FiatAmountConverter>,
) -> Element<'a, SparkOverviewMessage> {
    let mut content = Column::new()
        .spacing(20)
        .push(h3_bold("Spark Wallet"))
        .push(
            p1_regular("Your Spark spending wallet is ready for instant Bitcoin transfers.")
                .style(theme::text::secondary),
        )
        .push(
            Container::new(
                Column::new()
                    .spacing(15)
                    .push(
                        Row::new()
                            .spacing(10)
                            .align_y(Alignment::Center)
                            .push(lightning_icon().size(18).color(color::ORANGE))
                            .push(iced_text("BTC Balance").size(13).color(color::GREY_2)),
                    )
                    .push(amount_with_size_and_unit(
                        &btc_balance,
                        H2_SIZE,
                        bitcoin_unit,
                    ))
                    .push_maybe(fiat_converter.clone().map(|converter| {
                        let fiat_amount = converter.convert(btc_balance);
                        text::text(format!(
                            "~{} {}",
                            fiat_amount.to_rounded_string(),
                            fiat_amount.currency()
                        ))
                        .size(14)
                        .color(color::GREY_3)
                    }))
                    .push(
                        Row::new()
                            .spacing(10)
                            .align_y(Alignment::Center)
                            .push(iced_text("BTKN Balance").size(13).color(color::GREY_2)),
                    )
                    .push(amount_with_size_and_unit(
                        &btkn_balance,
                        H2_SIZE,
                        bitcoin_unit,
                    ))
                    .push_maybe(fiat_converter.map(|converter| {
                        let fiat_amount = converter.convert(btkn_balance);
                        text::text(format!(
                            "~{} {}",
                            fiat_amount.to_rounded_string(),
                            fiat_amount.currency()
                        ))
                        .size(14)
                        .color(color::GREY_3)
                    })),
            )
            .padding(20)
            .width(Length::Fill)
            .style(theme::container::balance_header),
        );

    // Action buttons
    content = content.push(
        Row::new()
            .spacing(10)
            .push(
                button::primary(Some(send_icon()), "Send")
                    .on_press(SparkOverviewMessage::Send)
                    .width(Length::Fill),
            )
            .push(
                button::secondary(Some(receive_icon()), "Receive")
                    .on_press(SparkOverviewMessage::Receive)
                    .width(Length::Fill),
            )
            .push(
                button::secondary(Some(arrow_down_up_icon()), "Move funds")
                    .on_press(SparkOverviewMessage::MoveFunds)
                    .width(Length::Fill),
            ),
    );

    if let Some(wallet_id) = wallet_id {
        content = content.push(
            card::simple(
                Column::new()
                    .spacing(6)
                    .push(iced_text("Wallet ID").size(12).color(color::GREY_3))
                    .push(iced_text(wallet_id).size(14)),
            )
            .width(Length::Fill),
        );
    }

    if let Some(error) = error {
        content = content.push(p1_regular(error).style(theme::text::error));
    }

    // Transaction history section
    content = content.push(
        Container::new(
            Column::new()
                .spacing(15)
                .push(
                    Row::new()
                        .spacing(10)
                        .align_y(Alignment::Center)
                        .push(h3_bold("Transaction History"))
                        .push({
                            let badge_text = if transactions.is_empty() {
                                text::p2_regular("Empty")
                            } else {
                                let count_str = transactions.len().to_string();
                                text::p2_regular(count_str)
                            };
                            badge::badge(badge_text)
                        }),
                )
                .push(if transactions.is_empty() {
                    card::simple(
                        Column::new()
                            .spacing(10)
                            .push(p1_regular("No transactions yet").style(theme::text::secondary))
                            .push(p2_regular(
                                "Send or receive Bitcoin to see your transaction history here",
                            ))
                            .align_x(Alignment::Center),
                    )
                    .width(Length::Fill)
                } else {
                    card::simple(
                        Column::with_children(
                            transactions
                                .iter()
                                .rev() // Show newest first
                                .take(20) // Limit to 20 most recent for performance
                                .map(|tx| transaction_row(tx, bitcoin_unit, None))
                                .collect::<Vec<Element<SparkOverviewMessage>>>(),
                        )
                        .spacing(10)
                        .width(Length::Fill),
                    )
                }),
        )
        .width(Length::Fill),
    );

    content = content.push(
        button::secondary(None, "Refresh")
            .on_press(SparkOverviewMessage::RefreshRequested)
            .width(Length::Fixed(160.0)),
    );

    Container::new(content)
        .width(Length::Fill)
        .max_width(900.0)
        .into()
}

fn transaction_row<'a>(
    transaction: &'a SparkTransaction,
    bitcoin_unit: BitcoinDisplayUnit,
    _fiat_converter: Option<&'a FiatAmountConverter>,
) -> Element<'a, SparkOverviewMessage> {
    use coincube_core::spark_wallet::SparkAssetType;

    let asset_type_str = match transaction.asset_type {
        SparkAssetType::Bitcoin => "BTC",
        SparkAssetType::Btkn => "BTKN",
    };

    let transaction_amount = Amount::from_sat(transaction.amount_sat);

    let direction = if transaction.direction == SparkTransactionDirection::Received {
        TransactionDirection::Incoming
    } else {
        TransactionDirection::Outgoing
    };

    let time_ago = format_time_ago(transaction.timestamp as i64);

    let mut item = TransactionListItem::new(direction, &transaction_amount, bitcoin_unit)
        .with_label(format!("{} {}", asset_type_str, transaction.counterparty))
        .with_time_ago(time_ago);

    item.view(SparkOverviewMessage::RefreshRequested).into()
}

pub fn spark_receive_view<'a>(
    receive_method: SparkReceiveMethod,
    current_address: Option<&'a str>,
    current_qr_data: Option<&'a qr_code::Data>,
    amount_input: &'a form::Value<String>,
    description_input: &'a str,
    loading: bool,
    error: Option<&'a str>,
    bitcoin_unit: BitcoinDisplayUnit,
    fiat_converter: Option<FiatAmountConverter>,
    invoice_expiry_seconds: Option<u64>,
) -> Element<'a, SparkReceiveMessage> {
    let mut content = Column::new()
        .spacing(20)
        .push(h3_bold("Receive via Spark"))
        .push(
            p1_regular(
                "Generate a Spark address or Lightning invoice to receive Bitcoin instantly.",
            )
            .style(theme::text::secondary),
        );

    // Method selection
    content = content.push(
        card::simple(
            Column::new()
                .spacing(15)
                .push(
                    Row::new()
                        .spacing(10)
                        .align_y(Alignment::Center)
                        .push(h4_bold("Receive Method"))
                        .push(badge::badge(
                            p2_regular(receive_method.display_name()).color(color::BLUE),
                        )),
                )
                .push(
                    Row::new()
                        .spacing(10)
                        .push(
                            button::secondary(None, "Spark Address")
                                .on_press(SparkReceiveMessage::ToggleMethod(
                                    SparkReceiveMethod::SparkAddress,
                                ))
                                .width(Length::Fill),
                        )
                        .push(
                            button::secondary(None, "Lightning Invoice")
                                .on_press(SparkReceiveMessage::ToggleMethod(
                                    SparkReceiveMethod::LightningInvoice,
                                ))
                                .width(Length::Fill),
                        ),
                ),
        )
        .width(Length::Fill),
    );

    // Amount input for Lightning invoices
    if receive_method == SparkReceiveMethod::LightningInvoice {
        content = content.push(
            card::simple(
                Column::new()
                    .spacing(15)
                    .push(h4_bold("Invoice Details"))
                    .push(form::Form::new(
                        "Amount (optional)",
                        amount_input,
                        SparkReceiveMessage::AmountInput,
                    ))
                    .push(
                        // Create a Value<String> for description input
                        {
                            let desc_value = form::Value {
                                value: description_input.to_string(),
                                warning: None,
                                valid: true,
                            };
                            form::Form::new(
                                "Description (optional)",
                                &desc_value,
                                SparkReceiveMessage::DescriptionInput,
                            )
                        },
                    )
                    .push(
                        button::primary(None, "Generate Invoice")
                            .on_press(SparkReceiveMessage::GenerateInvoice)
                            .width(Length::Fill),
                    ),
            )
            .width(Length::Fill),
        );
    }

    // Address/Invoice display
    if let Some(address) = current_address {
        content = content.push(
            card::simple(
                Column::new()
                    .spacing(15)
                    .push(
                        Row::new()
                            .spacing(10)
                            .align_y(Alignment::Center)
                            .push(h4_bold(match receive_method {
                                SparkReceiveMethod::SparkAddress => "Your Spark Address",
                                SparkReceiveMethod::LightningInvoice => "Your Lightning Invoice",
                            }))
                            .push_maybe(invoice_expiry_seconds.map(|seconds| {
                                let text = format!("Expires in {}m", seconds / 60);
                                let badge_text = p2_regular(text).color(color::ORANGE);
                                badge::badge(badge_text)
                            })),
                    )
                    .push(match current_qr_data {
                        Some(qr_data) => Container::new(qr_code::QRCode::new(qr_data).cell_size(4))
                            .width(Length::Fixed(200.0))
                            .height(Length::Fixed(200.0))
                            .center_x(Length::Fill)
                            .center_y(Length::Fill),
                        None => Container::new(Space::new())
                            .width(Length::Fixed(200.0))
                            .height(Length::Fixed(200.0))
                            .center_x(Length::Fill)
                            .center_y(Length::Fill),
                    })
                    .push(
                        Container::new(
                            iced_text(address)
                                .size(12)
                                .style(theme::text::secondary)
                                .width(Length::Fill),
                        )
                        .padding(10)
                        .style(theme::container::foreground)
                        .width(Length::Fill),
                    )
                    .push(
                        Row::new()
                            .spacing(10)
                            .push(
                                button::secondary(None, "Show QR Code")
                                    .on_press(SparkReceiveMessage::ShowQrCode)
                                    .width(Length::Fill),
                            )
                            .push(
                                button::secondary(None, "Copy")
                                    .on_press(SparkReceiveMessage::Copy)
                                    .width(Length::Fill),
                            ),
                    ),
            )
            .width(Length::Fill),
        );
    } else if receive_method == SparkReceiveMethod::SparkAddress {
        content = content.push(
            card::simple(
                Column::new()
                    .spacing(15)
                    .push(h4_bold("Generate Address"))
                    .push(
                        p2_regular("Click the button below to generate a new Spark address.")
                            .style(theme::text::secondary),
                    )
                    .push(
                        button::primary(None, "Generate Spark Address")
                            .on_press(SparkReceiveMessage::GenerateAddress)
                            .width(Length::Fill),
                    ),
            )
            .width(Length::Fill),
        );
    }

    if let Some(error) = error {
        content = content.push(p1_regular(error).style(theme::text::error));
    }

    Container::new(content)
        .width(Length::Fill)
        .max_width(900.0)
        .into()
}

pub fn spark_qr_modal<'a>(
    qr_data: &'a qr_code::Data,
    address: &'a str,
    receive_method: SparkReceiveMethod,
) -> Element<'a, SparkReceiveMessage> {
    Container::new(
        Column::new()
            .spacing(20)
            .push(
                Row::new()
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .push(h3_bold(match receive_method {
                        SparkReceiveMethod::SparkAddress => "Spark Address QR Code",
                        SparkReceiveMethod::LightningInvoice => "Lightning Invoice QR Code",
                    }))
                    .push(Space::new())
                    .push(
                        button::transparent(None, "")
                            .on_press(SparkReceiveMessage::CloseQrCode)
                            .style(theme::button::transparent),
                    ),
            )
            .push(
                Container::new(qr_code::QRCode::new(qr_data).cell_size(6))
                    .width(Length::Fixed(300.0))
                    .height(Length::Fixed(300.0))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .push(
                Container::new(
                    iced_text(address)
                        .size(12)
                        .style(theme::text::secondary)
                        .width(Length::Fill),
                )
                .padding(10)
                .style(theme::container::foreground)
                .width(Length::Fill),
            )
            .push(
                button::secondary(None, "Copy")
                    .on_press(SparkReceiveMessage::Copy)
                    .width(Length::Fill),
            ),
    )
    .padding(30)
    .width(Length::Fixed(400.0))
    .into()
}

pub fn spark_send_confirmation_modal<'a>(
    recipient: &'a str,
    amount_sat: u64,
    description: &'a str,
    fee_estimate: u64,
    bitcoin_unit: BitcoinDisplayUnit,
    fiat_converter: Option<FiatAmountConverter>,
) -> Element<'a, SparkSendMessage> {
    Container::new(
        Column::new()
            .spacing(20)
            .push(
                Row::new()
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .push(h3_bold("Confirm Payment"))
                    .push(Space::new())
                    .push(
                        button::transparent(None, "")
                            .on_press(SparkSendMessage::CancelSend)
                            .style(theme::button::transparent),
                    ),
            )
            .push(
                p1_regular("Please review your payment details before confirming.")
                    .style(theme::text::secondary),
            )
            .push(
                card::simple(
                    Column::new()
                        .spacing(15)
                        .push(
                            Row::new()
                                .spacing(10)
                                .align_y(Alignment::Center)
                                .push(h4_bold("Recipient"))
                                .push(Space::new())
                                .push({
                                    let badge_text = if recipient.is_empty() {
                                        text::p2_regular("Empty")
                                    } else {
                                        text::p2_regular("Valid")
                                    };
                                    badge::badge(badge_text)
                                }),
                        )
                        .push(
                            Container::new(
                                iced_text(recipient)
                                    .size(12)
                                    .style(theme::text::secondary)
                                    .width(Length::Fill),
                            )
                            .padding(10)
                            .style(theme::container::foreground)
                            .width(Length::Fill),
                        ),
                )
                .width(Length::Fill),
            )
            .push(
                card::simple(
                    Column::new()
                        .spacing(10)
                        .push(
                            Row::new()
                                .spacing(10)
                                .align_y(Alignment::Center)
                                .push(h4_bold("Amount"))
                                .push(Space::new())
                                .push(amount_with_size_and_unit(
                                    &Amount::from_sat(amount_sat),
                                    H4_SIZE,
                                    bitcoin_unit,
                                )),
                        )
                        .push_maybe(fiat_converter.clone().map(|converter| {
                            let fiat_amount = converter.convert(Amount::from_sat(amount_sat));
                            p2_regular(format!(
                                "~{} {}",
                                fiat_amount.to_rounded_string(),
                                fiat_amount.currency()
                            ))
                            .style(theme::text::secondary)
                        }))
                        .push_maybe(if !description.is_empty() {
                            Some(
                                Column::new()
                                    .spacing(5)
                                    .push(iced_text("Description").size(12).color(color::GREY_3))
                                    .push(iced_text(description).size(14)),
                            )
                        } else {
                            None
                        }),
                )
                .width(Length::Fill),
            )
            .push(
                card::simple(
                    Column::new()
                        .spacing(10)
                        .push(
                            Row::new()
                                .spacing(10)
                                .align_y(Alignment::Center)
                                .push(h4_bold("Fee"))
                                .push(Space::new())
                                .push({
                                    let badge_text = text::p2_regular("Low");
                                    badge::badge(badge_text)
                                }),
                        )
                        .push(
                            p2_regular("Spark transactions have near-zero fees.")
                                .style(theme::text::secondary),
                        ),
                )
                .width(Length::Fill),
            )
            .push(
                Row::new()
                    .spacing(10)
                    .push(
                        button::secondary(None, "Cancel")
                            .on_press(SparkSendMessage::CancelSend)
                            .width(Length::Fill),
                    )
                    .push(
                        button::primary(None, "Confirm & Send")
                            .on_press(SparkSendMessage::SendConfirmed)
                            .width(Length::Fill),
                    ),
            ),
    )
    .padding(30)
    .width(Length::Fixed(500.0))
    .into()
}

pub fn spark_send_view<'a>(
    recipient_input: &'a form::Value<String>,
    amount_input: &'a form::Value<String>,
    description_input: &'a str,
    fee_estimate: Option<u64>,
    loading: bool,
    sending: bool,
    error: Option<&'a str>,
    transaction_id: Option<&'a str>,
    amount_locked: bool,
    bitcoin_unit: BitcoinDisplayUnit,
    fiat_converter: Option<FiatAmountConverter>,
) -> Element<'a, SparkSendMessage> {
    let mut content = Column::new()
        .spacing(20)
        .push(h3_bold("Send via Spark"))
        .push(
        p1_regular(
            "Send Bitcoin instantly to a Spark address or Lightning invoice with near-zero fees.",
        )
        .style(theme::text::secondary),
    );

    if let Some(txid) = transaction_id {
        // Success screen
        content = content.push(
            card::simple(
                Column::new()
                    .spacing(15)
                    .push(
                        Row::new()
                            .spacing(10)
                            .align_y(Alignment::Center)
                            .push(check_circle_icon().color(color::GREEN))
                            .push(h4_bold("Payment Sent!")),
                    )
                    .push(
                        p2_regular("Your Bitcoin has been sent successfully via Spark.")
                            .style(theme::text::secondary),
                    )
                    .push(
                        Container::new(
                            Column::new()
                                .spacing(6)
                                .push(iced_text("Transaction ID").size(12).color(color::GREY_3))
                                .push(iced_text(txid).size(14)),
                        )
                        .padding(10)
                        .style(theme::container::foreground)
                        .width(Length::Fill),
                    )
                    .push(
                        Row::new()
                            .spacing(10)
                            .push(
                                button::secondary(None, "View in History")
                                    .on_press(SparkSendMessage::Close)
                                    .width(Length::Fill),
                            )
                            .push(
                                button::primary(None, "Send Another")
                                    .on_press(SparkSendMessage::Close)
                                    .width(Length::Fill),
                            ),
                    ),
            )
            .width(Length::Fill),
        );
    } else {
        // Send form
        content = content.push(
            card::simple(
                Column::new()
                    .spacing(15)
                    .push(h4_bold("Recipient"))
                    .push(
                        Container::new(form::Form::new(
                            "Spark address or Lightning invoice",
                            recipient_input,
                            SparkSendMessage::RecipientInput,
                        ))
                        .width(Length::Fill),
                    )
                    .push(
                        p2_regular("Enter a Spark address (sp1...) or Lightning invoice (lnbc...)")
                            .style(theme::text::secondary),
                    ),
            )
            .width(Length::Fill),
        );

        content = content.push(
            card::simple(
                Column::new()
                    .spacing(15)
                    .push(
                        Row::new()
                            .spacing(10)
                            .align_y(Alignment::Center)
                            .push(h4_bold("Amount"))
                            .push_maybe(if amount_locked {
                                Some({
                                    let badge_text = text::p2_regular("Locked (from invoice)");
                                    badge::badge(badge_text)
                                })
                            } else {
                                None
                            }),
                    )
                    .push(if amount_locked {
                        // Amount is locked from BOLT11 invoice, show disabled field
                        Container::new(form::Form::new_disabled(
                            "Amount from invoice",
                            amount_input,
                        ))
                        .width(Length::Fill)
                    } else {
                        // User can enter amount
                        Container::new(form::Form::new(
                            "Amount in sats",
                            amount_input,
                            SparkSendMessage::AmountInput,
                        ))
                        .width(Length::Fill)
                    })
                    .push_maybe(if !amount_locked {
                        // Only show amount controls when amount is not locked
                        Some(
                            Row::new()
                                .spacing(10)
                                .push(
                                    button::secondary(None, "Use Max")
                                        .on_press(SparkSendMessage::FeeEstimateRequested)
                                        .width(Length::Fill),
                                )
                                .push(
                                    button::secondary(None, "Clear")
                                        .on_press(SparkSendMessage::AmountInput("".to_string()))
                                        .width(Length::Fill),
                                ),
                        )
                    } else {
                        None
                    }),
            )
            .width(Length::Fill),
        );

        content = content.push(
            card::simple(
                Column::new()
                    .spacing(15)
                    .push(h4_bold("Description (optional)"))
                    .push(
                        // Create a Value<String> for description input
                        {
                            let desc_value = form::Value {
                                value: description_input.to_string(),
                                warning: None,
                                valid: true,
                            };
                            Container::new(form::Form::new(
                                "What's this payment for?",
                                &desc_value,
                                SparkSendMessage::DescriptionInput,
                            ))
                            .width(Length::Fill)
                        },
                    ),
            )
            .width(Length::Fill),
        );

        // Fee estimate
        if let Some(fee) = fee_estimate {
            content = content.push(
                card::simple(
                    Column::new()
                        .spacing(10)
                        .push(
                            Row::new()
                                .spacing(10)
                                .align_y(Alignment::Center)
                                .push(h4_bold("Fee Estimate"))
                                .push({
                                    let fee_text = if fee <= 1 { "< 1 sat" } else { "Low fee" };
                                    let badge_text = text::p2_regular(fee_text);
                                    badge::badge(badge_text)
                                }),
                        )
                        .push(
                            p2_regular("Spark transactions have near-zero fees.")
                                .style(theme::text::secondary),
                        ),
                )
                .width(Length::Fill),
            );
        }

        // Send button
        let send_button = if sending {
            button::primary(None, "Sending...").width(Length::Fill)
        } else {
            button::primary(None, "Send")
                .on_press(SparkSendMessage::ShowConfirmation)
                .width(Length::Fill)
        };

        content = content.push(send_button);
    }

    if let Some(error) = error {
        content = content.push(p1_regular(error).style(theme::text::error));
    }

    Container::new(content)
        .width(Length::Fill)
        .max_width(900.0)
        .into()
}

pub fn spark_move_funds_view<'a>(
    direction: MoveFundsDirection,
    amount_input: &'a form::Value<String>,
    fee_estimate: Option<u64>,
    expected_time: Option<u64>,
    loading: bool,
    error: Option<&'a str>,
    bitcoin_unit: BitcoinDisplayUnit,
    fiat_converter: Option<FiatAmountConverter>,
) -> Element<'a, SparkMoveFundsMessage> {
    let mut content = Column::new().spacing(20).push(h3_bold("Move Funds")).push(
        p1_regular("Transfer Bitcoin between your Vault and Spark Wallet.")
            .style(theme::text::secondary),
    );

    // Direction selection
    content = content.push(
        card::simple(
            Column::new()
                .spacing(15)
                .push(
                    Row::new()
                        .spacing(10)
                        .align_y(Alignment::Center)
                        .push(h4_bold("Transfer Direction"))
                        .push(badge::badge(
                            p2_regular(direction.display_name()).color(color::BLUE),
                        )),
                )
                .push(
                    Row::new()
                        .spacing(10)
                        .push(
                            button::secondary(None, "Vault → Spark")
                                .on_press(SparkMoveFundsMessage::ToggleDirection(
                                    MoveFundsDirection::VaultToSpark,
                                ))
                                .width(Length::Fill),
                        )
                        .push(
                            button::secondary(None, "Spark → Vault")
                                .on_press(SparkMoveFundsMessage::ToggleDirection(
                                    MoveFundsDirection::SparkToVault,
                                ))
                                .width(Length::Fill),
                        ),
                ),
        )
        .width(Length::Fill),
    );

    // Amount input
    content = content.push(
        card::simple(
            Column::new()
                .spacing(15)
                .push(h4_bold("Amount"))
                .push(
                    Container::new(form::Form::new(
                        "Amount in sats",
                        amount_input,
                        SparkMoveFundsMessage::AmountInput,
                    ))
                    .width(Length::Fill),
                )
                .push(
                    Row::new()
                        .spacing(10)
                        .push(
                            button::secondary(None, "Use Max")
                                .on_press(SparkMoveFundsMessage::EstimateRequested)
                                .width(Length::Fill),
                        )
                        .push(
                            button::secondary(None, "Clear")
                                .on_press(SparkMoveFundsMessage::AmountInput("".to_string()))
                                .width(Length::Fill),
                        ),
                ),
        )
        .width(Length::Fill),
    );

    // Fee/Time estimate based on direction
    match direction {
        MoveFundsDirection::VaultToSpark => {
            if let Some(fee) = fee_estimate {
                content = content.push(
                    card::simple(
                        Column::new()
                            .spacing(10)
                            .push(
                                Row::new()
                                    .spacing(10)
                                    .align_y(Alignment::Center)
                                    .push(h4_bold("On-chain Fee Estimate"))
                                    .push({
                                        let badge_text = text::p2_regular("Estimated");
                                        badge::badge(badge_text)
                                    }),
                            )
                            .push(
                                p2_regular(format!("~{} sats (on-chain transaction fee)", fee))
                                    .style(theme::text::secondary),
                            ),
                    )
                    .width(Length::Fill),
                );
            }
        }
        MoveFundsDirection::SparkToVault => {
            if let Some(time) = expected_time {
                content = content.push(
                    card::simple(
                        Column::new()
                            .spacing(10)
                            .push(
                                Row::new()
                                    .spacing(10)
                                    .align_y(Alignment::Center)
                                    .push(h4_bold("Expected Time"))
                                    .push({
                                        let badge_text = text::p2_regular("Cooperative exit");
                                        badge::badge(badge_text)
                                    }),
                            )
                            .push(
                                p2_regular(format!(
                                    "~{} minutes (cooperative exit path)",
                                    time / 60
                                ))
                                .style(theme::text::secondary),
                            ),
                    )
                    .width(Length::Fill),
                );
            }
        }
    }

    // Move button
    let move_button = if loading {
        button::primary(None, "Moving...").width(Length::Fill)
    } else {
        button::primary(None, "Move Funds")
            .on_press(SparkMoveFundsMessage::MoveRequested)
            .width(Length::Fill)
    };

    content = content.push(move_button);

    if let Some(error) = error {
        content = content.push(p1_regular(error).style(theme::text::error));
    }

    Container::new(content)
        .width(Length::Fill)
        .max_width(900.0)
        .into()
}
