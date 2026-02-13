use crate::config::VolumeProfileConfig;
use crate::types::{NormalizedTrade, VolumeProfileSnapshot};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use tracing::info;

/// Maintains a rolling volume profile per symbol and computes POC/VAH/VAL.
pub struct VolumeProfiler {
    tick_size: Decimal,
    value_area_pct: Decimal,
    session_reset_hours: i64,
    profiles: BTreeMap<String, SymbolProfile>,
    /// Per-symbol tick sizes (override the default tick_size)
    symbol_tick_sizes: BTreeMap<String, Decimal>,
}

struct SymbolProfile {
    /// Volume at each price tick
    levels: BTreeMap<i64, Decimal>, // tick_index -> total volume
    session_start: DateTime<Utc>,
    total_volume: Decimal,
    session_high: Decimal,
    session_low: Decimal,
    /// Recent trades for VWAP and HVN calculation (last 1 hour)
    recent_trades: Vec<(DateTime<Utc>, Decimal, Decimal)>, // (timestamp, price, volume)
}

impl SymbolProfile {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            levels: BTreeMap::new(),
            session_start: now,
            total_volume: Decimal::ZERO,
            session_high: Decimal::ZERO,
            session_low: Decimal::MAX,
            recent_trades: Vec::new(),
        }
    }

    fn reset(&mut self, now: DateTime<Utc>) {
        self.levels.clear();
        self.session_start = now;
        self.total_volume = Decimal::ZERO;
        self.session_high = Decimal::ZERO;
        self.session_low = Decimal::MAX;
        self.recent_trades.clear();
    }

    /// Clean trades older than 1 hour
    fn clean_old_trades(&mut self, now: DateTime<Utc>) {
        if let Some(one_hour_ago) = Duration::try_hours(1) {
            let cutoff = now - one_hour_ago;
            self.recent_trades.retain(|(ts, _, _)| *ts >= cutoff);
        }
    }

    /// Calculate VWAP from recent trades
    fn calculate_vwap(&self) -> Decimal {
        if self.recent_trades.is_empty() {
            return Decimal::ZERO;
        }

        let mut sum_pv = Decimal::ZERO;
        let mut sum_v = Decimal::ZERO;

        for (_, price, volume) in &self.recent_trades {
            sum_pv += price * volume;
            sum_v += volume;
        }

        if sum_v == Decimal::ZERO {
            Decimal::ZERO
        } else {
            sum_pv / sum_v
        }
    }

    /// Find HVN (High Volume Node) from recent trades
    /// Returns the price level with highest volume in last 1 hour
    fn find_hvn(&self, tick_size: Decimal) -> Option<Decimal> {
        if self.recent_trades.is_empty() {
            return None;
        }

        // Group by tick and sum volume
        let mut tick_volumes: BTreeMap<i64, Decimal> = BTreeMap::new();
        for (_, price, volume) in &self.recent_trades {
            let tick = price_to_tick(*price, tick_size);
            *tick_volumes.entry(tick).or_insert(Decimal::ZERO) += volume;
        }

        // Find tick with max volume
        tick_volumes
            .iter()
            .max_by(|a, b| a.1.cmp(b.1))
            .map(|(&tick, _)| tick_to_price(tick, tick_size))
    }
}

impl VolumeProfiler {
    pub fn new(config: &VolumeProfileConfig) -> Self {
        Self {
            tick_size: Decimal::try_from(config.tick_size).unwrap_or(Decimal::ONE),
            value_area_pct: Decimal::try_from(config.value_area_pct).unwrap_or_else(|_| {
                Decimal::new(70, 2)
            }),
            session_reset_hours: config.session_reset_hours as i64,
            profiles: BTreeMap::new(),
            symbol_tick_sizes: BTreeMap::new(),
        }
    }

    /// Set a per-symbol tick size (overrides the default).
    pub fn set_tick_size(&mut self, symbol: &str, tick: Decimal) {
        self.symbol_tick_sizes.insert(symbol.to_string(), tick);
    }

    /// Get the tick size for a symbol (per-symbol or default).
    fn tick_size_for(&self, symbol: &str) -> Decimal {
        self.symbol_tick_sizes
            .get(symbol)
            .copied()
            .unwrap_or(self.tick_size)
    }

    /// Add a trade to the volume profile. Returns updated snapshot if enough data.
    pub fn process_trade(&mut self, trade: &NormalizedTrade) -> Option<VolumeProfileSnapshot> {
        // Get tick size before mutable borrow of profiles
        let sym_tick = self.tick_size_for(&trade.symbol);

        let profile = self
            .profiles
            .entry(trade.symbol.clone())
            .or_insert_with(|| SymbolProfile::new(trade.timestamp));

        // Reset session if expired
        if let Some(duration) = Duration::try_hours(self.session_reset_hours) {
            if trade.timestamp - profile.session_start > duration {
                info!(symbol = %trade.symbol, "Resetting volume profile session");
                profile.reset(trade.timestamp);
            }
        }

        // Update profile
        let tick_index = price_to_tick(trade.price, sym_tick);
        *profile.levels.entry(tick_index).or_insert(Decimal::ZERO) += trade.quantity;
        profile.total_volume += trade.quantity;

        // Add to recent trades for VWAP and HVN
        profile.recent_trades.push((trade.timestamp, trade.price, trade.quantity));
        profile.clean_old_trades(trade.timestamp);

        if trade.price > profile.session_high {
            profile.session_high = trade.price;
        }
        if trade.price < profile.session_low || profile.session_low == Decimal::MAX {
            profile.session_low = trade.price;
        }

        // Only compute snapshot periodically (when we have enough data)
        if profile.levels.len() < 3 {
            return None;
        }

        Some(self.compute_snapshot(&trade.symbol, trade.timestamp))
    }

    fn compute_snapshot(&self, symbol: &str, timestamp: DateTime<Utc>) -> VolumeProfileSnapshot {
        let profile = &self.profiles[symbol];
        let sym_tick = self.tick_size_for(symbol);

        // Find POC (tick with maximum volume)
        let (&poc_tick, _) = profile
            .levels
            .iter()
            .max_by(|a, b| a.1.cmp(b.1))
            .unwrap();

        // Compute Value Area using the expanding algorithm:
        // Start at POC, alternately add the row above and below with more volume
        let target_volume = profile.total_volume * self.value_area_pct;
        let mut area_volume = *profile.levels.get(&poc_tick).unwrap_or(&Decimal::ZERO);
        let mut va_high_tick = poc_tick;
        let mut va_low_tick = poc_tick;

        let ticks: Vec<i64> = profile.levels.keys().copied().collect();

        while area_volume < target_volume {
            let above = next_tick_above(&ticks, va_high_tick);
            let below = next_tick_below(&ticks, va_low_tick);

            let above_vol = above
                .and_then(|t| profile.levels.get(&t))
                .copied()
                .unwrap_or(Decimal::ZERO);
            let below_vol = below
                .and_then(|t| profile.levels.get(&t))
                .copied()
                .unwrap_or(Decimal::ZERO);

            if above.is_none() && below.is_none() {
                break;
            }

            if above_vol >= below_vol {
                if let Some(t) = above {
                    va_high_tick = t;
                    area_volume += above_vol;
                } else if let Some(t) = below {
                    va_low_tick = t;
                    area_volume += below_vol;
                }
            } else if let Some(t) = below {
                va_low_tick = t;
                area_volume += below_vol;
            } else if let Some(t) = above {
                va_high_tick = t;
                area_volume += above_vol;
            }
        }

        let poc = tick_to_price(poc_tick, sym_tick);
        let vah = tick_to_price(va_high_tick, sym_tick);
        let val = tick_to_price(va_low_tick, sym_tick);

        // Calculate VWAP and HVN
        let vwap = profile.calculate_vwap();
        let hvn = profile.find_hvn(sym_tick);

        info!(
            symbol = %symbol,
            poc = %poc,
            vah = %vah,
            val = %val,
            vwap = %vwap,
            hvn = ?hvn,
            total_volume = %profile.total_volume,
            "Volume profile updated"
        );

        VolumeProfileSnapshot {
            symbol: symbol.to_string(),
            poc,
            vah,
            val,
            total_volume: profile.total_volume,
            session_high: profile.session_high,
            session_low: profile.session_low,
            vwap,
            hvn,
            timestamp,
        }
    }
}

fn price_to_tick(price: Decimal, tick_size: Decimal) -> i64 {
    (price / tick_size)
        .floor()
        .to_string()
        .parse::<i64>()
        .unwrap_or(0)
}

fn tick_to_price(tick: i64, tick_size: Decimal) -> Decimal {
    Decimal::from(tick) * tick_size
}

fn next_tick_above(ticks: &[i64], current: i64) -> Option<i64> {
    ticks.iter().copied().find(|&t| t > current)
}

fn next_tick_below(ticks: &[i64], current: i64) -> Option<i64> {
    ticks.iter().rev().copied().find(|&t| t < current)
}

