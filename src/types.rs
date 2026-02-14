use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Side of a trade or order
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn opposite(self) -> Self {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

/// Normalized trade from exchange
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedTrade {
    pub symbol: String,
    pub price: Decimal,
    pub quantity: Decimal,
    pub side: Side,
    pub timestamp: DateTime<Utc>,
    pub trade_id: u64,
}

/// Depth update (bid/ask level)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthLevel {
    pub price: Decimal,
    pub quantity: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthUpdate {
    pub symbol: String,
    pub bids: Vec<DepthLevel>,
    pub asks: Vec<DepthLevel>,
    pub timestamp: DateTime<Utc>,
}

/// Market data event (union of trade and depth)
#[derive(Debug, Clone)]
pub enum MarketEvent {
    Trade(NormalizedTrade),
    Depth(DepthUpdate),
}

/// Footprint: volume at each price level within a bar
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FootprintLevel {
    pub bid_volume: Decimal,
    pub ask_volume: Decimal,
}

/// Range bar with embedded footprint data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeBar {
    pub symbol: String,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    pub buy_volume: Decimal,
    pub sell_volume: Decimal,
    pub open_time: DateTime<Utc>,
    pub close_time: DateTime<Utc>,
    pub footprint: BTreeMap<String, FootprintLevel>, // price_key -> volumes
    pub bar_index: u64,
}

impl RangeBar {
    pub fn delta(&self) -> Decimal {
        self.buy_volume - self.sell_volume
    }
}

/// Volume profile for a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeProfileSnapshot {
    pub symbol: String,
    pub poc: Decimal, // Point of Control - highest volume price
    pub vah: Decimal, // Value Area High
    pub val: Decimal, // Value Area Low
    pub total_volume: Decimal,
    pub session_high: Decimal,
    pub session_low: Decimal,
    pub vwap: Decimal,        // Volume Weighted Average Price (last 1 hour)
    pub hvn: Option<Decimal>, // High Volume Node (last 1 hour)
    pub timestamp: DateTime<Utc>,
}

/// Order flow metrics for a bar
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderFlowMetrics {
    pub symbol: String,
    pub cvd: Decimal,       // Cumulative Volume Delta
    pub bar_delta: Decimal, // Delta for current bar
    pub absorption_detected: bool,
    pub absorption_side: Option<Side>, // Side being absorbed
    pub imbalance_ratio: Decimal,
    pub cvd_1min_change: Decimal,    // CVD change over last 1 minute
    pub cvd_rapid_drop: bool,        // True if CVD dropped rapidly (sell-side explosion)
    pub cvd_rapid_rise: bool,        // True if CVD rose rapidly (buy-side explosion)
    pub avg_bar_volume: Decimal,     // Per-symbol rolling average bar volume
    pub volume_burst_ratio: Decimal, // current volume / avg_bar_volume
    pub volume_burst: bool,          // True if current volume is bursting vs symbol baseline
    pub timestamp: DateTime<Utc>,
}

/// Setup type for trading signals
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SetupType {
    AAA,                // Absorption At Area
    MomentumSqueeze,    // Breakout with delta confirmation
    AbsorptionReversal, // Pure absorption reversal
    AdvancedOrderFlow,  // Advanced order flow: zone filtering + CVD + orderbook imbalance
}

impl std::fmt::Display for SetupType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupType::AAA => write!(f, "AAA"),
            SetupType::MomentumSqueeze => write!(f, "MomentumSqueeze"),
            SetupType::AbsorptionReversal => write!(f, "AbsorptionReversal"),
            SetupType::AdvancedOrderFlow => write!(f, "AdvancedOrderFlow"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitReason {
    StopLoss,
    TakeProfit,
    TP2,
    SoftStop,
    Liquidation,
}

impl std::fmt::Display for ExitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitReason::StopLoss => write!(f, "StopLoss"),
            ExitReason::TakeProfit => write!(f, "TakeProfit"),
            ExitReason::TP2 => write!(f, "TP2"),
            ExitReason::SoftStop => write!(f, "SoftStop"),
            ExitReason::Liquidation => write!(f, "Liquidation"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryFeatures {
    pub imbalance_ratio: Decimal,
    pub cvd_1min_change: Decimal,
    pub volume_burst_ratio: Decimal,
    pub bar_range_pct: Decimal,
    pub zone_distance_pct: Decimal,
    pub near_val: bool,
    pub near_vah: bool,
    pub near_hvn: bool,
}

/// Trading signal generated by strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeSignal {
    pub id: String,
    pub symbol: String,
    pub side: Side,
    pub setup: SetupType,
    pub entry_price: Decimal,
    pub stop_loss: Decimal,
    pub take_profit: Decimal,
    pub confidence: Decimal,
    pub entry_features: Option<EntryFeatures>,
    pub timestamp: DateTime<Utc>,
}

impl TradeSignal {
    pub fn new(
        symbol: String,
        side: Side,
        setup: SetupType,
        entry_price: Decimal,
        stop_loss: Decimal,
        take_profit: Decimal,
        confidence: Decimal,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            symbol,
            side,
            setup,
            entry_price,
            stop_loss,
            take_profit,
            confidence,
            entry_features: None,
            timestamp: Utc::now(),
        }
    }

    pub fn with_entry_features(mut self, features: EntryFeatures) -> Self {
        self.entry_features = Some(features);
        self
    }
}

/// Margin type for futures trading
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarginType {
    Isolated,
    Cross,
}

impl std::fmt::Display for MarginType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarginType::Isolated => write!(f, "Isolated"),
            MarginType::Cross => write!(f, "Cross"),
        }
    }
}

/// Position status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionStatus {
    Open,
    Closed,
    Liquidated,
}

/// Simulated position with leverage support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: String,
    pub symbol: String,
    pub side: Side,
    pub entry_price: Decimal,
    pub quantity: Decimal,
    pub stop_loss: Decimal,
    pub take_profit: Decimal,
    pub setup: SetupType,
    pub status: PositionStatus,
    pub pnl: Decimal,
    pub entry_time: DateTime<Utc>,
    pub exit_time: Option<DateTime<Utc>>,
    pub exit_price: Option<Decimal>,
    pub exit_reason: Option<ExitReason>,
    pub break_even_moved: bool,
    // Leverage fields
    pub leverage: Decimal,
    pub margin_type: MarginType,
    pub liquidation_price: Decimal,
    pub unrealized_pnl: Decimal,
    pub initial_margin: Decimal,
    pub maintenance_margin: Decimal,
    // Multi-stage exit tracking
    pub tp1_filled: bool,           // TP1 (50% at VWAP) executed
    pub tp1_price: Option<Decimal>, // VWAP target
    pub tp2_price: Option<Decimal>, // VAH target
    pub original_quantity: Decimal, // Original full quantity
    pub entry_features: Option<EntryFeatures>,
    pub max_favorable_excursion_pct: Decimal,
    pub max_adverse_excursion_pct: Decimal,
    pub time_to_mfe_secs: Option<i64>,
    pub time_to_mae_secs: Option<i64>,
}

impl Position {
    /// Calculate unrealized PnL based on current mark price
    pub fn calculate_unrealized_pnl(&self, mark_price: Decimal) -> Decimal {
        let raw_pnl = match self.side {
            Side::Buy => (mark_price - self.entry_price) * self.quantity,
            Side::Sell => (self.entry_price - mark_price) * self.quantity,
        };
        raw_pnl
    }

    /// Calculate margin ratio: (balance + unrealized_pnl) / maintenance_margin * 100
    /// Returns percentage. If <= 100%, liquidation occurs
    pub fn calculate_margin_ratio(&self, account_balance: Decimal, mark_price: Decimal) -> Decimal {
        if self.maintenance_margin == Decimal::ZERO {
            return Decimal::from(999); // Safe value
        }
        let unrealized = self.calculate_unrealized_pnl(mark_price);
        let equity = account_balance + unrealized;
        (equity / self.maintenance_margin) * Decimal::from(100)
    }

    /// Check if position should be liquidated based on mark price
    pub fn should_liquidate(&self, mark_price: Decimal) -> bool {
        match self.side {
            Side::Buy => mark_price <= self.liquidation_price,
            Side::Sell => mark_price >= self.liquidation_price,
        }
    }
}

/// Per-symbol trading statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymbolStats {
    pub total_trades: u32,
    pub wins: u32,
    pub losses: u32,
    pub total_pnl: Decimal,
    pub total_win_pnl: Decimal,
    pub total_loss_pnl: Decimal,
    pub open_positions: u32,
}

impl SymbolStats {
    pub fn record_close(&mut self, pnl: Decimal) {
        self.total_trades += 1;
        self.total_pnl += pnl;
        if pnl >= Decimal::ZERO {
            self.wins += 1;
            self.total_win_pnl += pnl;
        } else {
            self.losses += 1;
            self.total_loss_pnl += pnl;
        }
    }

    pub fn win_rate(&self) -> Decimal {
        if self.total_trades == 0 {
            return Decimal::ZERO;
        }
        Decimal::from(self.wins) * Decimal::from(100) / Decimal::from(self.total_trades)
    }

    pub fn profit_factor(&self) -> Decimal {
        if self.total_loss_pnl == Decimal::ZERO {
            return Decimal::ZERO;
        }
        self.total_win_pnl / self.total_loss_pnl.abs()
    }

    pub fn avg_win(&self) -> Decimal {
        if self.wins == 0 {
            return Decimal::ZERO;
        }
        self.total_win_pnl / Decimal::from(self.wins)
    }

    pub fn avg_loss(&self) -> Decimal {
        if self.losses == 0 {
            return Decimal::ZERO;
        }
        self.total_loss_pnl / Decimal::from(self.losses)
    }
}

/// Shared bot status read by the hourly reporter task
#[derive(Debug, Clone, Default)]
pub struct BotStats {
    pub balance: Decimal,
    pub daily_pnl: Decimal,
    pub open_positions: usize,
    pub total_trades: u32,
    pub symbol_stats: BTreeMap<String, SymbolStats>,
}

/// Events flowing through the processing pipeline
#[derive(Debug, Clone)]
pub enum ProcessingEvent {
    NewBar(RangeBar),
    VolumeProfile(VolumeProfileSnapshot),
    OrderFlow(OrderFlowMetrics),
    Signal(TradeSignal),
}

/// Events from the execution engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionEvent {
    PositionOpened(Position),
    PositionClosed(Position),
    PositionLiquidated(Position),
    TP1Filled {
        position_id: String,
        tp1_price: Decimal,
        partial_pnl: Decimal,
    },
    StopMoved {
        position_id: String,
        new_stop: Decimal,
    },
    DailyLimitReached {
        pnl: Decimal,
    },
    /// Hourly status report: network ping + current PnL
    HourlyReport {
        balance: Decimal,
        daily_pnl: Decimal,
        open_positions: usize,
        ping_ms: f64,
        total_trades: u32,
        symbol_stats: BTreeMap<String, SymbolStats>,
    },
}
