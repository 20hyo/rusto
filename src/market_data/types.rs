use serde::Deserialize;

/// Raw Binance aggTrade stream message
/// Stream: <symbol>@aggTrade
#[derive(Debug, Deserialize)]
pub struct BinanceAggTrade {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "a")]
    pub agg_trade_id: u64,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "q")]
    pub quantity: String,
    #[serde(rename = "f")]
    pub first_trade_id: u64,
    #[serde(rename = "l")]
    pub last_trade_id: u64,
    #[serde(rename = "T")]
    pub trade_time: u64,
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,
}

/// Raw Binance depth stream message
/// Stream: <symbol>@depth@100ms
#[derive(Debug, Deserialize)]
pub struct BinanceDepthUpdate {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "T")]
    pub transaction_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "U")]
    pub first_update_id: u64,
    #[serde(rename = "u")]
    pub final_update_id: u64,
    #[serde(rename = "pu")]
    pub prev_final_update_id: u64,
    #[serde(rename = "b")]
    pub bids: Vec<[String; 2]>, // [price, quantity]
    #[serde(rename = "a")]
    pub asks: Vec<[String; 2]>,
}

/// Combined stream wrapper
#[derive(Debug, Deserialize)]
pub struct BinanceCombinedStream {
    pub stream: String,
    pub data: serde_json::Value,
}
