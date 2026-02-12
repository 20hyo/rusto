use crate::config::RangeBarConfig;
use crate::types::{FootprintLevel, NormalizedTrade, RangeBar, Side};
use chrono::Utc;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use tracing::info;

/// Builds range bars from a stream of normalized trades.
/// A new bar is completed when price moves `range_size` from the bar's open.
pub struct RangeBarBuilder {
    config: RangeBarConfig,
    /// Per-symbol state
    builders: BTreeMap<String, SymbolBarState>,
}

struct SymbolBarState {
    range_size: Decimal,
    current: Option<BuildingBar>,
    bar_count: u64,
}

struct BuildingBar {
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    volume: Decimal,
    buy_volume: Decimal,
    sell_volume: Decimal,
    open_time: chrono::DateTime<Utc>,
    footprint: BTreeMap<String, FootprintLevel>,
}

impl BuildingBar {
    fn new(trade: &NormalizedTrade) -> Self {
        let mut footprint = BTreeMap::new();
        let key = price_key(trade.price);
        let mut level = FootprintLevel::default();
        match trade.side {
            Side::Buy => level.ask_volume = trade.quantity,
            Side::Sell => level.bid_volume = trade.quantity,
        }
        footprint.insert(key, level);

        Self {
            open: trade.price,
            high: trade.price,
            low: trade.price,
            close: trade.price,
            volume: trade.quantity,
            buy_volume: if trade.side == Side::Buy {
                trade.quantity
            } else {
                Decimal::ZERO
            },
            sell_volume: if trade.side == Side::Sell {
                trade.quantity
            } else {
                Decimal::ZERO
            },
            open_time: trade.timestamp,
            footprint,
        }
    }

    fn update(&mut self, trade: &NormalizedTrade) {
        self.close = trade.price;
        if trade.price > self.high {
            self.high = trade.price;
        }
        if trade.price < self.low {
            self.low = trade.price;
        }
        self.volume += trade.quantity;
        match trade.side {
            Side::Buy => self.buy_volume += trade.quantity,
            Side::Sell => self.sell_volume += trade.quantity,
        }

        let key = price_key(trade.price);
        let level = self.footprint.entry(key).or_default();
        match trade.side {
            Side::Buy => level.ask_volume += trade.quantity,
            Side::Sell => level.bid_volume += trade.quantity,
        }
    }

    fn range(&self) -> Decimal {
        self.high - self.low
    }
}

/// Quantize price to a string key for footprint bucketing
fn price_key(price: Decimal) -> String {
    price.round_dp(1).to_string()
}

impl RangeBarBuilder {
    pub fn new(config: RangeBarConfig) -> Self {
        Self {
            config,
            builders: BTreeMap::new(),
        }
    }

    /// Process a trade and return a completed bar if the range threshold was met.
    pub fn process_trade(&mut self, trade: &NormalizedTrade) -> Option<RangeBar> {
        let state = self
            .builders
            .entry(trade.symbol.clone())
            .or_insert_with(|| SymbolBarState {
                range_size: self.config.range_for(&trade.symbol),
                current: None,
                bar_count: 0,
            });

        match &mut state.current {
            None => {
                state.current = Some(BuildingBar::new(trade));
                None
            }
            Some(bar) => {
                bar.update(trade);

                if bar.range() >= state.range_size {
                    let completed = state.current.take().unwrap();
                    state.bar_count += 1;

                    let range_bar = RangeBar {
                        symbol: trade.symbol.clone(),
                        open: completed.open,
                        high: completed.high,
                        low: completed.low,
                        close: completed.close,
                        volume: completed.volume,
                        buy_volume: completed.buy_volume,
                        sell_volume: completed.sell_volume,
                        open_time: completed.open_time,
                        close_time: trade.timestamp,
                        footprint: completed.footprint,
                        bar_index: state.bar_count,
                    };

                    info!(
                        symbol = %range_bar.symbol,
                        bar = state.bar_count,
                        open = %range_bar.open,
                        high = %range_bar.high,
                        low = %range_bar.low,
                        close = %range_bar.close,
                        delta = %range_bar.delta(),
                        volume = %range_bar.volume,
                        "Range bar completed"
                    );

                    // Start new bar with current trade
                    state.current = Some(BuildingBar::new(trade));

                    Some(range_bar)
                } else {
                    None
                }
            }
        }
    }
}
