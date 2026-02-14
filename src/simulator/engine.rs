use crate::binance::ExchangeInfoManager;
use crate::config::SimulatorConfig;
use crate::risk::RiskManager;
use crate::simulator::order_book::LocalOrderBook;
use crate::simulator::position::PositionManager;
use crate::simulator::trade_log::TradeLogger;
use crate::types::{
    BotStats, DepthUpdate, ExecutionEvent, ExitReason, MarginType, MarketEvent, NormalizedTrade,
    ProcessingEvent, SymbolStats, TradeSignal,
};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::types::VolumeProfileSnapshot;
use chrono::Timelike;

#[derive(Default, Clone)]
struct HourlyPerformance {
    pnls: Vec<Decimal>,
}

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
    require_orderbook_for_entry: bool,
    max_spread_bps: Decimal,
    min_depth_imbalance_ratio: Decimal,
    expectancy_filter_enabled: bool,
    expectancy_min_trades_per_hour: usize,
    expectancy_min_avg_pnl: Decimal,
    expectancy_lookback_trades: usize,
    slippage_model_enabled: bool,
    max_model_slippage_bps: Decimal,
    impact_depth_levels: usize,
    impact_weight_bps: Decimal,
    hourly_performance: BTreeMap<(String, u32), HourlyPerformance>,
    /// Per-symbol trading statistics
    symbol_stats: BTreeMap<String, SymbolStats>,
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
        let max_spread_bps = Decimal::try_from(config.max_spread_bps).unwrap_or(Decimal::new(4, 0));
        let min_depth_imbalance_ratio =
            Decimal::try_from(config.min_depth_imbalance_ratio).unwrap_or(Decimal::new(105, 2));
        let require_orderbook_for_entry = config.require_orderbook_for_entry;
        let expectancy_filter_enabled = config.expectancy_filter_enabled;
        let expectancy_min_trades_per_hour = config.expectancy_min_trades_per_hour;
        let expectancy_min_avg_pnl =
            Decimal::try_from(config.expectancy_min_avg_pnl).unwrap_or(Decimal::ZERO);
        let expectancy_lookback_trades = config.expectancy_lookback_trades;
        let slippage_model_enabled = config.slippage_model_enabled;
        let max_model_slippage_bps =
            Decimal::try_from(config.max_model_slippage_bps).unwrap_or(Decimal::new(6, 0));
        let impact_depth_levels = config.impact_depth_levels;
        let impact_weight_bps =
            Decimal::try_from(config.impact_weight_bps).unwrap_or(Decimal::new(8, 0));
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
            require_orderbook_for_entry,
            max_spread_bps,
            min_depth_imbalance_ratio,
            expectancy_filter_enabled,
            expectancy_min_trades_per_hour,
            expectancy_min_avg_pnl,
            expectancy_lookback_trades,
            slippage_model_enabled,
            max_model_slippage_bps,
            impact_depth_levels,
            impact_weight_bps,
            hourly_performance: BTreeMap::new(),
            symbol_stats: BTreeMap::new(),
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
                s.total_trades = self.symbol_stats.values().map(|ss| ss.total_trades).sum();

                // Clone symbol stats and set open position counts from live data
                let mut ss = self.symbol_stats.clone();
                // Reset open_positions (they are set fresh each sync)
                for v in ss.values_mut() {
                    v.open_positions = 0;
                }
                for pos in self.position_manager.open_positions() {
                    ss.entry(pos.symbol.clone()).or_default().open_positions += 1;
                }
                s.symbol_stats = ss;
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
        if !self.passes_execution_quality_filters(&signal) {
            return;
        }
        if !self.passes_expectancy_filter(&signal) {
            return;
        }

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
        let (validated_entry, validated_quantity) =
            if let Some(ref exchange_info) = self.exchange_info {
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
        if !self.passes_slippage_model(
            &validated_signal.symbol,
            validated_signal.side,
            validated_entry,
            validated_quantity,
        ) {
            return;
        }

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
        self.trade_logger.log_entry(&position);

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

    fn passes_execution_quality_filters(&self, signal: &TradeSignal) -> bool {
        let book = match self.order_books.get(&signal.symbol) {
            Some(b) => b,
            None if self.require_orderbook_for_entry => {
                warn!(
                    symbol = %signal.symbol,
                    "Signal rejected: no order book snapshot available"
                );
                return false;
            }
            None => return true,
        };

        let Some(spread) = book.spread() else {
            if self.require_orderbook_for_entry {
                warn!(symbol = %signal.symbol, "Signal rejected: missing spread data");
                return false;
            }
            return true;
        };
        let Some(mid) = book.mid_price() else {
            if self.require_orderbook_for_entry {
                warn!(symbol = %signal.symbol, "Signal rejected: missing mid price");
                return false;
            }
            return true;
        };
        if mid <= Decimal::ZERO {
            return false;
        }

        let spread_bps = (spread / mid) * Decimal::from(10_000);
        if spread_bps > self.max_spread_bps {
            warn!(
                symbol = %signal.symbol,
                spread_bps = %spread_bps,
                max_spread_bps = %self.max_spread_bps,
                "Signal rejected: spread too wide"
            );
            return false;
        }

        let (bid_vol, ask_vol, ratio) = book.depth_imbalance();
        let side_ok = match signal.side {
            crate::types::Side::Buy => ratio >= self.min_depth_imbalance_ratio,
            crate::types::Side::Sell => {
                if bid_vol <= Decimal::ZERO {
                    false
                } else {
                    (ask_vol / bid_vol) >= self.min_depth_imbalance_ratio
                }
            }
        };
        if !side_ok {
            warn!(
                symbol = %signal.symbol,
                side = ?signal.side,
                bid_vol = %bid_vol,
                ask_vol = %ask_vol,
                ratio = %ratio,
                min_depth_imbalance_ratio = %self.min_depth_imbalance_ratio,
                "Signal rejected: insufficient depth imbalance"
            );
            return false;
        }

        true
    }

    fn passes_expectancy_filter(&self, signal: &TradeSignal) -> bool {
        if !self.expectancy_filter_enabled {
            return true;
        }

        let hour = signal.timestamp.hour();
        let key = (signal.symbol.clone(), hour);
        let Some(stats) = self.hourly_performance.get(&key) else {
            return true;
        };
        if stats.pnls.len() < self.expectancy_min_trades_per_hour {
            return true;
        }

        let avg =
            stats.pnls.iter().copied().sum::<Decimal>() / Decimal::from(stats.pnls.len() as u64);
        if avg < self.expectancy_min_avg_pnl {
            warn!(
                symbol = %signal.symbol,
                utc_hour = hour,
                avg_pnl = %avg,
                min_avg_pnl = %self.expectancy_min_avg_pnl,
                samples = stats.pnls.len(),
                "Signal rejected: UTC-hour expectancy below threshold"
            );
            return false;
        }
        true
    }

    fn passes_slippage_model(
        &self,
        symbol: &str,
        side: crate::types::Side,
        entry_price: Decimal,
        quantity: Decimal,
    ) -> bool {
        if !self.slippage_model_enabled {
            return true;
        }
        let Some(book) = self.order_books.get(symbol) else {
            return true;
        };
        if entry_price <= Decimal::ZERO || quantity <= Decimal::ZERO {
            return false;
        }

        let mid = match book.mid_price() {
            Some(v) if v > Decimal::ZERO => v,
            _ => return true,
        };
        let spread = book.spread().unwrap_or(Decimal::ZERO);
        let half_spread_bps = (spread / mid) * Decimal::from(5_000);
        let top_depth = match side {
            crate::types::Side::Buy => book.top_ask_depth(self.impact_depth_levels),
            crate::types::Side::Sell => book.top_bid_depth(self.impact_depth_levels),
        };
        if top_depth <= Decimal::ZERO {
            return true;
        }
        let impact_ratio = quantity / top_depth;
        let impact_bps = impact_ratio * self.impact_weight_bps;
        let total_slippage_bps = half_spread_bps + impact_bps;

        if total_slippage_bps > self.max_model_slippage_bps {
            warn!(
                symbol = %symbol,
                side = ?side,
                quantity = %quantity,
                top_depth = %top_depth,
                estimated_slippage_bps = %total_slippage_bps,
                max_model_slippage_bps = %self.max_model_slippage_bps,
                "Signal rejected: estimated slippage too high"
            );
            return false;
        }
        true
    }

    fn record_hourly_expectancy(&mut self, position: &crate::types::Position) {
        let hour = position.entry_time.hour();
        let key = (position.symbol.clone(), hour);
        let stats = self.hourly_performance.entry(key).or_default();
        stats.pnls.push(position.pnl);
        if stats.pnls.len() > self.expectancy_lookback_trades {
            let keep = self.expectancy_lookback_trades;
            stats.pnls.drain(..stats.pnls.len() - keep);
        }
    }

    fn on_trade(&mut self, trade: &NormalizedTrade) {
        // Keep shared stats up to date for the hourly reporter task
        self.sync_bot_stats();
        // Update per-position MFE/MAE before checking exits
        self.position_manager
            .update_excursions(&trade.symbol, trade.price, trade.timestamp);

        // First, check for liquidations (highest priority)
        let liquidated = self.check_liquidations(&trade.symbol, trade.price);
        for position in &liquidated {
            self.risk_manager.close_position(position);
            self.trade_logger.log_trade(position);
            self.record_hourly_expectancy(position);
            self.symbol_stats
                .entry(position.symbol.clone())
                .or_default()
                .record_close(position.pnl);

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
        let closed = self
            .position_manager
            .check_exits(&trade.symbol, trade.price, self.fee_rate);

        for position in &closed {
            self.risk_manager.close_position(position);
            self.trade_logger.log_trade(position);
            self.record_hourly_expectancy(position);
            self.symbol_stats
                .entry(position.symbol.clone())
                .or_default()
                .record_close(position.pnl);

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
                    .should_move_to_break_even(pos, trade.price, trade.timestamp)
                {
                    let new_stop = self.risk_manager.break_even_stop_price(pos);
                    if self
                        .position_manager
                        .move_stop_to_break_even(&pos_id, new_stop)
                    {
                        info!(
                            position_id = %pos_id,
                            new_stop = %new_stop,
                            "Stop moved to protected break-even"
                        );

                        // Send execution event
                        if let Some(tx) = &self.execution_tx {
                            let _ = tx.try_send(ExecutionEvent::StopMoved {
                                position_id: pos_id.clone(),
                                new_stop,
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
    fn check_liquidations(
        &mut self,
        symbol: &str,
        mark_price: Decimal,
    ) -> Vec<crate::types::Position> {
        self.position_manager
            .check_liquidations(symbol, mark_price, self.fee_rate)
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
            .map(|p| {
                (
                    p.id.clone(),
                    p.side,
                    p.setup,
                    p.entry_price,
                    p.entry_time,
                    p.tp1_filled,
                    p.tp1_price,
                    p.tp2_price,
                    p.quantity,
                )
            })
            .collect();

        for (
            pos_id,
            side,
            setup,
            entry_price,
            entry_time,
            tp1_filled,
            tp1_price,
            tp2_price,
            quantity,
        ) in open_positions
        {
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

                            // Mark TP1 as filled and move stop to protected break-even
                            let be_stop = self
                                .position_manager
                                .open_positions()
                                .into_iter()
                                .find(|p| p.id == pos_id)
                                .map(|p| self.risk_manager.break_even_stop_price(p))
                                .unwrap_or(entry_price);
                            if self.position_manager.mark_tp1_filled(&pos_id, be_stop) {
                                info!(
                                    position_id = %pos_id,
                                    new_stop = %be_stop,
                                    "Stop moved after TP1"
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
                        if let Some(pos) = self.position_manager.close_position(
                            &pos_id,
                            tp2,
                            self.fee_rate,
                            ExitReason::TP2,
                        ) {
                            self.risk_manager.close_position(&pos);
                            self.trade_logger.log_trade(&pos);
                            self.record_hourly_expectancy(&pos);
                            self.symbol_stats
                                .entry(pos.symbol.clone())
                                .or_default()
                                .record_close(pos.pnl);

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

            // Soft Stop: after timeout, cut only if trade is still in meaningful drawdown.
            let elapsed_secs = (current_time - entry_time).num_seconds();
            let soft_stop_secs = self.config.soft_stop_seconds as i64;
            let soft_stop_drawdown = Decimal::try_from(self.config.soft_stop_drawdown_pct)
                .unwrap_or(Decimal::new(15, 2));
            if elapsed_secs >= soft_stop_secs && !tp1_filled {
                let drawdown_level = match side {
                    crate::types::Side::Buy => {
                        entry_price * (Decimal::ONE - (soft_stop_drawdown / Decimal::from(100)))
                    }
                    crate::types::Side::Sell => {
                        entry_price * (Decimal::ONE + (soft_stop_drawdown / Decimal::from(100)))
                    }
                };
                let no_progress = match side {
                    crate::types::Side::Buy => current_price <= drawdown_level,
                    crate::types::Side::Sell => current_price >= drawdown_level,
                };

                if no_progress {
                    if let Some(pos) = self.position_manager.close_position(
                        &pos_id,
                        current_price,
                        self.fee_rate,
                        ExitReason::SoftStop,
                    ) {
                        self.risk_manager.close_position(&pos);
                        self.trade_logger.log_trade(&pos);
                        self.record_hourly_expectancy(&pos);
                        self.symbol_stats
                            .entry(pos.symbol.clone())
                            .or_default()
                            .record_close(pos.pnl);

                        warn!(
                            position_id = %pos_id,
                            elapsed_secs = %elapsed_secs,
                            drawdown_level = %drawdown_level,
                            pnl = %pos.pnl,
                            "Soft Stop triggered: drawdown after timeout"
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
        let finalized: Vec<_> = self
            .position_manager
            .finalized_positions()
            .into_iter()
            .cloned()
            .collect();
        self.trade_logger
            .print_summary(&finalized, self.risk_manager.initial_balance());

        info!(
            balance = %self.risk_manager.balance(),
            daily_pnl = %self.risk_manager.daily_pnl(),
            total_trades = finalized.len(),
            "Final summary"
        );
    }
}
