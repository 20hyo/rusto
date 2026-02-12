use crate::config::RiskConfig;
use crate::types::{Position, Side, TradeSignal};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use tracing::{info, warn};

/// Manages risk: position sizing, break-even stops, daily limits
pub struct RiskManager {
    config: RiskConfig,
    balance: Decimal,
    daily_pnl: Decimal,
    daily_limit: Decimal,
    max_concurrent: usize,
    break_even_ticks: Decimal,
    /// Currently open positions per symbol
    open_positions: BTreeMap<String, Vec<String>>, // symbol -> position_ids
    daily_halted: bool,
    leverage: Decimal,
}

impl RiskManager {
    pub fn new(config: &RiskConfig, leverage: Decimal) -> Self {
        let balance = Decimal::try_from(config.initial_balance).unwrap_or(Decimal::from(10000));
        let daily_limit = balance
            * Decimal::try_from(config.daily_loss_limit_pct).unwrap_or_else(|_| Decimal::new(3, 2));

        Self {
            config: config.clone(),
            balance,
            daily_pnl: Decimal::ZERO,
            daily_limit,
            max_concurrent: config.max_concurrent_positions,
            break_even_ticks: Decimal::from(config.break_even_ticks),
            open_positions: BTreeMap::new(),
            daily_halted: false,
            leverage,
        }
    }

    /// Check if a new trade is allowed
    pub fn can_trade(&self, signal: &TradeSignal) -> bool {
        if self.daily_halted {
            warn!("Trading halted: daily loss limit reached");
            return false;
        }

        // Max concurrent positions
        let total_open: usize = self.open_positions.values().map(|v| v.len()).sum();
        if total_open >= self.max_concurrent {
            warn!(
                "Max concurrent positions reached: {}/{}",
                total_open, self.max_concurrent
            );
            return false;
        }

        // Max one position per symbol
        if let Some(positions) = self.open_positions.get(&signal.symbol) {
            if !positions.is_empty() {
                warn!(
                    "Already have position for symbol: {}",
                    signal.symbol
                );
                return false;
            }
        }

        true
    }

    /// Calculate position size based on risk and leverage
    /// For leveraged trading:
    /// - risk_amount = balance * max_risk_per_trade (what we're willing to lose)
    /// - stop_distance = abs(entry - stop)
    /// - quantity = risk_amount / stop_distance
    /// - required_margin = (entry_price * quantity) / leverage
    pub fn calculate_position_size(&self, signal: &TradeSignal) -> Decimal {
        let stop_distance = (signal.entry_price - signal.stop_loss).abs();
        if stop_distance == Decimal::ZERO {
            return Decimal::ZERO;
        }

        let risk_amount = self.balance
            * Decimal::try_from(self.config.max_risk_per_trade)
                .unwrap_or_else(|_| Decimal::new(1, 2));

        // Calculate quantity based on risk per trade
        let quantity = risk_amount / stop_distance;

        // Calculate required margin for this position
        let required_margin = (signal.entry_price * quantity) / self.leverage;

        // Ensure we have enough balance for the margin
        if required_margin > self.balance {
            warn!(
                symbol = %signal.symbol,
                required_margin = %required_margin,
                balance = %self.balance,
                "Insufficient balance for position, reducing size"
            );
            // Reduce quantity to fit available balance
            let adjusted_quantity = (self.balance * self.leverage) / signal.entry_price;
            info!(
                symbol = %signal.symbol,
                risk_amount = %risk_amount,
                stop_distance = %stop_distance,
                quantity = %adjusted_quantity,
                required_margin = %self.balance,
                leverage = %self.leverage,
                "Position size calculated (adjusted)"
            );
            return adjusted_quantity;
        }

        info!(
            symbol = %signal.symbol,
            risk_amount = %risk_amount,
            stop_distance = %stop_distance,
            quantity = %quantity,
            required_margin = %required_margin,
            leverage = %self.leverage,
            "Position size calculated"
        );

        quantity
    }

    /// Register a new open position
    pub fn register_position(&mut self, position: &Position) {
        self.open_positions
            .entry(position.symbol.clone())
            .or_insert_with(Vec::new)
            .push(position.id.clone());
    }

    /// Close a position and update PnL
    pub fn close_position(&mut self, position: &Position) {
        if let Some(positions) = self.open_positions.get_mut(&position.symbol) {
            positions.retain(|id| id != &position.id);
        }

        self.daily_pnl += position.pnl;
        self.balance += position.pnl;

        info!(
            position_id = %position.id,
            pnl = %position.pnl,
            daily_pnl = %self.daily_pnl,
            balance = %self.balance,
            "Position closed"
        );

        // Check daily loss limit
        if self.daily_pnl < -self.daily_limit {
            warn!(
                daily_pnl = %self.daily_pnl,
                limit = %self.daily_limit,
                "Daily loss limit reached! Halting trading."
            );
            self.daily_halted = true;
        }
    }

    /// Check if stop should be moved to break-even
    /// Condition: price has moved `break_even_ticks` in favor
    pub fn should_move_to_break_even(
        &self,
        position: &Position,
        current_price: Decimal,
    ) -> bool {
        if position.break_even_moved {
            return false;
        }

        let favorable_move = match position.side {
            Side::Buy => current_price - position.entry_price,
            Side::Sell => position.entry_price - current_price,
        };

        favorable_move >= self.break_even_ticks
    }

    /// Reset daily stats (call at session start)
    pub fn reset_daily(&mut self) {
        self.daily_pnl = Decimal::ZERO;
        self.daily_halted = false;
        info!("Daily risk stats reset");
    }

    pub fn is_halted(&self) -> bool {
        self.daily_halted
    }

    pub fn balance(&self) -> Decimal {
        self.balance
    }

    pub fn daily_pnl(&self) -> Decimal {
        self.daily_pnl
    }
}
