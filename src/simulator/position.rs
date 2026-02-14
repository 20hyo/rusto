use crate::types::{MarginType, Position, PositionStatus, Side, TradeSignal};
use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

/// Calculate liquidation price for leveraged position
/// Formula (Isolated Margin):
/// - Long: liq_price = entry * (1 - (1/leverage - mmr - fees))
/// - Short: liq_price = entry * (1 + (1/leverage - mmr - fees))
/// where mmr = maintenance margin rate, fees = 2 * taker_fee
pub fn calculate_liquidation_price(
    side: Side,
    entry_price: Decimal,
    leverage: Decimal,
    maintenance_margin_rate: Decimal,
    taker_fee: Decimal,
) -> Decimal {
    let fee_cost = taker_fee * Decimal::from(2); // Entry + exit fees
    let leverage_inv = Decimal::ONE / leverage;

    match side {
        Side::Buy => {
            // Long liquidation: price drops
            let adjustment = leverage_inv - maintenance_margin_rate - fee_cost;
            entry_price * (Decimal::ONE - adjustment)
        }
        Side::Sell => {
            // Short liquidation: price rises
            let adjustment = leverage_inv - maintenance_margin_rate - fee_cost;
            entry_price * (Decimal::ONE + adjustment)
        }
    }
}

/// Calculate initial margin required for position
/// initial_margin = (entry_price * quantity) / leverage
pub fn calculate_initial_margin(
    entry_price: Decimal,
    quantity: Decimal,
    leverage: Decimal,
) -> Decimal {
    (entry_price * quantity) / leverage
}

/// Calculate maintenance margin required for position
/// maintenance_margin = (entry_price * quantity) * maintenance_margin_rate
pub fn calculate_maintenance_margin(
    entry_price: Decimal,
    quantity: Decimal,
    maintenance_margin_rate: Decimal,
) -> Decimal {
    entry_price * quantity * maintenance_margin_rate
}

/// Manages simulated position lifecycle
pub struct PositionManager {
    positions: Vec<Position>,
}

impl PositionManager {
    pub fn new() -> Self {
        Self {
            positions: Vec::new(),
        }
    }

    /// Open a new position from a trade signal with leverage
    pub fn open_position(
        &mut self,
        signal: &TradeSignal,
        quantity: Decimal,
        leverage: Decimal,
        margin_type: MarginType,
        maintenance_margin_rate: Decimal,
        taker_fee: Decimal,
    ) -> Position {
        let liquidation_price = calculate_liquidation_price(
            signal.side,
            signal.entry_price,
            leverage,
            maintenance_margin_rate,
            taker_fee,
        );

        let initial_margin = calculate_initial_margin(
            signal.entry_price,
            quantity,
            leverage,
        );

        let maintenance_margin = calculate_maintenance_margin(
            signal.entry_price,
            quantity,
            maintenance_margin_rate,
        );

        let position = Position {
            id: Uuid::new_v4().to_string(),
            symbol: signal.symbol.clone(),
            side: signal.side,
            entry_price: signal.entry_price,
            quantity,
            stop_loss: signal.stop_loss,
            take_profit: signal.take_profit,
            setup: signal.setup,
            status: PositionStatus::Open,
            pnl: Decimal::ZERO,
            entry_time: Utc::now(),
            exit_time: None,
            exit_price: None,
            break_even_moved: false,
            leverage,
            margin_type,
            liquidation_price,
            unrealized_pnl: Decimal::ZERO,
            initial_margin,
            maintenance_margin,
            tp1_filled: false,
            tp1_price: None,
            tp2_price: None,
            original_quantity: quantity,
        };
        self.positions.push(position.clone());
        position
    }

    /// Close a partial position (e.g., 50% at TP1)
    /// Returns the realized PnL for the partial close
    pub fn close_partial(
        &mut self,
        position_id: &str,
        close_quantity: Decimal,
        exit_price: Decimal,
        fee_rate: Decimal,
    ) -> Option<Decimal> {
        let pos = self
            .positions
            .iter_mut()
            .find(|p| p.id == position_id && p.status == PositionStatus::Open)?;

        if close_quantity >= pos.quantity {
            // Closing full position
            return None;
        }

        // Calculate PnL for closed portion
        let raw_pnl = match pos.side {
            Side::Buy => (exit_price - pos.entry_price) * close_quantity,
            Side::Sell => (pos.entry_price - exit_price) * close_quantity,
        };

        // Subtract fees for closed portion
        let notional = pos.entry_price * close_quantity + exit_price * close_quantity;
        let fees = notional * fee_rate;
        let partial_pnl = raw_pnl - fees;

        // Update position: reduce quantity, accumulate PnL
        pos.quantity -= close_quantity;
        pos.pnl += partial_pnl;

        Some(partial_pnl)
    }

    /// Close a position at a given price
    pub fn close_position(
        &mut self,
        position_id: &str,
        exit_price: Decimal,
        fee_rate: Decimal,
    ) -> Option<Position> {
        let pos = self
            .positions
            .iter_mut()
            .find(|p| p.id == position_id && p.status == PositionStatus::Open)?;

        let raw_pnl = match pos.side {
            Side::Buy => (exit_price - pos.entry_price) * pos.quantity,
            Side::Sell => (pos.entry_price - exit_price) * pos.quantity,
        };

        // Subtract fees (entry + exit)
        let notional = pos.entry_price * pos.quantity + exit_price * pos.quantity;
        let fees = notional * fee_rate;
        let net_pnl = raw_pnl - fees;

        pos.pnl += net_pnl; // Add to any existing partial PnL
        pos.exit_price = Some(exit_price);
        pos.exit_time = Some(Utc::now());
        pos.status = PositionStatus::Closed;

        Some(pos.clone())
    }

    /// Move stop to break-even for a position
    pub fn move_stop_to_break_even(&mut self, position_id: &str, stop_price: Decimal) -> bool {
        if let Some(pos) = self
            .positions
            .iter_mut()
            .find(|p| p.id == position_id && p.status == PositionStatus::Open)
        {
            pos.stop_loss = stop_price;
            pos.break_even_moved = true;
            true
        } else {
            false
        }
    }

    /// Mark TP1 as filled and move stop to break-even
    pub fn mark_tp1_filled(&mut self, position_id: &str, stop_price: Decimal) -> bool {
        if let Some(pos) = self
            .positions
            .iter_mut()
            .find(|p| p.id == position_id && p.status == PositionStatus::Open)
        {
            pos.tp1_filled = true;
            pos.stop_loss = stop_price;
            pos.break_even_moved = true;
            true
        } else {
            false
        }
    }

    /// Get all open positions
    pub fn open_positions(&self) -> Vec<&Position> {
        self.positions
            .iter()
            .filter(|p| p.status == PositionStatus::Open)
            .collect()
    }

    /// Get open positions for a specific symbol
    pub fn open_positions_for(&self, symbol: &str) -> Vec<&Position> {
        self.positions
            .iter()
            .filter(|p| p.status == PositionStatus::Open && p.symbol == symbol)
            .collect()
    }

    /// Get all closed positions
    pub fn closed_positions(&self) -> Vec<&Position> {
        self.positions
            .iter()
            .filter(|p| p.status == PositionStatus::Closed)
            .collect()
    }

    /// Get all finalized positions (closed + liquidated)
    pub fn finalized_positions(&self) -> Vec<&Position> {
        self.positions
            .iter()
            .filter(|p| p.status == PositionStatus::Closed || p.status == PositionStatus::Liquidated)
            .collect()
    }

    /// Check if any position should be liquidated based on liquidation price
    pub fn check_liquidations(&mut self, symbol: &str, mark_price: Decimal, fee_rate: Decimal) -> Vec<Position> {
        let ids_to_liquidate: Vec<String> = self
            .positions
            .iter()
            .filter(|p| p.status == PositionStatus::Open && p.symbol == symbol)
            .filter(|p| p.should_liquidate(mark_price))
            .map(|p| p.id.clone())
            .collect();

        let mut liquidated = Vec::new();
        for id in ids_to_liquidate {
            if let Some(pos) = self
                .positions
                .iter_mut()
                .find(|p| p.id == id && p.status == PositionStatus::Open)
            {
                // Liquidate at liquidation price with full loss
                let liquidation_price = pos.liquidation_price;
                let raw_pnl = match pos.side {
                    Side::Buy => (liquidation_price - pos.entry_price) * pos.quantity,
                    Side::Sell => (pos.entry_price - liquidation_price) * pos.quantity,
                };

                // Subtract fees (entry + exit)
                let notional = pos.entry_price * pos.quantity + liquidation_price * pos.quantity;
                let fees = notional * fee_rate;
                let net_pnl = raw_pnl - fees;

                pos.pnl = net_pnl;
                pos.exit_price = Some(liquidation_price);
                pos.exit_time = Some(Utc::now());
                pos.status = PositionStatus::Liquidated;

                liquidated.push(pos.clone());
            }
        }
        liquidated
    }

    /// Check if any position should be stopped out or take profit hit
    pub fn check_exits(&mut self, symbol: &str, current_price: Decimal, fee_rate: Decimal) -> Vec<Position> {
        let ids_to_close: Vec<(String, Decimal)> = self
            .positions
            .iter()
            .filter(|p| p.status == PositionStatus::Open && p.symbol == symbol)
            .filter_map(|p| {
                match p.side {
                    Side::Buy => {
                        if current_price <= p.stop_loss {
                            Some((p.id.clone(), p.stop_loss))
                        } else if current_price >= p.take_profit {
                            Some((p.id.clone(), p.take_profit))
                        } else {
                            None
                        }
                    }
                    Side::Sell => {
                        if current_price >= p.stop_loss {
                            Some((p.id.clone(), p.stop_loss))
                        } else if current_price <= p.take_profit {
                            Some((p.id.clone(), p.take_profit))
                        } else {
                            None
                        }
                    }
                }
            })
            .collect();

        let mut closed = Vec::new();
        for (id, price) in ids_to_close {
            if let Some(pos) = self.close_position(&id, price, fee_rate) {
                closed.push(pos);
            }
        }
        closed
    }
}
