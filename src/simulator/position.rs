use crate::types::{Position, PositionStatus, Side, TradeSignal};
use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

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

    /// Open a new position from a trade signal
    pub fn open_position(
        &mut self,
        signal: &TradeSignal,
        quantity: Decimal,
    ) -> Position {
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
        };
        self.positions.push(position.clone());
        position
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

        pos.pnl = net_pnl;
        pos.exit_price = Some(exit_price);
        pos.exit_time = Some(Utc::now());
        pos.status = PositionStatus::Closed;

        Some(pos.clone())
    }

    /// Move stop to break-even for a position
    pub fn move_stop_to_break_even(&mut self, position_id: &str) -> bool {
        if let Some(pos) = self
            .positions
            .iter_mut()
            .find(|p| p.id == position_id && p.status == PositionStatus::Open)
        {
            pos.stop_loss = pos.entry_price;
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
