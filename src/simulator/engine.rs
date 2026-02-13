use crate::binance::ExchangeInfoManager;
use crate::config::SimulatorConfig;
use crate::risk::RiskManager;
use crate::simulator::order_book::LocalOrderBook;
use crate::simulator::position::PositionManager;
use crate::simulator::trade_log::TradeLogger;
use crate::types::{
    BotStats, DepthUpdate, ExecutionEvent, MarginType, MarketEvent, NormalizedTrade,
    ProcessingEvent, TradeSignal,
};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::types::VolumeProfileSnapshot;

/// Paper trading execution engine with leverage support
pub struct SimulatorEngine {
    config: SimulatorConfig,
    risk_manager: RiskManager,
    position_manager: PositionManager,
    trade_logger: TradeLogger,
    order_books: BTreeMap<String, LocalOrderBook>,
    fee_rate: Decimal,
    execution_tx: Option<mpsc::Sender<ExecutionEvent>>,
    leverage: Decimal,
    margin_type: MarginType,
    maintenance_margin_rate: Decimal,
    exchange_info: Option<Arc<ExchangeInfoManager>>,
    latest_profiles: BTreeMap<String, VolumeProfileSnapshot>,
    /// Shared state read by the hourly reporter task
    bot_stats: Option<Arc<Mutex<BotStats>>>,
}

impl SimulatorEngine {
    pub fn new(
        config: SimulatorConfig,
        risk_manager: RiskManager,
        trade_logger: TradeLogger,
    ) -> Self {
        let fee_rate = Decimal::try_from(config.taker_fee).unwrap_or_else(|_| Decimal::new(4, 4));
        let leverage = Decimal::try_from(config.leverage).unwrap_or(Decimal::from(100));
        let maintenance_margin_rate = Decimal::try_from(config.maintenance_margin_rate)
            .unwrap_or_else(|_| Decimal::new(4, 3)); // 0.004
        let margin_type = match config.margin_type.to_lowercase().as_str() {
            "cross" => MarginType::Cross,
            _ => MarginType::Isolated,
        };

        Self {
            config,
            risk_manager,
            position_manager: PositionManager::new(),
            trade_logger,
            order_books: BTreeMap::new(),
            fee_rate,
            execution_tx: None,
            leverage,
            margin_type,
            maintenance_margin_rate,
            exchange_info: None,
            latest_profiles: BTreeMap::new(),
            bot_stats: None,
        }
    }

    pub fn set_execution_channel(&mut self, tx: mpsc::Sender<ExecutionEvent>) {
        self.execution_tx = Some(tx);
    }

    pub fn set_exchange_info(&mut self, exchange_info: Arc<ExchangeInfoManager>) {
        self.exchange_info = Some(exchange_info);
    }

    pub fn set_bot_stats(&mut self, stats: Arc<Mutex<BotStats>>) {
        self.bot_stats = Some(stats);
    }

    /// Main loop: consume processing events and market events
    pub async fn run(
        &mut self,
        mut processing_rx: mpsc::Receiver<ProcessingEvent>,
        mut market_rx: tokio::sync::broadcast::Receiver<MarketEvent>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!("Simulator engine started");

        loop {
            tokio::select! {
                // Processing events (signals)
                Some(event) = processing_rx.recv() => {
                    self.handle_processing_event(event);
                }
                // Market events (for position management)
                Ok(event) = market_rx.recv() => {
                    self.handle_market_event(event);
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Simulator engine shutting down");
                        self.shutdown_summary();
                        return;
                    }
                }
            }
        }
    }

    /// Sync current balance/pnl/positions into the shared BotStats
    fn sync_bot_stats(&self) {
        if let Some(stats) = &self.bot_stats {
            if let Ok(mut s) = stats.lock() {
                s.balance = self.risk_manager.balance();
                s.daily_pnl = self.risk_manager.daily_pnl();
                s.open_positions = self.position_manager.open_positions().len();
            }
        }
    }

    fn handle_processing_event(&mut self, event: ProcessingEvent) {
        match event {
            ProcessingEvent::Signal(signal) => {
                self.execute_signal(signal);
            }
            ProcessingEvent::VolumeProfile(profile) => {
                self.latest_profiles.insert(profile.symbol.clone(), profile);
            }
            _ => {
                // Other events (bars, flow) handled by processing task
            }
        }
    }

    fn handle_market_event(&mut self, event: MarketEvent) {
        match event {
            MarketEvent::Trade(trade) => {
                self.on_trade(&trade);
            }
            MarketEvent::Depth(depth) => {
                self.on_depth(&depth);
            }
        }
    }

    fn execute_signal(&mut self, signal: TradeSignal) {
        if !self.risk_manager.can_trade(&signal) {
            warn!(
                symbol = %signal.symbol,
                setup = %signal.setup,
                "Signal rejected by risk manager"
            );
            return;
        }

        let quantity = self.risk_manager.calculate_position_size(&signal);
        if quantity <= Decimal::ZERO {
            warn!("Position size is zero, skipping");
            return;
        }

        // Validate and adjust order parameters using exchange info
        let (validated_entry, validated_quantity) = if let Some(ref exchange_info) = self.exchange_info {
            if let Some(symbol_info) = exchange_info.get_symbol_info(&signal.symbol) {
                match symbol_info.validate_order(signal.entry_price, quantity) {
                    Ok((rounded_price, rounded_qty)) => {
                        if rounded_price != signal.entry_price || rounded_qty != quantity {
                            info!(
                                symbol = %signal.symbol,
                                original_price = %signal.entry_price,
                                rounded_price = %rounded_price,
                                original_qty = %quantity,
                                rounded_qty = %rounded_qty,
                                "Order parameters adjusted to exchange filters"
                            );
                        }
                        (rounded_price, rounded_qty)
                    }
                    Err(e) => {
                        warn!(
                            symbol = %signal.symbol,
                            entry = %signal.entry_price,
                            quantity = %quantity,
                            error = ?e,
                            "Order validation failed"
                        );
                        return;
                    }
                }
            } else {
                warn!(
                    symbol = %signal.symbol,
                    "Symbol info not found in exchange info, using original values"
                );
                (signal.entry_price, quantity)
            }
        } else {
            // No exchange info available, use original values
            (signal.entry_price, quantity)
        };

        // Create modified signal with validated values
        let mut validated_signal = signal.clone();
        validated_signal.entry_price = validated_entry;

        let mut position = self.position_manager.open_position(
            &validated_signal,
            validated_quantity,
            self.leverage,
            self.margin_type,
            self.maintenance_margin_rate,
            self.fee_rate,
        );

        // For AdvancedOrderFlow strategy, set TP1/TP2 from volume profile
        if position.setup == crate::types::SetupType::AdvancedOrderFlow {
            if let Some(profile) = self.latest_profiles.get(&position.symbol) {
                position.tp1_price = Some(profile.vwap);
                position.tp2_price = Some(profile.vah);

                info!(
                    position_id = %position.id,
                    tp1_vwap = %profile.vwap,
                    tp2_vah = %profile.vah,
                    "AdvancedOrderFlow: TP1/TP2 set from profile"
                );
            }
        }

        self.risk_manager.register_position(&position);

        info!(
            id = %position.id,
            symbol = %position.symbol,
            side = ?position.side,
            setup = %position.setup,
            entry = %position.entry_price,
            stop = %position.stop_loss,
            target = %position.take_profit,
            liquidation = %position.liquidation_price,
            leverage = %position.leverage,
            margin_type = %position.margin_type,
            qty = %position.quantity,
            "Position opened"
        );

        // Send execution event
        if let Some(tx) = &self.execution_tx {
            let _ = tx.try_send(ExecutionEvent::PositionOpened(position));
        }
    }

    fn on_trade(&mut self, trade: &NormalizedTrade) {
        // Keep shared stats up to date for the hourly reporter task
        self.sync_bot_stats();

        // First, check for liquidations (highest priority)
        let liquidated = self.check_liquidations(&trade.symbol, trade.price);
        for position in &liquidated {
            self.risk_manager.close_position(position);
            self.trade_logger.log_trade(position);

            warn!(
                id = %position.id,
                symbol = %position.symbol,
                pnl = %position.pnl,
                exit_price = %position.liquidation_price,
                liquidation_price = %position.liquidation_price,
                "POSITION LIQUIDATED"
            );

            // Send liquidation event
            if let Some(tx) = &self.execution_tx {
                let _ = tx.try_send(ExecutionEvent::PositionLiquidated(position.clone()));
            }
        }

        // Check multi-stage exits (TP1/TP2/Soft Stop) for AdvancedOrderFlow
        self.check_multi_stage_exits(&trade.symbol, trade.price, trade.timestamp);

        // Then check normal exits (stop loss / take profit)
        let closed = self.position_manager.check_exits(
            &trade.symbol,
            trade.price,
            self.fee_rate,
        );

        for position in &closed {
            self.risk_manager.close_position(position);
            self.trade_logger.log_trade(position);

            info!(
                id = %position.id,
                symbol = %position.symbol,
                pnl = %position.pnl,
                exit_price = %position.exit_price.unwrap_or_default(),
                "Position closed"
            );

            // Send execution event
            if let Some(tx) = &self.execution_tx {
                let _ = tx.try_send(ExecutionEvent::PositionClosed(position.clone()));
            }
        }

        // Check break-even moves
        let open_positions: Vec<_> = self
            .position_manager
            .open_positions_for(&trade.symbol)
            .into_iter()
            .map(|p| p.id.clone())
            .collect();

        for pos_id in open_positions {
            if let Some(pos) = self
                .position_manager
                .open_positions()
                .iter()
                .find(|p| p.id == pos_id)
            {
                if self
                    .risk_manager
                    .should_move_to_break_even(pos, trade.price)
                {
                    let entry_price = pos.entry_price; // Copy before mutable borrow
                    if self.position_manager.move_stop_to_break_even(&pos_id) {
                        info!(
                            position_id = %pos_id,
                            "Stop moved to break-even"
                        );

                        // Send execution event
                        if let Some(tx) = &self.execution_tx {
                            let _ = tx.try_send(ExecutionEvent::StopMoved {
                                position_id: pos_id.clone(),
                                new_stop: entry_price,
                            });
                        }
                    }
                }
            }
        }
    }

    fn on_depth(&mut self, depth: &DepthUpdate) {
        let book = self
            .order_books
            .entry(depth.symbol.clone())
            .or_insert_with(|| {
                LocalOrderBook::new(depth.symbol.clone(), self.config.order_book_depth)
            });
        book.update(depth);
    }

    /// Check for liquidations based on current price
    fn check_liquidations(&mut self, symbol: &str, mark_price: Decimal) -> Vec<crate::types::Position> {
        self.position_manager.check_liquidations(symbol, mark_price, self.fee_rate)
    }

    /// Check multi-stage exits: TP1 (50% at VWAP), TP2 (100% at VAH), Soft Stop (10s timeout)
    fn check_multi_stage_exits(
        &mut self,
        symbol: &str,
        current_price: Decimal,
        current_time: chrono::DateTime<chrono::Utc>,
    ) {
        use crate::types::SetupType;

        let open_positions: Vec<_> = self
            .position_manager
            .open_positions_for(symbol)
            .into_iter()
            .map(|p| (p.id.clone(), p.side, p.setup, p.entry_price, p.entry_time, p.tp1_filled, p.tp1_price, p.tp2_price, p.quantity))
            .collect();

        for (pos_id, side, setup, entry_price, entry_time, tp1_filled, tp1_price, tp2_price, quantity) in open_positions {
            // Only apply to AdvancedOrderFlow strategy
            if setup != SetupType::AdvancedOrderFlow {
                continue;
            }

            // TP1: VWAP reached, close 50%
            if !tp1_filled {
                if let Some(tp1) = tp1_price {
                    let tp1_reached = match side {
                        crate::types::Side::Buy => current_price >= tp1,
                        crate::types::Side::Sell => current_price <= tp1,
                    };

                    if tp1_reached {
                        let half_qty = quantity / Decimal::TWO;
                        if let Some(partial_pnl) = self.position_manager.close_partial(
                            &pos_id,
                            half_qty,
                            tp1,
                            self.fee_rate,
                        ) {
                            info!(
                                position_id = %pos_id,
                                tp1_price = %tp1,
                                partial_pnl = %partial_pnl,
                                "TP1 hit: 50% closed at VWAP"
                            );

                            // Mark TP1 as filled and move stop to break-even
                            if self.position_manager.mark_tp1_filled(&pos_id, entry_price) {
                                info!(
                                    position_id = %pos_id,
                                    "Stop moved to break-even after TP1"
                                );

                                // Send TP1 execution event
                                if let Some(tx) = &self.execution_tx {
                                    let _ = tx.try_send(ExecutionEvent::TP1Filled {
                                        position_id: pos_id.clone(),
                                        tp1_price: tp1,
                                        partial_pnl,
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // TP2: VAH reached (or reverse flow), close 100%
            if tp1_filled {
                if let Some(tp2) = tp2_price {
                    let tp2_reached = match side {
                        crate::types::Side::Buy => current_price >= tp2,
                        crate::types::Side::Sell => current_price <= tp2,
                    };

                    if tp2_reached {
                        if let Some(pos) = self.position_manager.close_position(&pos_id, tp2, self.fee_rate) {
                            self.risk_manager.close_position(&pos);
                            self.trade_logger.log_trade(&pos);

                            info!(
                                position_id = %pos_id,
                                tp2_price = %tp2,
                                total_pnl = %pos.pnl,
                                "TP2 hit: 100% closed at VAH"
                            );

                            // Send TP2 execution event
                            if let Some(tx) = &self.execution_tx {
                                let _ = tx.try_send(ExecutionEvent::PositionClosed(pos));
                            }
                        }
                    }
                }
            }

            // Soft Stop: If 10 seconds passed and price hasn't moved in our favor, exit
            let elapsed_secs = (current_time - entry_time).num_seconds();
            if elapsed_secs >= 10 && !tp1_filled {
                let no_progress = match side {
                    crate::types::Side::Buy => current_price <= entry_price,
                    crate::types::Side::Sell => current_price >= entry_price,
                };

                if no_progress {
                    if let Some(pos) = self.position_manager.close_position(&pos_id, current_price, self.fee_rate) {
                        self.risk_manager.close_position(&pos);
                        self.trade_logger.log_trade(&pos);

                        warn!(
                            position_id = %pos_id,
                            elapsed_secs = %elapsed_secs,
                            pnl = %pos.pnl,
                            "Soft Stop triggered: No progress after 10s"
                        );

                        // Send execution event
                        if let Some(tx) = &self.execution_tx {
                            let _ = tx.try_send(ExecutionEvent::PositionClosed(pos));
                        }
                    }
                }
            }
        }
    }

    fn shutdown_summary(&mut self) {
        let closed: Vec<_> = self
            .position_manager
            .closed_positions()
            .into_iter()
            .cloned()
            .collect();
        self.trade_logger.print_summary(&closed);

        info!(
            balance = %self.risk_manager.balance(),
            daily_pnl = %self.risk_manager.daily_pnl(),
            total_trades = closed.len(),
            "Final summary"
        );
    }
}
