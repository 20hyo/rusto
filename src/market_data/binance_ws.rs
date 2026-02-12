use crate::market_data::types::{BinanceAggTrade, BinanceCombinedStream, BinanceDepthUpdate};
use crate::types::{DepthLevel, DepthUpdate, MarketEvent, NormalizedTrade, Side};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use rust_decimal::Decimal;
use std::str::FromStr;
use tokio::sync::broadcast;
use tokio_tungstenite::connect_async;
use tracing::{error, info, warn};

const BINANCE_FUTURES_WS: &str = "wss://fstream.binance.com/stream?streams=";

pub struct BinanceWebSocket {
    symbols: Vec<String>,
    tx: broadcast::Sender<MarketEvent>,
}

impl BinanceWebSocket {
    pub fn new(symbols: Vec<String>, tx: broadcast::Sender<MarketEvent>) -> Self {
        Self { symbols, tx }
    }

    fn build_url(&self) -> String {
        let streams: Vec<String> = self
            .symbols
            .iter()
            .flat_map(|s| {
                let lower = s.to_lowercase();
                vec![
                    format!("{}@aggTrade", lower),
                    format!("{}@depth@100ms", lower),
                ]
            })
            .collect();
        format!("{}{}", BINANCE_FUTURES_WS, streams.join("/"))
    }

    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        loop {
            let url = self.build_url();
            info!("Connecting to Binance WebSocket: {}", url);

            match connect_async(&url).await {
                Ok((ws_stream, _response)) => {
                    info!("Connected to Binance WebSocket");
                    let (_write, mut read) = ws_stream.split();

                    loop {
                        tokio::select! {
                            msg = read.next() => {
                                match msg {
                                    Some(Ok(tungstenite::Message::Text(text))) => {
                                        self.handle_message(&text);
                                    }
                                    Some(Ok(tungstenite::Message::Ping(_))) => {}
                                    Some(Ok(tungstenite::Message::Close(_))) => {
                                        warn!("WebSocket closed by server");
                                        break;
                                    }
                                    Some(Err(e)) => {
                                        error!("WebSocket error: {}", e);
                                        break;
                                    }
                                    None => {
                                        warn!("WebSocket stream ended");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            _ = shutdown.changed() => {
                                if *shutdown.borrow() {
                                    info!("Shutdown signal received, closing WebSocket");
                                    return;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to connect to Binance WebSocket: {}", e);
                }
            }

            // Check shutdown before reconnecting
            if *shutdown.borrow() {
                return;
            }

            warn!("Reconnecting in 5 seconds...");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    fn handle_message(&self, text: &str) {
        let combined: BinanceCombinedStream = match serde_json::from_str(text) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to parse combined stream: {}", e);
                return;
            }
        };

        if combined.stream.contains("aggTrade") {
            self.handle_agg_trade(&combined.data);
        } else if combined.stream.contains("depth") {
            self.handle_depth(&combined.data);
        }
    }

    fn handle_agg_trade(&self, data: &serde_json::Value) {
        let trade: BinanceAggTrade = match serde_json::from_value(data.clone()) {
            Ok(t) => t,
            Err(e) => {
                warn!("Failed to parse aggTrade: {}", e);
                return;
            }
        };

        let price = match Decimal::from_str(&trade.price) {
            Ok(p) => p,
            Err(_) => return,
        };
        let quantity = match Decimal::from_str(&trade.quantity) {
            Ok(q) => q,
            Err(_) => return,
        };

        // is_buyer_maker=true means the buyer was the maker, so the aggressor is the seller
        let side = if trade.is_buyer_maker {
            Side::Sell
        } else {
            Side::Buy
        };

        let timestamp = millis_to_datetime(trade.trade_time);

        let normalized = NormalizedTrade {
            symbol: trade.symbol.to_lowercase(),
            price,
            quantity,
            side,
            timestamp,
            trade_id: trade.agg_trade_id,
        };

        let _ = self.tx.send(MarketEvent::Trade(normalized));
    }

    fn handle_depth(&self, data: &serde_json::Value) {
        let depth: BinanceDepthUpdate = match serde_json::from_value(data.clone()) {
            Ok(d) => d,
            Err(e) => {
                warn!("Failed to parse depth: {}", e);
                return;
            }
        };

        let parse_levels = |raw: &[[String; 2]]| -> Vec<DepthLevel> {
            raw.iter()
                .filter_map(|[p, q]| {
                    let price = Decimal::from_str(p).ok()?;
                    let quantity = Decimal::from_str(q).ok()?;
                    Some(DepthLevel { price, quantity })
                })
                .collect()
        };

        let update = DepthUpdate {
            symbol: depth.symbol.to_lowercase(),
            bids: parse_levels(&depth.bids),
            asks: parse_levels(&depth.asks),
            timestamp: millis_to_datetime(depth.event_time),
        };

        let _ = self.tx.send(MarketEvent::Depth(update));
    }
}

fn millis_to_datetime(millis: u64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis as i64).unwrap_or_else(Utc::now)
}
