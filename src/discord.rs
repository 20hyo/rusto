use crate::binance::NetworkStats;
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
            ExecutionEvent::PositionLiquidated(position) => {
                self.send_position_liquidated(&position).await;
            }
            ExecutionEvent::TP1Filled { position_id, tp1_price, partial_pnl } => {
                self.send_tp1_filled(&position_id, tp1_price, partial_pnl).await;
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

        // Calculate notional value
        let notional_value = position.entry_price * position.quantity;

        let message = format!(
            "{} **선물 포지션 진입 ({}배)**\n\
            **심볼**: {}\n\
            **방향**: {:?}\n\
            **전략**: {}\n\
            **진입가**: ${}\n\
            **손절가**: ${}\n\
            **목표가**: ${}\n\
            **청산가**: ${} ⚠️\n\
            **레버리지**: {}x\n\
            **마진 타입**: {}\n\
            **수량**: {}\n\
            **포지션 가치**: ${:.2}\n\
            **필요 증거금**: ${:.2}\n\
            **유지 증거금**: ${:.2}\n\
            **시간**: {}",
            side_emoji,
            position.leverage,
            position.symbol.to_uppercase(),
            position.side,
            position.setup,
            position.entry_price,
            position.stop_loss,
            position.take_profit,
            position.liquidation_price,
            position.leverage,
            position.margin_type,
            position.quantity,
            notional_value,
            position.initial_margin,
            position.maintenance_margin,
            position.entry_time.format("%Y-%m-%d %H:%M:%S UTC")
        );

        self.send_embed("🚀 포지션 진입", &message, 0x00FF00).await;
    }

    async fn send_position_closed(&self, position: &Position) {
        let pnl = position.pnl;
        let entry_price = position.entry_price;
        let exit_price = position.exit_price.unwrap_or(entry_price);

        // ROI 계산: (PnL / Initial Margin) * 100
        let roi = if position.initial_margin > Decimal::ZERO {
            (pnl / position.initial_margin) * Decimal::from(100)
        } else {
            Decimal::ZERO
        };

        // 수익률 계산 (포지션 가치 대비)
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
            "{} **선물 포지션 청산 ({}배)**\n\
            **심볼**: {}\n\
            **방향**: {:?}\n\
            **전략**: {}\n\
            **진입가**: ${}\n\
            **청산가**: ${}\n\
            **레버리지**: {}x\n\
            **수량**: {}\n\
            **손익**: ${:.2}\n\
            **ROI (증거금 대비)**: {:.2}%\n\
            **수익률 (포지션 대비)**: {:.2}%\n\
            **진입시간**: {}\n\
            **청산시간**: {}",
            emoji,
            position.leverage,
            position.symbol.to_uppercase(),
            position.side,
            position.setup,
            entry_price,
            exit_price,
            position.leverage,
            position.quantity,
            pnl,
            roi,
            pnl_pct,
            position.entry_time.format("%Y-%m-%d %H:%M:%S UTC"),
            position.exit_time.map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "N/A".to_string())
        );

        self.send_embed("💰 포지션 청산", &message, color).await;
    }

    async fn send_position_liquidated(&self, position: &Position) {
        let pnl = position.pnl;
        let entry_price = position.entry_price;
        let liquidation_price = position.liquidation_price;

        // ROI 계산
        let roi = if position.initial_margin > Decimal::ZERO {
            (pnl / position.initial_margin) * Decimal::from(100)
        } else {
            Decimal::ZERO
        };

        let message = format!(
            "💀 **포지션 강제 청산 (LIQUIDATED)**\n\
            **심볼**: {}\n\
            **방향**: {:?}\n\
            **전략**: {}\n\
            **진입가**: ${}\n\
            **청산가**: ${}\n\
            **레버리지**: {}x\n\
            **마진 타입**: {}\n\
            **수량**: {}\n\
            **손실**: ${:.2}\n\
            **ROI**: {:.2}%\n\
            **진입시간**: {}\n\
            **청산시간**: {}\n\
            ⚠️ **청산 사유**: 가격이 청산가에 도달하여 강제 청산되었습니다.",
            position.symbol.to_uppercase(),
            position.side,
            position.setup,
            entry_price,
            liquidation_price,
            position.leverage,
            position.margin_type,
            position.quantity,
            pnl,
            roi,
            position.entry_time.format("%Y-%m-%d %H:%M:%S UTC"),
            position.exit_time.map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "N/A".to_string())
        );

        self.send_embed("⚠️ 강제 청산", &message, 0xFF0000).await;
    }

    async fn send_tp1_filled(&self, position_id: &str, tp1_price: Decimal, partial_pnl: Decimal) {
        let (emoji, color) = if partial_pnl >= Decimal::ZERO {
            ("✅", 0x00FF00)
        } else {
            ("⚠️", 0xFFAA00)
        };

        let message = format!(
            "{} **TP1 달성 (50% 청산)**\n\
            **포지션 ID**: {}\n\
            **TP1 가격**: ${} (VWAP)\n\
            **부분 손익**: ${:.2}\n\
            **상태**: 50% 청산 완료, 손절가 → 본절로 이동",
            emoji, position_id, tp1_price, partial_pnl
        );

        self.send_embed("🎯 TP1 달성", &message, color).await;
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

    /// Send startup notification with network stats
    pub async fn send_startup_message(&self, stats: &NetworkStats, symbols: &[String]) {
        // Determine ping quality
        let (ping_emoji, ping_status) = if stats.avg_latency_ms < 10.0 {
            ("🟢", "매우 좋음")
        } else if stats.avg_latency_ms < 20.0 {
            ("🟡", "양호")
        } else if stats.avg_latency_ms < 50.0 {
            ("🟠", "보통")
        } else {
            ("🔴", "느림")
        };

        // Determine time sync status
        let (sync_emoji, sync_status) = if stats.time_offset_ms.abs() < 100 {
            ("✅", "정상")
        } else if stats.time_offset_ms.abs() < 300 {
            ("⚠️", "주의")
        } else {
            ("❌", "경고")
        };

        let symbols_list = symbols
            .iter()
            .map(|s| s.to_uppercase())
            .collect::<Vec<_>>()
            .join(", ");

        let message = format!(
            "🚀 **Rusto 페이퍼 트레이딩 봇 시작**\n\n\
            📡 **네트워크 상태**\n\
            {} **평균 핑**: {:.2}ms ({})\n\
            **최소/최대 핑**: {:.2}ms / {:.2}ms\n\
            {} **시간 동기화**: {}ms 오프셋 ({})\n\
            **측정 샘플**: {}회\n\n\
            💹 **거래 설정**\n\
            **심볼**: {}\n\
            **모드**: 페이퍼 트레이딩 (시뮬레이션)\n\n\
            ⏰ **시작 시간**: {}\n\n\
            ✅ 모든 Pre-flight 체크 통과. 매매 시작합니다!",
            ping_emoji,
            stats.avg_latency_ms,
            ping_status,
            stats.min_latency_ms,
            stats.max_latency_ms,
            sync_emoji,
            stats.time_offset_ms,
            sync_status,
            stats.samples,
            symbols_list,
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        );

        self.send_embed("🎯 봇 시작", &message, 0x00BFFF).await;
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
