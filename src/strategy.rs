use crate::config::{RiskConfig, StrategyConfig};
use crate::types::{
    OrderFlowMetrics, RangeBar, SetupType, Side, TradeSignal, VolumeProfileSnapshot,
};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use tracing::info;

/// Generates trading signals based on order flow + volume profile analysis
pub struct StrategyEngine {
    config: StrategyConfig,
    risk_config: RiskConfig,
    /// Latest volume profile per symbol
    profiles: BTreeMap<String, VolumeProfileSnapshot>,
    /// Recent bars per symbol
    recent_bars: BTreeMap<String, Vec<RangeBar>>,
    /// Latest order flow per symbol
    latest_flow: BTreeMap<String, OrderFlowMetrics>,
}

impl StrategyEngine {
    pub fn new(config: StrategyConfig, risk_config: RiskConfig) -> Self {
        Self {
            config,
            risk_config,
            profiles: BTreeMap::new(),
            recent_bars: BTreeMap::new(),
            latest_flow: BTreeMap::new(),
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

        let mut signals = Vec::new();

        for setup in &self.config.enabled_setups {
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
        let distance_threshold =
            tick_size * Decimal::from(self.config.aaa_poc_distance_ticks);
        let stop_distance =
            tick_size * Decimal::from(self.risk_config.default_stop_ticks);
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
        let stop_distance =
            Decimal::from(self.risk_config.default_stop_ticks);
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

        let stop_distance =
            Decimal::from(self.risk_config.default_stop_ticks);
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
    fn check_advanced_orderflow(&self, bar: &RangeBar) -> Option<TradeSignal> {
        let profile = self.profiles.get(&bar.symbol)?;
        let flow = self.latest_flow.get(&bar.symbol)?;

        let tick_size = Decimal::ONE;
        let zone_threshold = tick_size * Decimal::from(5); // 5 ticks

        // ========== LONG 진입 조건 ==========
        // ① Zone: VAL 또는 HVN 근처
        let near_val = (bar.close - profile.val).abs() <= zone_threshold;
        let near_hvn = profile.hvn
            .map_or(false, |hvn| (bar.close - hvn).abs() <= zone_threshold);

        if (near_val || near_hvn)
            && flow.cvd_rapid_drop // ② CVD 급락 (매도세 폭발)
            && flow.absorption_detected
            && flow.absorption_side == Some(Side::Sell) // ③ 매도 흡수 (Bid >> Ask)
        {
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
                near_zone = if near_val { "VAL" } else { "HVN" },
                "🎯 Long: 매도 압축 포착!"
            );

            return Some(TradeSignal::new(
                bar.symbol.clone(),
                Side::Buy,
                SetupType::AdvancedOrderFlow,
                entry,
                stop,
                tp2,
                Decimal::try_from(0.85).unwrap_or(Decimal::ONE),
            ));
        }

        // ========== SHORT 진입 조건 ==========
        // ① Zone: VAH 또는 HVN 근처
        let near_vah = (bar.close - profile.vah).abs() <= zone_threshold;

        if (near_vah || near_hvn)
            && flow.cvd_rapid_rise // ② CVD 급등 (매수세 폭발)
            && flow.absorption_detected
            && flow.absorption_side == Some(Side::Buy) // ③ 매수 흡수 (Ask >> Bid)
        {
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
                near_zone = if near_vah { "VAH" } else { "HVN" },
                "🎯 Short: 매수 압축 포착!"
            );

            return Some(TradeSignal::new(
                bar.symbol.clone(),
                Side::Sell,
                SetupType::AdvancedOrderFlow,
                entry,
                stop,
                tp2,
                Decimal::try_from(0.85).unwrap_or(Decimal::ONE),
            ));
        }

        None
    }
}
