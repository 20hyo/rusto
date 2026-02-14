use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub range_bar: RangeBarConfig,
    pub volume_profile: VolumeProfileConfig,
    pub order_flow: OrderFlowConfig,
    pub strategy: StrategyConfig,
    pub risk: RiskConfig,
    pub simulator: SimulatorConfig,
    pub logging: LoggingConfig,
    pub discord: DiscordConfig,
    pub binance: BinanceConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GeneralConfig {
    pub symbols: Vec<String>,
    pub log_level: String,
    #[serde(default)]
    pub auto_select_symbols: bool,
    #[serde(default = "default_top_n")]
    pub top_n_symbols: usize,
}

fn default_top_n() -> usize {
    20
}

#[derive(Debug, Deserialize, Clone)]
pub struct RangeBarConfig {
    pub default_pct: Option<f64>,
    #[serde(flatten)]
    pub symbol_ranges: HashMap<String, f64>,
}

impl RangeBarConfig {
    pub fn range_for(&self, symbol: &str) -> Decimal {
        let val = self
            .symbol_ranges
            .get(symbol)
            .or_else(|| self.symbol_ranges.get("default"))
            .copied()
            .unwrap_or(10.0);
        Decimal::try_from(val).unwrap_or(Decimal::TEN)
    }

    /// Calculate range for a symbol using its current price.
    /// Priority: symbol override → default_pct × price → config default.
    pub fn range_for_with_price(&self, symbol: &str, price: Decimal) -> Decimal {
        // 1. Symbol-specific override
        if let Some(&val) = self.symbol_ranges.get(symbol) {
            return Decimal::try_from(val).unwrap_or(Decimal::TEN);
        }
        // 2. Dynamic: default_pct% of price
        if let Some(pct) = self.default_pct {
            if let Ok(pct_dec) = Decimal::try_from(pct) {
                let range = price * pct_dec / Decimal::from(100);
                if range > Decimal::ZERO {
                    return range;
                }
            }
        }
        // 3. Fallback to default key
        self.range_for(symbol)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct VolumeProfileConfig {
    pub tick_size: f64,
    pub value_area_pct: f64,
    pub session_reset_hours: u64,
    #[serde(default = "default_tick_multiplier")]
    pub tick_multiplier: u32,
}

fn default_tick_multiplier() -> u32 {
    10
}

#[derive(Debug, Deserialize, Clone)]
pub struct OrderFlowConfig {
    pub absorption_delta_ratio: f64,
    pub max_price_delta_ticks: u32,
    pub large_volume_multiplier: f64,
    #[serde(default = "default_volume_baseline_bars")]
    pub volume_baseline_bars: usize,
    #[serde(default = "default_volume_burst_multiplier")]
    pub volume_burst_multiplier: f64,
}

fn default_volume_baseline_bars() -> usize {
    40
}

fn default_volume_burst_multiplier() -> f64 {
    1.8
}

#[derive(Debug, Deserialize, Clone)]
pub struct StrategyConfig {
    pub enabled_setups: Vec<String>,
    pub aaa_poc_distance_ticks: u32,
    pub momentum_lookback_bars: usize,
    pub min_delta_confirmation: f64,
    #[serde(default = "default_advanced_zone_ticks")]
    pub advanced_zone_ticks: u32,
    #[serde(default = "default_advanced_min_imbalance_ratio")]
    pub advanced_min_imbalance_ratio: f64,
    #[serde(default = "default_advanced_min_cvd_1min_change")]
    pub advanced_min_cvd_1min_change: f64,
    #[serde(default = "default_advanced_min_bar_range_pct")]
    pub advanced_min_bar_range_pct: f64,
    #[serde(default = "default_advanced_cooldown_bars")]
    pub advanced_cooldown_bars: usize,
    #[serde(default = "default_advanced_require_reversal_bar")]
    pub advanced_require_reversal_bar: bool,
    #[serde(default = "default_advanced_min_volume_burst_ratio")]
    pub advanced_min_volume_burst_ratio: f64,
    #[serde(default = "default_advanced_auto_tune_volume_burst")]
    pub advanced_auto_tune_volume_burst: bool,
    #[serde(default = "default_advanced_tuning_lookback_bars")]
    pub advanced_tuning_lookback_bars: usize,
    #[serde(default = "default_advanced_tuning_lookahead_bars")]
    pub advanced_tuning_lookahead_bars: usize,
    #[serde(default = "default_advanced_tuning_stop_pct")]
    pub advanced_tuning_stop_pct: f64,
    #[serde(default = "default_advanced_tuning_target_pct")]
    pub advanced_tuning_target_pct: f64,
    #[serde(default = "default_advanced_tuning_min_trades")]
    pub advanced_tuning_min_trades: usize,
    #[serde(default = "default_regime_switching_enabled")]
    pub regime_switching_enabled: bool,
    #[serde(default = "default_regime_window_bars")]
    pub regime_window_bars: usize,
    #[serde(default = "default_regime_trend_threshold_pct")]
    pub regime_trend_threshold_pct: f64,
    #[serde(default = "default_regime_high_vol_threshold_pct")]
    pub regime_high_vol_threshold_pct: f64,
    #[serde(default = "default_regime_aggressive_multiplier")]
    pub regime_aggressive_multiplier: f64,
    #[serde(default = "default_regime_conservative_multiplier")]
    pub regime_conservative_multiplier: f64,
    #[serde(default = "default_regime_aggressive_cooldown_mult")]
    pub regime_aggressive_cooldown_mult: f64,
    #[serde(default = "default_regime_conservative_cooldown_mult")]
    pub regime_conservative_cooldown_mult: f64,
}

fn default_advanced_zone_ticks() -> u32 {
    5
}

fn default_advanced_min_imbalance_ratio() -> f64 {
    1.8
}

fn default_advanced_min_cvd_1min_change() -> f64 {
    5.0
}

fn default_advanced_min_bar_range_pct() -> f64 {
    0.03
}

fn default_advanced_cooldown_bars() -> usize {
    3
}

fn default_advanced_require_reversal_bar() -> bool {
    true
}

fn default_advanced_min_volume_burst_ratio() -> f64 {
    1.8
}

fn default_advanced_auto_tune_volume_burst() -> bool {
    true
}

fn default_advanced_tuning_lookback_bars() -> usize {
    120
}

fn default_advanced_tuning_lookahead_bars() -> usize {
    8
}

fn default_advanced_tuning_stop_pct() -> f64 {
    0.20
}

fn default_advanced_tuning_target_pct() -> f64 {
    0.35
}

fn default_advanced_tuning_min_trades() -> usize {
    8
}

fn default_regime_switching_enabled() -> bool {
    true
}

fn default_regime_window_bars() -> usize {
    40
}

fn default_regime_trend_threshold_pct() -> f64 {
    0.25
}

fn default_regime_high_vol_threshold_pct() -> f64 {
    0.12
}

fn default_regime_aggressive_multiplier() -> f64 {
    0.9
}

fn default_regime_conservative_multiplier() -> f64 {
    1.15
}

fn default_regime_aggressive_cooldown_mult() -> f64 {
    0.75
}

fn default_regime_conservative_cooldown_mult() -> f64 {
    1.4
}

#[derive(Debug, Deserialize, Clone)]
pub struct RiskConfig {
    pub initial_balance: f64,
    pub max_risk_per_trade: f64,
    pub daily_loss_limit_pct: f64,
    pub max_concurrent_positions: usize,
    pub break_even_ticks: u32,
    #[serde(default = "default_break_even_min_hold_secs")]
    pub break_even_min_hold_secs: u64,
    #[serde(default = "default_break_even_trigger_rr")]
    pub break_even_trigger_rr: f64,
    #[serde(default = "default_break_even_profit_lock_ticks")]
    pub break_even_profit_lock_ticks: u32,
    #[serde(default = "default_confidence_sizing_enabled")]
    pub confidence_sizing_enabled: bool,
    #[serde(default = "default_min_confidence_scale")]
    pub min_confidence_scale: f64,
    #[serde(default = "default_max_confidence_scale")]
    pub max_confidence_scale: f64,
    #[serde(default = "default_consecutive_loss_limit")]
    pub consecutive_loss_limit: u32,
    #[serde(default = "default_symbol_cooldown_minutes")]
    pub symbol_cooldown_minutes: u64,
    pub default_stop_ticks: u32,
    pub default_target_multiplier: f64,
}

fn default_break_even_min_hold_secs() -> u64 {
    45
}

fn default_break_even_trigger_rr() -> f64 {
    1.2
}

fn default_break_even_profit_lock_ticks() -> u32 {
    1
}

fn default_confidence_sizing_enabled() -> bool {
    true
}

fn default_min_confidence_scale() -> f64 {
    0.6
}

fn default_max_confidence_scale() -> f64 {
    1.2
}

fn default_consecutive_loss_limit() -> u32 {
    3
}

fn default_symbol_cooldown_minutes() -> u64 {
    30
}

#[derive(Debug, Deserialize, Clone)]
pub struct SimulatorConfig {
    pub slippage_ticks: u32,
    pub maker_fee: f64,
    pub taker_fee: f64,
    pub order_book_depth: usize,
    pub leverage: f64,
    pub margin_type: String,
    pub maintenance_margin_rate: f64,
    #[serde(default = "default_soft_stop_seconds")]
    pub soft_stop_seconds: u64,
    #[serde(default = "default_soft_stop_drawdown_pct")]
    pub soft_stop_drawdown_pct: f64,
    #[serde(default = "default_require_orderbook_for_entry")]
    pub require_orderbook_for_entry: bool,
    #[serde(default = "default_max_spread_bps")]
    pub max_spread_bps: f64,
    #[serde(default = "default_min_depth_imbalance_ratio")]
    pub min_depth_imbalance_ratio: f64,
    #[serde(default = "default_expectancy_filter_enabled")]
    pub expectancy_filter_enabled: bool,
    #[serde(default = "default_expectancy_min_trades_per_hour")]
    pub expectancy_min_trades_per_hour: usize,
    #[serde(default = "default_expectancy_min_avg_pnl")]
    pub expectancy_min_avg_pnl: f64,
    #[serde(default = "default_expectancy_lookback_trades")]
    pub expectancy_lookback_trades: usize,
    #[serde(default = "default_slippage_model_enabled")]
    pub slippage_model_enabled: bool,
    #[serde(default = "default_max_model_slippage_bps")]
    pub max_model_slippage_bps: f64,
    #[serde(default = "default_impact_depth_levels")]
    pub impact_depth_levels: usize,
    #[serde(default = "default_impact_weight_bps")]
    pub impact_weight_bps: f64,
}

fn default_soft_stop_seconds() -> u64 {
    45
}

fn default_soft_stop_drawdown_pct() -> f64 {
    0.15
}

fn default_require_orderbook_for_entry() -> bool {
    true
}

fn default_max_spread_bps() -> f64 {
    4.0
}

fn default_min_depth_imbalance_ratio() -> f64 {
    1.05
}

fn default_expectancy_filter_enabled() -> bool {
    true
}

fn default_expectancy_min_trades_per_hour() -> usize {
    12
}

fn default_expectancy_min_avg_pnl() -> f64 {
    0.0
}

fn default_expectancy_lookback_trades() -> usize {
    80
}

fn default_slippage_model_enabled() -> bool {
    true
}

fn default_max_model_slippage_bps() -> f64 {
    6.0
}

fn default_impact_depth_levels() -> usize {
    5
}

fn default_impact_weight_bps() -> f64 {
    8.0
}

#[derive(Debug, Deserialize, Clone)]
pub struct LoggingConfig {
    pub trades_csv_path: String,
    pub trades_json_path: String,
    pub trades_db_path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DiscordConfig {
    pub enabled: bool,
}

impl DiscordConfig {
    pub fn webhook_url(&self) -> Result<String, String> {
        std::env::var("DISCORD_WEBHOOK_URL")
            .map_err(|_| "DISCORD_WEBHOOK_URL not set in .env file".to_string())
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BinanceConfig {
    pub api_url: String,
    pub max_time_offset_ms: i64,
    pub max_latency_ms: f64,
    pub ping_samples: usize,
}

impl AppConfig {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if !self.general.auto_select_symbols && self.general.symbols.is_empty() {
            return Err(
                "At least one symbol must be configured (or enable auto_select_symbols)".into(),
            );
        }
        if self.risk.max_risk_per_trade <= 0.0 || self.risk.max_risk_per_trade > 0.1 {
            return Err("max_risk_per_trade must be between 0 and 0.1".into());
        }
        if self.risk.daily_loss_limit_pct <= 0.0 || self.risk.daily_loss_limit_pct > 0.5 {
            return Err("daily_loss_limit_pct must be between 0 and 0.5".into());
        }
        if self.volume_profile.value_area_pct <= 0.0 || self.volume_profile.value_area_pct > 1.0 {
            return Err("value_area_pct must be between 0 and 1".into());
        }
        if self.risk.min_confidence_scale <= 0.0
            || self.risk.max_confidence_scale <= 0.0
            || self.risk.min_confidence_scale > self.risk.max_confidence_scale
        {
            return Err("confidence scale range is invalid".into());
        }
        if self.simulator.max_spread_bps <= 0.0 {
            return Err("max_spread_bps must be > 0".into());
        }
        if self.simulator.min_depth_imbalance_ratio <= 0.0 {
            return Err("min_depth_imbalance_ratio must be > 0".into());
        }
        if self.strategy.regime_window_bars < 10 {
            return Err("regime_window_bars must be >= 10".into());
        }
        if self.simulator.expectancy_min_trades_per_hour == 0 {
            return Err("expectancy_min_trades_per_hour must be > 0".into());
        }
        if self.simulator.expectancy_lookback_trades == 0 {
            return Err("expectancy_lookback_trades must be > 0".into());
        }
        if self.simulator.max_model_slippage_bps <= 0.0 {
            return Err("max_model_slippage_bps must be > 0".into());
        }
        if self.simulator.impact_depth_levels == 0 {
            return Err("impact_depth_levels must be > 0".into());
        }
        Ok(())
    }
}
