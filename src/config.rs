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
}

#[derive(Debug, Deserialize, Clone)]
pub struct RangeBarConfig {
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
}

#[derive(Debug, Deserialize, Clone)]
pub struct VolumeProfileConfig {
    pub tick_size: f64,
    pub value_area_pct: f64,
    pub session_reset_hours: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OrderFlowConfig {
    pub absorption_delta_ratio: f64,
    pub max_price_delta_ticks: u32,
    pub large_volume_multiplier: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StrategyConfig {
    pub enabled_setups: Vec<String>,
    pub aaa_poc_distance_ticks: u32,
    pub momentum_lookback_bars: usize,
    pub min_delta_confirmation: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RiskConfig {
    pub initial_balance: f64,
    pub max_risk_per_trade: f64,
    pub daily_loss_limit_pct: f64,
    pub max_concurrent_positions: usize,
    pub break_even_ticks: u32,
    pub default_stop_ticks: u32,
    pub default_target_multiplier: f64,
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
        if self.general.symbols.is_empty() {
            return Err("At least one symbol must be configured".into());
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
        Ok(())
    }
}
