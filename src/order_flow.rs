use crate::config::OrderFlowConfig;
use crate::types::{OrderFlowMetrics, RangeBar, Side};
use chrono::Utc;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use tracing::info;

/// Tracks order flow metrics: CVD, delta, absorption detection
pub struct OrderFlowTracker {
    absorption_delta_ratio: Decimal,
    max_price_delta_ticks: Decimal,
    large_volume_multiplier: Decimal,
    /// Per-symbol cumulative volume delta
    cvd: BTreeMap<String, Decimal>,
    /// Recent bar deltas for average calculation
    recent_deltas: BTreeMap<String, Vec<Decimal>>,
}

impl OrderFlowTracker {
    pub fn new(config: &OrderFlowConfig) -> Self {
        Self {
            absorption_delta_ratio: Decimal::try_from(config.absorption_delta_ratio)
                .unwrap_or(Decimal::from(3)),
            max_price_delta_ticks: Decimal::from(config.max_price_delta_ticks),
            large_volume_multiplier: Decimal::try_from(config.large_volume_multiplier)
                .unwrap_or(Decimal::TWO),
            cvd: BTreeMap::new(),
            recent_deltas: BTreeMap::new(),
        }
    }

    /// Analyze a completed range bar for order flow signals
    pub fn analyze_bar(&mut self, bar: &RangeBar) -> OrderFlowMetrics {
        let bar_delta = bar.delta();

        // Update CVD
        let cvd = self.cvd.entry(bar.symbol.clone()).or_insert(Decimal::ZERO);
        *cvd += bar_delta;
        let current_cvd = *cvd;

        // Track recent deltas
        let deltas = self
            .recent_deltas
            .entry(bar.symbol.clone())
            .or_insert_with(Vec::new);
        deltas.push(bar_delta);
        if deltas.len() > 50 {
            deltas.remove(0);
        }

        // Absorption detection from footprint
        let (absorption_detected, absorption_side) = self.detect_absorption(bar);

        // Imbalance ratio
        let imbalance_ratio = if bar.sell_volume > Decimal::ZERO {
            bar.buy_volume / bar.sell_volume
        } else if bar.buy_volume > Decimal::ZERO {
            Decimal::from(999)
        } else {
            Decimal::ONE
        };

        if absorption_detected {
            info!(
                symbol = %bar.symbol,
                absorption_side = ?absorption_side,
                bar_delta = %bar_delta,
                cvd = %current_cvd,
                "Absorption detected"
            );
        }

        OrderFlowMetrics {
            symbol: bar.symbol.clone(),
            cvd: current_cvd,
            bar_delta,
            absorption_detected,
            absorption_side,
            imbalance_ratio,
            timestamp: Utc::now(),
        }
    }

    /// Detect absorption from footprint data.
    /// Sell absorption: high bid_volume (aggressive sellers) but price didn't fall
    /// Buy absorption: high ask_volume (aggressive buyers) but price didn't rise
    fn detect_absorption(&self, bar: &RangeBar) -> (bool, Option<Side>) {
        // Price movement relative to range
        let price_delta = bar.close - bar.open;
        let price_delta_abs = price_delta.abs();

        // Check each footprint level for absorption patterns
        for level in bar.footprint.values() {
            let total = level.bid_volume + level.ask_volume;
            if total == Decimal::ZERO {
                continue;
            }

            // Sell absorption: high bid_volume (sellers hitting bids) but price stable/up
            if level.bid_volume > Decimal::ZERO && level.ask_volume > Decimal::ZERO {
                let bid_ratio = level.bid_volume / level.ask_volume;
                if bid_ratio > self.absorption_delta_ratio
                    && price_delta_abs <= self.max_price_delta_ticks
                {
                    // Large selling absorbed — price didn't drop
                    return (true, Some(Side::Sell));
                }

                let ask_ratio = level.ask_volume / level.bid_volume;
                if ask_ratio > self.absorption_delta_ratio
                    && price_delta_abs <= self.max_price_delta_ticks
                {
                    // Large buying absorbed — price didn't rise
                    return (true, Some(Side::Buy));
                }
            }
        }

        (false, None)
    }

    /// Check if current bar volume is large relative to recent average
    pub fn is_large_volume(&self, bar: &RangeBar) -> bool {
        let deltas = match self.recent_deltas.get(&bar.symbol) {
            Some(d) if d.len() >= 5 => d,
            _ => return false,
        };

        let avg_volume: Decimal = deltas.iter().map(|d| d.abs()).sum::<Decimal>()
            / Decimal::from(deltas.len() as u64);

        bar.volume > avg_volume * self.large_volume_multiplier
    }
}
