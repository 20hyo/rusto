use crate::types::DepthUpdate;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// Local order book maintained from depth stream updates
pub struct LocalOrderBook {
    pub symbol: String,
    /// Bids: price -> quantity (descending price order)
    pub bids: BTreeMap<Decimal, Decimal>,
    /// Asks: price -> quantity (ascending price order)
    pub asks: BTreeMap<Decimal, Decimal>,
    max_depth: usize,
}

impl LocalOrderBook {
    pub fn new(symbol: String, max_depth: usize) -> Self {
        Self {
            symbol,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            max_depth,
        }
    }

    /// Apply a depth update
    pub fn update(&mut self, depth: &DepthUpdate) {
        for level in &depth.bids {
            if level.quantity == Decimal::ZERO {
                self.bids.remove(&level.price);
            } else {
                self.bids.insert(level.price, level.quantity);
            }
        }

        for level in &depth.asks {
            if level.quantity == Decimal::ZERO {
                self.asks.remove(&level.price);
            } else {
                self.asks.insert(level.price, level.quantity);
            }
        }

        // Trim to max depth
        while self.bids.len() > self.max_depth {
            if let Some(&lowest_bid) = self.bids.keys().next() {
                self.bids.remove(&lowest_bid);
            }
        }
        while self.asks.len() > self.max_depth {
            if let Some(&highest_ask) = self.asks.keys().last() {
                self.asks.remove(&highest_ask);
            }
        }
    }

    /// Best bid price
    pub fn best_bid(&self) -> Option<Decimal> {
        self.bids.keys().last().copied()
    }

    /// Best ask price
    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.keys().next().copied()
    }

    /// Mid price
    pub fn mid_price(&self) -> Option<Decimal> {
        let bid = self.best_bid()?;
        let ask = self.best_ask()?;
        Some((bid + ask) / Decimal::TWO)
    }

    /// Spread
    pub fn spread(&self) -> Option<Decimal> {
        let bid = self.best_bid()?;
        let ask = self.best_ask()?;
        Some(ask - bid)
    }
}
