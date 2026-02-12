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
}

struct SymbolProfile {
    /// Volume at each price tick
    levels: BTreeMap<i64, Decimal>, // tick_index -> total volume
    session_start: DateTime<Utc>,
    total_volume: Decimal,
    session_high: Decimal,
    session_low: Decimal,
}

impl SymbolProfile {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            levels: BTreeMap::new(),
            session_start: now,
            total_volume: Decimal::ZERO,
            session_high: Decimal::ZERO,
            session_low: Decimal::MAX,
        }
    }

    fn reset(&mut self, now: DateTime<Utc>) {
        self.levels.clear();
        self.session_start = now;
        self.total_volume = Decimal::ZERO;
        self.session_high = Decimal::ZERO;
        self.session_low = Decimal::MAX;
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
        }
    }

    /// Add a trade to the volume profile. Returns updated snapshot if enough data.
    pub fn process_trade(&mut self, trade: &NormalizedTrade) -> Option<VolumeProfileSnapshot> {
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
        let tick_index = price_to_tick(trade.price, self.tick_size);
        *profile.levels.entry(tick_index).or_insert(Decimal::ZERO) += trade.quantity;
        profile.total_volume += trade.quantity;

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

        let poc = tick_to_price(poc_tick, self.tick_size);
        let vah = tick_to_price(va_high_tick, self.tick_size);
        let val = tick_to_price(va_low_tick, self.tick_size);

        info!(
            symbol = %symbol,
            poc = %poc,
            vah = %vah,
            val = %val,
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

