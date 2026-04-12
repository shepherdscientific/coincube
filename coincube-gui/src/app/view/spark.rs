use coincube_core::{
    miniscript::bitcoin::Amount,
    spark_wallet::{SparkTransaction, SparkTransactionDirection, SparkTransactionStatus},
};
use coincube_ui::{
    color,
    component::{amount::*, badge, button, card, text::*},
    icon::{lightning_icon, receive_icon, send_icon},
    theme,
    widget::*,
};
use iced::{Alignment, Length};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::view::{FiatAmountConverter, SparkOverviewMessage};

pub fn spark_overview_view<'a>(
    wallet_id: Option<&'a str>,
    balance: Amount,
    transactions: &'a [SparkTransaction],
    error: Option<&'a str>,
    bitcoin_unit: BitcoinDisplayUnit,
    fiat_converter: Option<&'a FiatAmountConverter>,
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
                    .spacing(10)
                    .push(
                        Row::new()
                            .spacing(10)
                            .align_y(Alignment::Center)
                            .push(lightning_icon().size(18).color(color::ORANGE))
                            .push(text("Available balance").size(13).color(color::GREY_2)),
                    )
                    .push(amount_with_size_and_unit(&balance, H2_SIZE, bitcoin_unit))
                    .push_maybe(fiat_converter.map(|converter| {
                        let fiat_amount = converter.convert(balance);
                        text(format!(
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

    if let Some(wallet_id) = wallet_id {
        content = content.push(
            card::simple(
                Column::new()
                    .spacing(6)
                    .push(text("Wallet ID").size(12).color(color::GREY_3))
                    .push(text(wallet_id).size(14)),
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
                        .push(badge::badge(
                            if transactions.is_empty() {
                                "Empty"
                            } else {
                                format!("{}", transactions.len()).as_str()
                            },
                            color::GREY_3,
                        )),
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
                    .into()
                } else {
                    Column::with_children(
                        transactions
                            .iter()
                            .rev() // Show newest first
                            .take(20) // Limit to 20 most recent for performance
                            .map(|tx| transaction_row(tx, bitcoin_unit, fiat_converter))
                            .collect(),
                    )
                    .spacing(10)
                    .width(Length::Fill)
                    .into()
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
    fiat_converter: Option<&'a FiatAmountConverter>,
) -> widget::Container<'a, SparkOverviewMessage, theme::Theme> {
    let (direction_text, direction_color, direction_icon) = match transaction.direction {
        SparkTransactionDirection::Sent => ("Sent", color::RED, send_icon().color(color::RED)),
        SparkTransactionDirection::Received => {
            ("Received", color::GREEN, receive_icon().color(color::GREEN))
        }
    };

    let status_text = match transaction.status {
        SparkTransactionStatus::Pending => "Pending",
        SparkTransactionStatus::Confirmed => "Confirmed",
    };
    let status_color = match transaction.status {
        SparkTransactionStatus::Pending => color::ORANGE,
        SparkTransactionStatus::Confirmed => color::GREEN,
    };

    // Convert timestamp to readable format
    let timestamp = if transaction.timestamp > 0 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let diff = now.saturating_sub(transaction.timestamp);

        if diff < 60 {
            format!("{}s ago", diff)
        } else if diff < 3600 {
            format!("{}m ago", diff / 60)
        } else if diff < 86400 {
            format!("{}h ago", diff / 3600)
        } else {
            format!("{}d ago", diff / 86400)
        }
    } else {
        "Unknown".to_string()
    };

    card::simple(
        widget::column![
            widget::row![
                widget::column![
                    text::p2_medium("Transaction").style(theme::text::secondary),
                    text::p2_bold(&transaction.txid[0..16]).style(theme::text::primary)
                ]
                .width(Length::Fill),
                badge::badge(direction_text, direction_color),
                widget::Space::new().width(8),
                badge::badge(status_text, status_color)
            ]
            .align_y(Alignment::Center),
            widget::Space::new().height(12),
            widget::row![
                widget::column![
                    text::p2_medium("Amount").style(theme::text::secondary),
                    amount_with_size_and_unit(
                        &Amount::from_sat(transaction.amount_sat),
                        P2_SIZE,
                        bitcoin_unit
                    )
                    .style(
                        if transaction.direction == SparkTransactionDirection::Sent {
                            theme::text::error
                        } else {
                            theme::text::success
                        }
                    ),
                    widget::Space::new().height(4),
                    widget::container(
                        fiat_converter
                            .map(|converter| {
                                let fiat_amount =
                                    converter.convert(Amount::from_sat(transaction.amount_sat));
                                text(format!("~{}", fiat_amount.to_rounded_string()))
                                    .size(12)
                                    .color(color::GREY_3)
                            })
                            .unwrap_or_else(|| widget::Space::new().into())
                    )
                ]
                .width(Length::Fill),
                widget::column![
                    text::p2_medium("Counterparty").style(theme::text::secondary),
                    text::p2_bold(&transaction.counterparty).style(theme::text::primary)
                ]
                .width(Length::Fill),
                widget::column![
                    text::p2_medium("Time").style(theme::text::secondary),
                    text::p2_bold(timestamp).style(theme::text::primary)
                ]
                .width(Length::Fill),
                direction_icon.size(20)
            ]
            .align_y(Alignment::Center)
        ]
        .padding(16),
    )
    .width(Length::Fill)
}
