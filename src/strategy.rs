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
}
