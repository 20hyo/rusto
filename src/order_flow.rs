use crate::config::OrderFlowConfig;
use crate::types::{OrderFlowMetrics, RangeBar, Side};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use tracing::info;

/// Tracks order flow metrics: CVD, delta, absorption detection
pub struct OrderFlowTracker {
    absorption_delta_ratio: Decimal,
    max_price_delta_ticks: Decimal,
    large_volume_multiplier: Decimal,
    volume_baseline_bars: usize,
    volume_burst_multiplier: Decimal,
    /// Per-symbol cumulative volume delta
    cvd: BTreeMap<String, Decimal>,
    /// Recent bar deltas for average calculation
    recent_deltas: BTreeMap<String, Vec<Decimal>>,
    /// Recent bar volumes for per-symbol burst detection
    recent_volumes: BTreeMap<String, Vec<Decimal>>,
    /// CVD history for 1-minute tracking (timestamp, cvd_value)
    cvd_history: BTreeMap<String, Vec<(DateTime<Utc>, Decimal)>>,
}

impl OrderFlowTracker {
    pub fn new(config: &OrderFlowConfig) -> Self {
        Self {
            absorption_delta_ratio: Decimal::try_from(config.absorption_delta_ratio)
                .unwrap_or(Decimal::from(3)),
            max_price_delta_ticks: Decimal::from(config.max_price_delta_ticks),
            large_volume_multiplier: Decimal::try_from(config.large_volume_multiplier)
                .unwrap_or(Decimal::TWO),
            volume_baseline_bars: config.volume_baseline_bars.max(5),
            volume_burst_multiplier: Decimal::try_from(config.volume_burst_multiplier)
                .unwrap_or(Decimal::new(18, 1)),
            cvd: BTreeMap::new(),
            recent_deltas: BTreeMap::new(),
            recent_volumes: BTreeMap::new(),
            cvd_history: BTreeMap::new(),
        }
    }

    /// Get CVD change over last 1 minute
    /// Returns (cvd_change, is_rapid_drop, is_rapid_rise)
    /// - rapid_drop: CVD fell quickly (sell-side explosion)
    /// - rapid_rise: CVD rose quickly (buy-side explosion)
    pub fn get_cvd_1min_change(&self, symbol: &str, now: DateTime<Utc>) -> (Decimal, bool, bool) {
        let history = match self.cvd_history.get(symbol) {
            Some(h) if !h.is_empty() => h,
            _ => return (Decimal::ZERO, false, false),
        };

        // Find CVD value from 1 minute ago
        let one_min_ago = if let Some(duration) = Duration::try_minutes(1) {
            now - duration
        } else {
            return (Decimal::ZERO, false, false);
        };

        // Get current CVD
        let current_cvd = self.cvd.get(symbol).copied().unwrap_or(Decimal::ZERO);

        // Find closest CVD value to 1 minute ago
        let mut cvd_1min_ago = current_cvd;
        for (ts, cvd) in history.iter().rev() {
            if *ts <= one_min_ago {
                cvd_1min_ago = *cvd;
                break;
            }
        }

        let cvd_change = current_cvd - cvd_1min_ago;

        // Calculate thresholds based on average bar delta
        let (rapid_drop, rapid_rise) = if let Some(deltas) = self.recent_deltas.get(symbol) {
            if !deltas.is_empty() {
                let avg_abs_delta: Decimal = deltas.iter().map(|d| d.abs()).sum::<Decimal>()
                    / Decimal::from(deltas.len() as u64);
                let threshold = avg_abs_delta * Decimal::from(5);

                // Rapid drop: CVD dropped by more than threshold
                let drop = cvd_change < -threshold;
                // Rapid rise: CVD rose by more than threshold
                let rise = cvd_change > threshold;

                (drop, rise)
            } else {
                (false, false)
            }
        } else {
            (false, false)
        };

        (cvd_change, rapid_drop, rapid_rise)
    }

    /// Clean CVD history older than 5 minutes
    fn clean_cvd_history(&mut self, symbol: &str, now: DateTime<Utc>) {
        if let Some(history) = self.cvd_history.get_mut(symbol) {
            if let Some(five_min_ago) = Duration::try_minutes(5) {
                let cutoff = now - five_min_ago;
                history.retain(|(ts, _)| *ts >= cutoff);
            }
        }
    }

    /// Analyze a completed range bar for order flow signals
    pub fn analyze_bar(&mut self, bar: &RangeBar) -> OrderFlowMetrics {
        let bar_delta = bar.delta();

        // Update CVD
        let cvd = self.cvd.entry(bar.symbol.clone()).or_insert(Decimal::ZERO);
        *cvd += bar_delta;
        let current_cvd = *cvd;

        // Record CVD history
        let history = self.cvd_history.entry(bar.symbol.clone()).or_insert_with(Vec::new);
        history.push((bar.close_time, current_cvd));
        self.clean_cvd_history(&bar.symbol, bar.close_time);

        // Track recent deltas
        let deltas = self
            .recent_deltas
            .entry(bar.symbol.clone())
            .or_insert_with(Vec::new);
        deltas.push(bar_delta);
        if deltas.len() > 50 {
            deltas.remove(0);
        }

        // Track recent volumes per symbol for burst detection
        let volumes = self
            .recent_volumes
            .entry(bar.symbol.clone())
            .or_insert_with(Vec::new);
        volumes.push(bar.volume);
        if volumes.len() > self.volume_baseline_bars {
            volumes.remove(0);
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

        // Get CVD 1-minute change
        let (cvd_1min_change, cvd_rapid_drop, cvd_rapid_rise) = self.get_cvd_1min_change(&bar.symbol, bar.close_time);
        let (avg_bar_volume, volume_burst_ratio, volume_burst) = self.get_volume_burst_metrics(&bar.symbol, bar.volume);

        if absorption_detected {
            info!(
                symbol = %bar.symbol,
                absorption_side = ?absorption_side,
                bar_delta = %bar_delta,
                cvd = %current_cvd,
                cvd_1min_change = %cvd_1min_change,
                cvd_rapid_drop = %cvd_rapid_drop,
                cvd_rapid_rise = %cvd_rapid_rise,
                volume_burst_ratio = %volume_burst_ratio,
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
            cvd_1min_change,
            cvd_rapid_drop,
            cvd_rapid_rise,
            avg_bar_volume,
            volume_burst_ratio,
            volume_burst,
            timestamp: Utc::now(),
        }
    }

    fn get_volume_burst_metrics(
        &self,
        symbol: &str,
        current_volume: Decimal,
    ) -> (Decimal, Decimal, bool) {
        let volumes = match self.recent_volumes.get(symbol) {
            Some(v) if v.len() >= 5 => v,
            _ => return (Decimal::ZERO, Decimal::ZERO, false),
        };

        let avg = volumes.iter().copied().sum::<Decimal>() / Decimal::from(volumes.len() as u64);
        if avg <= Decimal::ZERO {
            return (avg, Decimal::ZERO, false);
        }

        let ratio = current_volume / avg;
        let burst = ratio >= self.volume_burst_multiplier;
        (avg, ratio, burst)
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
        let volumes = match self.recent_volumes.get(&bar.symbol) {
            Some(v) if v.len() >= 5 => v,
            _ => return false,
        };

        let avg_volume: Decimal = volumes.iter().copied().sum::<Decimal>()
            / Decimal::from(volumes.len() as u64);

        bar.volume > avg_volume * self.large_volume_multiplier
    }
}
