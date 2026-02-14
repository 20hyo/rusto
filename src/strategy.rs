use crate::config::{RiskConfig, StrategyConfig};
use crate::types::{
    EntryFeatures, OrderFlowMetrics, RangeBar, SetupType, Side, TradeSignal, VolumeProfileSnapshot,
};
use rusqlite::{params, Connection};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use tracing::{info, warn};

#[derive(Clone)]
struct AdvancedSample {
    bar: RangeBar,
    flow: OrderFlowMetrics,
    profile: VolumeProfileSnapshot,
}

/// Generates trading signals based on order flow + volume profile analysis
pub struct StrategyEngine {
    config: StrategyConfig,
    risk_config: RiskConfig,
    tuning_db_path: Option<String>,
    /// Latest volume profile per symbol
    profiles: BTreeMap<String, VolumeProfileSnapshot>,
    /// Recent bars per symbol
    recent_bars: BTreeMap<String, Vec<RangeBar>>,
    /// Latest order flow per symbol
    latest_flow: BTreeMap<String, OrderFlowMetrics>,
    /// Historical samples for adaptive burst-threshold tuning
    advanced_samples: BTreeMap<String, Vec<AdvancedSample>>,
    /// Tuned volume burst ratio per symbol
    tuned_volume_burst_ratio: BTreeMap<String, Decimal>,
    /// Last bar index where tuning was run per symbol
    last_burst_tune_bar: BTreeMap<String, u64>,
    /// Last bar index where AdvancedOrderFlow signal was emitted (per symbol)
    last_advanced_signal_bar: BTreeMap<String, u64>,
}

impl StrategyEngine {
    pub fn new(
        config: StrategyConfig,
        risk_config: RiskConfig,
        tuning_db_path: Option<String>,
    ) -> Self {
        if let Some(path) = tuning_db_path.as_deref() {
            Self::ensure_tuning_log_table(path);
        }

        Self {
            config,
            risk_config,
            tuning_db_path,
            profiles: BTreeMap::new(),
            recent_bars: BTreeMap::new(),
            latest_flow: BTreeMap::new(),
            advanced_samples: BTreeMap::new(),
            tuned_volume_burst_ratio: BTreeMap::new(),
            last_burst_tune_bar: BTreeMap::new(),
            last_advanced_signal_bar: BTreeMap::new(),
        }
    }

    pub fn update_profile(&mut self, profile: VolumeProfileSnapshot) {
        self.profiles.insert(profile.symbol.clone(), profile);
    }

    pub fn update_flow(&mut self, flow: OrderFlowMetrics) {
        self.latest_flow.insert(flow.symbol.clone(), flow);
    }

    /// Process a completed bar and check all enabled setups
    pub fn process_bar(&mut self, bar: &RangeBar) -> Vec<TradeSignal> {
        let bars = self
            .recent_bars
            .entry(bar.symbol.clone())
            .or_insert_with(Vec::new);
        bars.push(bar.clone());
        if bars.len() > 100 {
            bars.drain(..bars.len() - 100);
        }
        if let (Some(flow), Some(profile)) = (
            self.latest_flow.get(&bar.symbol).cloned(),
            self.profiles.get(&bar.symbol).cloned(),
        ) {
            let samples = self
                .advanced_samples
                .entry(bar.symbol.clone())
                .or_insert_with(Vec::new);
            samples.push(AdvancedSample {
                bar: bar.clone(),
                flow,
                profile,
            });
            if samples.len() > 400 {
                samples.drain(..samples.len() - 400);
            }
            self.maybe_tune_volume_burst_ratio(&bar.symbol, bar.bar_index);
        }

        let mut signals = Vec::new();

        let enabled_setups = self.config.enabled_setups.clone();
        for setup in enabled_setups {
            match setup.as_str() {
                "AAA" => {
                    if let Some(sig) = self.check_aaa(bar) {
                        signals.push(sig);
                    }
                }
                "MomentumSqueeze" => {
                    if let Some(sig) = self.check_momentum_squeeze(bar) {
                        signals.push(sig);
                    }
                }
                "AbsorptionReversal" => {
                    if let Some(sig) = self.check_absorption_reversal(bar) {
                        signals.push(sig);
                    }
                }
                "AdvancedOrderFlow" => {
                    if let Some(sig) = self.check_advanced_orderflow(bar) {
                        signals.push(sig);
                    }
                }
                _ => {}
            }
        }

        signals
    }

    /// AAA (Absorption At Area):
    /// Price near VAL + sell absorption → Long (target: VAH)
    /// Price near VAH + buy absorption → Short (target: VAL)
    fn check_aaa(&self, bar: &RangeBar) -> Option<TradeSignal> {
        let profile = self.profiles.get(&bar.symbol)?;
        let flow = self.latest_flow.get(&bar.symbol)?;

        if !flow.absorption_detected {
            return None;
        }

        let tick_size = Decimal::ONE; // simplified tick
        let distance_threshold = tick_size * Decimal::from(self.config.aaa_poc_distance_ticks);
        let stop_distance = tick_size * Decimal::from(self.risk_config.default_stop_ticks);
        // Near VAL + sell absorption → Long
        if flow.absorption_side == Some(Side::Sell)
            && (bar.close - profile.val).abs() <= distance_threshold
        {
            let entry = bar.close;
            let stop = entry - stop_distance;
            let target = profile.vah; // Target the VAH
            let confidence = Decimal::try_from(0.7).unwrap_or(Decimal::ONE);

            info!(
                symbol = %bar.symbol,
                setup = "AAA",
                side = "Long",
                entry = %entry,
                stop = %stop,
                target = %target,
                "AAA Long signal at VAL"
            );

            return Some(TradeSignal::new(
                bar.symbol.clone(),
                Side::Buy,
                SetupType::AAA,
                entry,
                stop,
                target,
                confidence,
            ));
        }

        // Near VAH + buy absorption → Short
        if flow.absorption_side == Some(Side::Buy)
            && (bar.close - profile.vah).abs() <= distance_threshold
        {
            let entry = bar.close;
            let stop = entry + stop_distance;
            let target = profile.val; // Target the VAL

            let confidence = Decimal::try_from(0.7).unwrap_or(Decimal::ONE);

            info!(
                symbol = %bar.symbol,
                setup = "AAA",
                side = "Short",
                entry = %entry,
                stop = %stop,
                target = %target,
                "AAA Short signal at VAH"
            );

            return Some(TradeSignal::new(
                bar.symbol.clone(),
                Side::Sell,
                SetupType::AAA,
                entry,
                stop,
                target,
                confidence,
            ));
        }

        None
    }

    /// Momentum Squeeze: breakout of session high/low + delta confirmation
    fn check_momentum_squeeze(&self, bar: &RangeBar) -> Option<TradeSignal> {
        let profile = self.profiles.get(&bar.symbol)?;
        let flow = self.latest_flow.get(&bar.symbol)?;
        let bars = self.recent_bars.get(&bar.symbol)?;

        if bars.len() < self.config.momentum_lookback_bars {
            return None;
        }

        let min_delta =
            Decimal::try_from(self.config.min_delta_confirmation).unwrap_or(Decimal::ONE);
        let stop_distance = Decimal::from(self.risk_config.default_stop_ticks);
        let target_mult =
            Decimal::try_from(self.risk_config.default_target_multiplier).unwrap_or(Decimal::TWO);

        // Breakout above session high + positive delta
        if bar.close > profile.session_high && flow.bar_delta > min_delta {
            let entry = bar.close;
            let stop = entry - stop_distance;
            let target = entry + (stop_distance * target_mult);

            info!(
                symbol = %bar.symbol,
                setup = "MomentumSqueeze",
                side = "Long",
                entry = %entry,
                "Breakout above session high"
            );

            return Some(TradeSignal::new(
                bar.symbol.clone(),
                Side::Buy,
                SetupType::MomentumSqueeze,
                entry,
                stop,
                target,
                Decimal::try_from(0.6).unwrap_or(Decimal::ONE),
            ));
        }

        // Breakout below session low + negative delta
        if bar.close < profile.session_low && flow.bar_delta < -min_delta {
            let entry = bar.close;
            let stop = entry + stop_distance;
            let target = entry - (stop_distance * target_mult);

            info!(
                symbol = %bar.symbol,
                setup = "MomentumSqueeze",
                side = "Short",
                entry = %entry,
                "Breakout below session low"
            );

            return Some(TradeSignal::new(
                bar.symbol.clone(),
                Side::Sell,
                SetupType::MomentumSqueeze,
                entry,
                stop,
                target,
                Decimal::try_from(0.6).unwrap_or(Decimal::ONE),
            ));
        }

        None
    }

    /// Absorption Reversal: absorption detected → enter opposite direction
    fn check_absorption_reversal(&self, bar: &RangeBar) -> Option<TradeSignal> {
        let flow = self.latest_flow.get(&bar.symbol)?;

        if !flow.absorption_detected {
            return None;
        }

        let stop_distance = Decimal::from(self.risk_config.default_stop_ticks);
        let target_mult =
            Decimal::try_from(self.risk_config.default_target_multiplier).unwrap_or(Decimal::TWO);

        match flow.absorption_side? {
            Side::Sell => {
                // Sell absorbed → price should go up → Long
                let entry = bar.close;
                let stop = entry - stop_distance;
                let target = entry + (stop_distance * target_mult);

                info!(
                    symbol = %bar.symbol,
                    setup = "AbsorptionReversal",
                    side = "Long",
                    entry = %entry,
                    "Sell absorption reversal"
                );

                Some(TradeSignal::new(
                    bar.symbol.clone(),
                    Side::Buy,
                    SetupType::AbsorptionReversal,
                    entry,
                    stop,
                    target,
                    Decimal::try_from(0.65).unwrap_or(Decimal::ONE),
                ))
            }
            Side::Buy => {
                // Buy absorbed → price should go down → Short
                let entry = bar.close;
                let stop = entry + stop_distance;
                let target = entry - (stop_distance * target_mult);

                info!(
                    symbol = %bar.symbol,
                    setup = "AbsorptionReversal",
                    side = "Short",
                    entry = %entry,
                    "Buy absorption reversal"
                );

                Some(TradeSignal::new(
                    bar.symbol.clone(),
                    Side::Sell,
                    SetupType::AbsorptionReversal,
                    entry,
                    stop,
                    target,
                    Decimal::try_from(0.65).unwrap_or(Decimal::ONE),
                ))
            }
        }
    }

    /// AdvancedOrderFlow: "폭발 직전의 압축 포착"
    /// LONG: VAL/HVN + CVD급락 + 매도흡수 → Best Bid 진입 → TP1(VWAP 50%), TP2(VAH 100%)
    /// SHORT: VAH/HVN + CVD급등 + 매수흡수 → Best Ask 진입 → TP1(VWAP 50%), TP2(VAL 100%)
    fn check_advanced_orderflow(&mut self, bar: &RangeBar) -> Option<TradeSignal> {
        let profile = self.profiles.get(&bar.symbol)?;
        let flow = self.latest_flow.get(&bar.symbol)?;

        // Cooldown to avoid rapid-fire signals in noisy conditions.
        if let Some(last_bar) = self.last_advanced_signal_bar.get(&bar.symbol) {
            if bar.bar_index.saturating_sub(*last_bar) < self.config.advanced_cooldown_bars as u64 {
                return None;
            }
        }

        let zone_threshold = Decimal::from(self.config.advanced_zone_ticks);
        let min_imbalance = Decimal::try_from(self.config.advanced_min_imbalance_ratio)
            .unwrap_or(Decimal::new(18, 1));
        let min_abs_cvd_change = Decimal::try_from(self.config.advanced_min_cvd_1min_change)
            .unwrap_or(Decimal::new(5, 0));
        let min_bar_range_pct =
            Decimal::try_from(self.config.advanced_min_bar_range_pct).unwrap_or(Decimal::new(3, 2));
        let min_volume_burst_ratio = self.min_volume_burst_ratio_for(&bar.symbol);

        let bar_range = (bar.high - bar.low).abs();
        let bar_range_pct = if bar.close > Decimal::ZERO {
            (bar_range / bar.close) * Decimal::from(100)
        } else {
            Decimal::ZERO
        };
        if bar_range_pct < min_bar_range_pct {
            return None;
        }

        if flow.cvd_1min_change.abs() < min_abs_cvd_change {
            return None;
        }
        if !flow.volume_burst || flow.volume_burst_ratio < min_volume_burst_ratio {
            return None;
        }

        match self.advanced_side_without_burst(bar, flow, profile, zone_threshold, min_imbalance)? {
            Side::Buy => {
                let near_val = (bar.close - profile.val).abs() <= zone_threshold;
                let near_hvn = profile
                    .hvn
                    .map_or(false, |hvn| (bar.close - hvn).abs() <= zone_threshold);
                let zone_distance_pct = self.zone_distance_pct(bar.close, profile);
                let features = EntryFeatures {
                    imbalance_ratio: flow.imbalance_ratio,
                    cvd_1min_change: flow.cvd_1min_change,
                    volume_burst_ratio: flow.volume_burst_ratio,
                    bar_range_pct,
                    zone_distance_pct,
                    near_val,
                    near_vah: false,
                    near_hvn,
                };
                let entry = bar.close;
                let stop = entry * Decimal::new(996, 3); // -0.4%
                let tp2 = profile.vah;

                info!(
                    symbol = %bar.symbol,
                    setup = "AdvancedOrderFlow",
                    side = "🟢 LONG",
                    entry = %entry,
                    stop = %stop,
                    tp1_vwap = %profile.vwap,
                    tp2_vah = %tp2,
                    cvd_change = %flow.cvd_1min_change,
                    volume_burst_ratio = %flow.volume_burst_ratio,
                    required_burst_ratio = %min_volume_burst_ratio,
                    near_zone = if near_val { "VAL" } else { "HVN" },
                    "🎯 Long: 매도 압축 포착!"
                );

                self.last_advanced_signal_bar
                    .insert(bar.symbol.clone(), bar.bar_index);

                return Some(
                    TradeSignal::new(
                        bar.symbol.clone(),
                        Side::Buy,
                        SetupType::AdvancedOrderFlow,
                        entry,
                        stop,
                        tp2,
                        Decimal::try_from(0.85).unwrap_or(Decimal::ONE),
                    )
                    .with_entry_features(features),
                );
            }
            Side::Sell => {
                let near_vah = (bar.close - profile.vah).abs() <= zone_threshold;
                let near_hvn = profile
                    .hvn
                    .map_or(false, |hvn| (bar.close - hvn).abs() <= zone_threshold);
                let zone_distance_pct = self.zone_distance_pct(bar.close, profile);
                let features = EntryFeatures {
                    imbalance_ratio: flow.imbalance_ratio,
                    cvd_1min_change: flow.cvd_1min_change,
                    volume_burst_ratio: flow.volume_burst_ratio,
                    bar_range_pct,
                    zone_distance_pct,
                    near_val: false,
                    near_vah,
                    near_hvn,
                };
                let entry = bar.close;
                let stop = entry * Decimal::new(1004, 3); // +0.4%
                let tp2 = profile.val;

                info!(
                    symbol = %bar.symbol,
                    setup = "AdvancedOrderFlow",
                    side = "🔴 SHORT",
                    entry = %entry,
                    stop = %stop,
                    tp1_vwap = %profile.vwap,
                    tp2_val = %tp2,
                    cvd_change = %flow.cvd_1min_change,
                    volume_burst_ratio = %flow.volume_burst_ratio,
                    required_burst_ratio = %min_volume_burst_ratio,
                    near_zone = if near_vah { "VAH" } else { "HVN" },
                    "🎯 Short: 매수 압축 포착!"
                );

                self.last_advanced_signal_bar
                    .insert(bar.symbol.clone(), bar.bar_index);

                return Some(
                    TradeSignal::new(
                        bar.symbol.clone(),
                        Side::Sell,
                        SetupType::AdvancedOrderFlow,
                        entry,
                        stop,
                        tp2,
                        Decimal::try_from(0.85).unwrap_or(Decimal::ONE),
                    )
                    .with_entry_features(features),
                );
            }
        }
    }

    fn zone_distance_pct(&self, price: Decimal, profile: &VolumeProfileSnapshot) -> Decimal {
        if price <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        let mut min_dist = (price - profile.val).abs();
        let vah_dist = (price - profile.vah).abs();
        if vah_dist < min_dist {
            min_dist = vah_dist;
        }
        if let Some(hvn) = profile.hvn {
            let hvn_dist = (price - hvn).abs();
            if hvn_dist < min_dist {
                min_dist = hvn_dist;
            }
        }
        (min_dist / price) * Decimal::from(100)
    }

    fn min_volume_burst_ratio_for(&self, symbol: &str) -> Decimal {
        self.tuned_volume_burst_ratio
            .get(symbol)
            .copied()
            .unwrap_or_else(|| {
                Decimal::try_from(self.config.advanced_min_volume_burst_ratio)
                    .unwrap_or(Decimal::new(18, 1))
            })
    }

    fn advanced_side_without_burst(
        &self,
        bar: &RangeBar,
        flow: &OrderFlowMetrics,
        profile: &VolumeProfileSnapshot,
        zone_threshold: Decimal,
        min_imbalance: Decimal,
    ) -> Option<Side> {
        // ========== LONG 조건 ==========
        let near_val = (bar.close - profile.val).abs() <= zone_threshold;
        let near_hvn = profile
            .hvn
            .map_or(false, |hvn| (bar.close - hvn).abs() <= zone_threshold);
        let reversal_ok_long = !self.config.advanced_require_reversal_bar || bar.close > bar.open;
        let sell_to_buy_ratio = if flow.imbalance_ratio > Decimal::ZERO {
            Decimal::ONE / flow.imbalance_ratio
        } else {
            Decimal::from(999)
        };

        if (near_val || near_hvn)
            && flow.cvd_rapid_drop
            && flow.absorption_detected
            && flow.absorption_side == Some(Side::Sell)
            && sell_to_buy_ratio >= min_imbalance
            && reversal_ok_long
            && profile.vwap > bar.close
            && profile.vah > profile.vwap
        {
            return Some(Side::Buy);
        }

        // ========== SHORT 조건 ==========
        let near_vah = (bar.close - profile.vah).abs() <= zone_threshold;
        let reversal_ok_short = !self.config.advanced_require_reversal_bar || bar.close < bar.open;
        if (near_vah || near_hvn)
            && flow.cvd_rapid_rise
            && flow.absorption_detected
            && flow.absorption_side == Some(Side::Buy)
            && flow.imbalance_ratio >= min_imbalance
            && reversal_ok_short
            && profile.vwap < bar.close
            && profile.val < profile.vwap
        {
            return Some(Side::Sell);
        }

        None
    }

    fn maybe_tune_volume_burst_ratio(&mut self, symbol: &str, current_bar_index: u64) {
        if !self.config.advanced_auto_tune_volume_burst {
            return;
        }

        if let Some(last) = self.last_burst_tune_bar.get(symbol) {
            if current_bar_index.saturating_sub(*last) < 5 {
                return;
            }
        }

        if let Some((best_ratio, trades, win_rate, expectancy_pct)) =
            self.backtest_best_volume_burst(symbol)
        {
            let prev = self.tuned_volume_burst_ratio.get(symbol).copied();
            let changed = prev.map_or(true, |p| p != best_ratio);
            self.tuned_volume_burst_ratio
                .insert(symbol.to_string(), best_ratio);
            self.log_tuning_result_sqlite(
                symbol,
                best_ratio,
                trades,
                win_rate,
                expectancy_pct,
                changed,
            );
            if changed {
                info!(
                    symbol = %symbol,
                    tuned_ratio = %best_ratio,
                    trades = trades,
                    win_rate = %win_rate,
                    expectancy_pct = %expectancy_pct,
                    "Auto-tuned volume burst threshold (rolling backtest)"
                );
            }
        }

        self.last_burst_tune_bar
            .insert(symbol.to_string(), current_bar_index);
    }

    fn ensure_tuning_log_table(path: &str) {
        let conn = match Connection::open(path) {
            Ok(c) => c,
            Err(e) => {
                warn!(db_path = %path, error = %e, "Failed to open SQLite for tuning logs");
                return;
            }
        };

        if let Err(e) = conn.execute(
            "CREATE TABLE IF NOT EXISTS volume_burst_tuning_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                symbol TEXT NOT NULL,
                tuned_ratio REAL NOT NULL,
                trades INTEGER NOT NULL,
                win_rate_pct REAL NOT NULL,
                expectancy_pct REAL NOT NULL,
                lookback_bars INTEGER NOT NULL,
                lookahead_bars INTEGER NOT NULL,
                stop_pct REAL NOT NULL,
                target_pct REAL NOT NULL,
                changed INTEGER NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        ) {
            warn!(db_path = %path, error = %e, "Failed to create tuning log table");
        }
    }

    fn log_tuning_result_sqlite(
        &self,
        symbol: &str,
        tuned_ratio: Decimal,
        trades: usize,
        win_rate_pct: Decimal,
        expectancy_pct: Decimal,
        changed: bool,
    ) {
        let Some(path) = self.tuning_db_path.as_deref() else {
            return;
        };

        let conn = match Connection::open(path) {
            Ok(c) => c,
            Err(e) => {
                warn!(db_path = %path, error = %e, "Failed to open SQLite for tuning insert");
                return;
            }
        };

        if let Err(e) = conn.execute(
            "INSERT INTO volume_burst_tuning_logs (
                symbol, tuned_ratio, trades, win_rate_pct, expectancy_pct,
                lookback_bars, lookahead_bars, stop_pct, target_pct, changed
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                symbol,
                tuned_ratio.to_string(),
                trades as i64,
                win_rate_pct.to_string(),
                expectancy_pct.to_string(),
                self.config.advanced_tuning_lookback_bars as i64,
                self.config.advanced_tuning_lookahead_bars as i64,
                self.config.advanced_tuning_stop_pct.to_string(),
                self.config.advanced_tuning_target_pct.to_string(),
                if changed { 1 } else { 0 },
            ],
        ) {
            warn!(db_path = %path, error = %e, "Failed to insert tuning log row");
        }
    }

    fn backtest_best_volume_burst(
        &self,
        symbol: &str,
    ) -> Option<(Decimal, usize, Decimal, Decimal)> {
        let samples = self.advanced_samples.get(symbol)?;
        let lookback = self.config.advanced_tuning_lookback_bars.max(40);
        let lookahead = self.config.advanced_tuning_lookahead_bars.max(2);
        if samples.len() <= lookahead + 20 {
            return None;
        }

        let stop_pct =
            Decimal::try_from(self.config.advanced_tuning_stop_pct).unwrap_or(Decimal::new(20, 2));
        let target_pct = Decimal::try_from(self.config.advanced_tuning_target_pct)
            .unwrap_or(Decimal::new(35, 2));
        if stop_pct <= Decimal::ZERO || target_pct <= Decimal::ZERO {
            return None;
        }

        let zone_threshold = Decimal::from(self.config.advanced_zone_ticks);
        let min_imbalance = Decimal::try_from(self.config.advanced_min_imbalance_ratio)
            .unwrap_or(Decimal::new(18, 1));
        let min_abs_cvd_change = Decimal::try_from(self.config.advanced_min_cvd_1min_change)
            .unwrap_or(Decimal::new(5, 0));
        let min_bar_range_pct =
            Decimal::try_from(self.config.advanced_min_bar_range_pct).unwrap_or(Decimal::new(3, 2));

        let start = samples.len().saturating_sub(lookback + lookahead + 1);
        let end = samples.len().saturating_sub(lookahead);
        if end <= start {
            return None;
        }

        let candidates = [1.2, 1.4, 1.6, 1.8, 2.1, 2.4, 2.8, 3.2];
        let mut best: Option<(Decimal, usize, Decimal, Decimal)> = None;
        let min_trades = self.config.advanced_tuning_min_trades.max(3);

        for candidate in candidates {
            let candidate_dec = Decimal::try_from(candidate).ok()?;
            let mut trades = 0usize;
            let mut wins = 0usize;

            for idx in start..end {
                let sample = &samples[idx];
                let bar = &sample.bar;
                let flow = &sample.flow;
                let profile = &sample.profile;

                let bar_range = (bar.high - bar.low).abs();
                let bar_range_pct = if bar.close > Decimal::ZERO {
                    (bar_range / bar.close) * Decimal::from(100)
                } else {
                    Decimal::ZERO
                };
                if bar_range_pct < min_bar_range_pct {
                    continue;
                }
                if flow.cvd_1min_change.abs() < min_abs_cvd_change {
                    continue;
                }
                if flow.volume_burst_ratio < candidate_dec {
                    continue;
                }

                let side = match self.advanced_side_without_burst(
                    bar,
                    flow,
                    profile,
                    zone_threshold,
                    min_imbalance,
                ) {
                    Some(s) => s,
                    None => continue,
                };

                trades += 1;
                if self
                    .evaluate_lookahead_outcome(samples, idx, lookahead, side, stop_pct, target_pct)
                {
                    wins += 1;
                }
            }

            if trades < min_trades {
                continue;
            }

            let losses = trades.saturating_sub(wins);
            let win_rate =
                Decimal::from(wins as u64) * Decimal::from(100) / Decimal::from(trades as u64);
            let expectancy_pct = ((Decimal::from(wins as u64) * target_pct)
                - (Decimal::from(losses as u64) * stop_pct))
                / Decimal::from(trades as u64);

            match &best {
                None => best = Some((candidate_dec, trades, win_rate, expectancy_pct)),
                Some((_b_ratio, b_trades, _b_wr, b_exp)) => {
                    if expectancy_pct > *b_exp || (expectancy_pct == *b_exp && trades > *b_trades) {
                        best = Some((candidate_dec, trades, win_rate, expectancy_pct));
                    }
                }
            }
        }

        best
    }

    fn evaluate_lookahead_outcome(
        &self,
        samples: &[AdvancedSample],
        idx: usize,
        lookahead: usize,
        side: Side,
        stop_pct: Decimal,
        target_pct: Decimal,
    ) -> bool {
        let entry = samples[idx].bar.close;
        if entry <= Decimal::ZERO {
            return false;
        }

        let stop_mul = Decimal::ONE - (stop_pct / Decimal::from(100));
        let target_mul = Decimal::ONE + (target_pct / Decimal::from(100));
        let (stop, target) = match side {
            Side::Buy => (entry * stop_mul, entry * target_mul),
            Side::Sell => (
                entry * (Decimal::ONE + (stop_pct / Decimal::from(100))),
                entry * (Decimal::ONE - (target_pct / Decimal::from(100))),
            ),
        };

        let end = (idx + lookahead).min(samples.len().saturating_sub(1));
        for sample in samples.iter().take(end + 1).skip(idx + 1) {
            let bar = &sample.bar;
            match side {
                Side::Buy => {
                    let hit_stop = bar.low <= stop;
                    let hit_target = bar.high >= target;
                    if hit_stop && hit_target {
                        return false;
                    }
                    if hit_stop {
                        return false;
                    }
                    if hit_target {
                        return true;
                    }
                }
                Side::Sell => {
                    let hit_stop = bar.high >= stop;
                    let hit_target = bar.low <= target;
                    if hit_stop && hit_target {
                        return false;
                    }
                    if hit_stop {
                        return false;
                    }
                    if hit_target {
                        return true;
                    }
                }
            }
        }

        let final_close = samples[end].bar.close;
        match side {
            Side::Buy => final_close > entry,
            Side::Sell => final_close < entry,
        }
    }
}
