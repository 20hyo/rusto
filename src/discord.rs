use crate::types::{ExecutionEvent, Position, Side};
use reqwest::Client;
use rust_decimal::Decimal;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::{error, info};

/// Discord notification bot that sends trade alerts via webhook
pub struct DiscordBot {
    webhook_url: String,
    client: Client,
}

impl DiscordBot {
    pub fn new(webhook_url: String) -> Self {
        Self {
            webhook_url,
            client: Client::new(),
        }
    }

    /// Main loop: monitor channel and send notifications
    pub async fn run(
        &self,
        mut execution_rx: mpsc::Receiver<ExecutionEvent>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!("Discord bot started");

        loop {
            tokio::select! {
                Some(event) = execution_rx.recv() => {
                    self.handle_execution_event(event).await;
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Discord bot shutting down");
                        return;
                    }
                }
            }
        }
    }

    async fn handle_execution_event(&self, event: ExecutionEvent) {
        match event {
            ExecutionEvent::PositionOpened(position) => {
                self.send_position_opened(&position).await;
            }
            ExecutionEvent::PositionClosed(position) => {
                self.send_position_closed(&position).await;
            }
            ExecutionEvent::StopMoved { position_id, new_stop } => {
                self.send_stop_moved(&position_id, new_stop).await;
            }
            ExecutionEvent::DailyLimitReached { pnl } => {
                self.send_daily_limit_reached(pnl).await;
            }
        }
    }

    async fn send_position_opened(&self, position: &Position) {
        let side_emoji = match position.side {
            Side::Buy => "🟢",
            Side::Sell => "🔴",
        };

        let message = format!(
            "{} **포지션 진입**\n\
            **심볼**: {}\n\
            **방향**: {:?}\n\
            **전략**: {}\n\
            **진입가**: ${}\n\
            **손절가**: ${}\n\
            **목표가**: ${}\n\
            **수량**: {}\n\
            **시간**: {}",
            side_emoji,
            position.symbol.to_uppercase(),
            position.side,
            position.setup,
            position.entry_price,
            position.stop_loss,
            position.take_profit,
            position.quantity,
            position.entry_time.format("%Y-%m-%d %H:%M:%S UTC")
        );

        self.send_embed("포지션 진입", &message, 0x00FF00).await;
    }

    async fn send_position_closed(&self, position: &Position) {
        let pnl = position.pnl;
        let entry_price = position.entry_price;
        let exit_price = position.exit_price.unwrap_or(entry_price);

        // 수익률 계산 (%)
        let pnl_pct = if entry_price > Decimal::ZERO {
            (pnl / (entry_price * position.quantity)) * Decimal::from(100)
        } else {
            Decimal::ZERO
        };

        let (emoji, color) = if pnl >= Decimal::ZERO {
            ("✅", 0x00FF00)
        } else {
            ("❌", 0xFF0000)
        };

        let message = format!(
            "{} **포지션 청산**\n\
            **심볼**: {}\n\
            **방향**: {:?}\n\
            **전략**: {}\n\
            **진입가**: ${}\n\
            **청산가**: ${}\n\
            **수량**: {}\n\
            **손익**: ${:.2}\n\
            **수익률**: {:.2}%\n\
            **진입시간**: {}\n\
            **청산시간**: {}",
            emoji,
            position.symbol.to_uppercase(),
            position.side,
            position.setup,
            entry_price,
            exit_price,
            position.quantity,
            pnl,
            pnl_pct,
            position.entry_time.format("%Y-%m-%d %H:%M:%S UTC"),
            position.exit_time.map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "N/A".to_string())
        );

        self.send_embed("포지션 청산", &message, color).await;
    }

    async fn send_stop_moved(&self, position_id: &str, new_stop: Decimal) {
        let message = format!(
            "🔄 **손절가 이동**\n\
            **포지션 ID**: {}\n\
            **새 손절가**: ${} (손익분기점)",
            position_id, new_stop
        );

        self.send_embed("손절가 이동", &message, 0xFFFF00).await;
    }

    async fn send_daily_limit_reached(&self, pnl: Decimal) {
        let message = format!(
            "⚠️ **일일 손실 한도 도달**\n\
            **금일 손익**: ${:.2}\n\
            **상태**: 매매 중단",
            pnl
        );

        self.send_embed("일일 한도 도달", &message, 0xFF0000).await;
    }

    async fn send_embed(&self, title: &str, description: &str, color: u32) {
        let payload = json!({
            "embeds": [{
                "title": title,
                "description": description,
                "color": color,
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "footer": {
                    "text": "Rusto Trading Bot"
                }
            }]
        });

        if let Err(e) = self
            .client
            .post(&self.webhook_url)
            .json(&payload)
            .send()
            .await
        {
            error!("Failed to send Discord notification: {}", e);
        } else {
            info!("Discord notification sent: {}", title);
        }
    }
}
