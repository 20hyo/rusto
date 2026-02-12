use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;
use tracing::{info, warn};

#[derive(Debug, Deserialize)]
struct ExchangeInfoResponse {
    symbols: Vec<SymbolData>,
}

#[derive(Debug, Clone, Deserialize)]
struct SymbolData {
    symbol: String,
    status: String,
    #[serde(rename = "baseAsset")]
    base_asset: String,
    #[serde(rename = "quoteAsset")]
    quote_asset: String,
    filters: Vec<Filter>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "filterType")]
enum Filter {
    #[serde(rename = "PRICE_FILTER")]
    PriceFilter {
        #[serde(rename = "minPrice")]
        min_price: String,
        #[serde(rename = "maxPrice")]
        max_price: String,
        #[serde(rename = "tickSize")]
        tick_size: String,
    },
    #[serde(rename = "LOT_SIZE")]
    LotSize {
        #[serde(rename = "minQty")]
        min_qty: String,
        #[serde(rename = "maxQty")]
        max_qty: String,
        #[serde(rename = "stepSize")]
        step_size: String,
    },
    #[serde(rename = "MIN_NOTIONAL")]
    MinNotional {
        #[serde(rename = "notional")]
        notional: String,
    },
    #[serde(rename = "MARKET_LOT_SIZE")]
    MarketLotSize {
        #[serde(rename = "minQty")]
        min_qty: String,
        #[serde(rename = "maxQty")]
        max_qty: String,
        #[serde(rename = "stepSize")]
        step_size: String,
    },
    #[serde(other)]
    Other,
}

/// Symbol trading rules and filters
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub symbol: String,
    pub status: String,
    pub base_asset: String,
    pub quote_asset: String,
    // Price filter
    pub price_tick_size: Decimal,
    pub min_price: Decimal,
    pub max_price: Decimal,
    // Lot size filter
    pub quantity_step_size: Decimal,
    pub min_quantity: Decimal,
    pub max_quantity: Decimal,
    // Min notional
    pub min_notional: Decimal,
}

impl SymbolInfo {
    /// Validate and round price to comply with tick size
    pub fn round_price(&self, price: Decimal) -> Result<Decimal, OrderValidationError> {
        if price < self.min_price {
            return Err(OrderValidationError::PriceTooLow {
                price,
                min: self.min_price,
            });
        }

        if price > self.max_price {
            return Err(OrderValidationError::PriceTooHigh {
                price,
                max: self.max_price,
            });
        }

        // Round to tick size
        let rounded = (price / self.price_tick_size)
            .round_dp(0)
            * self.price_tick_size;

        Ok(rounded)
    }

    /// Validate and round quantity to comply with step size
    pub fn round_quantity(&self, quantity: Decimal) -> Result<Decimal, OrderValidationError> {
        if quantity < self.min_quantity {
            return Err(OrderValidationError::QuantityTooLow {
                quantity,
                min: self.min_quantity,
            });
        }

        if quantity > self.max_quantity {
            return Err(OrderValidationError::QuantityTooHigh {
                quantity,
                max: self.max_quantity,
            });
        }

        // Round to step size
        let rounded = (quantity / self.quantity_step_size)
            .round_dp(0)
            * self.quantity_step_size;

        Ok(rounded)
    }

    /// Validate notional value (price * quantity)
    pub fn validate_notional(&self, price: Decimal, quantity: Decimal) -> Result<(), OrderValidationError> {
        let notional = price * quantity;

        if notional < self.min_notional {
            return Err(OrderValidationError::NotionalTooLow {
                notional,
                min: self.min_notional,
            });
        }

        Ok(())
    }

    /// Full order validation (price, quantity, and notional)
    pub fn validate_order(
        &self,
        price: Decimal,
        quantity: Decimal,
    ) -> Result<(Decimal, Decimal), OrderValidationError> {
        let rounded_price = self.round_price(price)?;
        let rounded_quantity = self.round_quantity(quantity)?;
        self.validate_notional(rounded_price, rounded_quantity)?;

        Ok((rounded_price, rounded_quantity))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OrderValidationError {
    #[error("Price {price} is below minimum {min}")]
    PriceTooLow { price: Decimal, min: Decimal },

    #[error("Price {price} is above maximum {max}")]
    PriceTooHigh { price: Decimal, max: Decimal },

    #[error("Quantity {quantity} is below minimum {min}")]
    QuantityTooLow { quantity: Decimal, min: Decimal },

    #[error("Quantity {quantity} is above maximum {max}")]
    QuantityTooHigh { quantity: Decimal, max: Decimal },

    #[error("Notional value {notional} is below minimum {min}")]
    NotionalTooLow { notional: Decimal, min: Decimal },
}

/// Manages exchange information and symbol filters
pub struct ExchangeInfoManager {
    client: Client,
    base_url: String,
    symbols: HashMap<String, SymbolInfo>,
}

impl ExchangeInfoManager {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            symbols: HashMap::new(),
        }
    }

    /// Fetch and parse exchange info from Binance Futures API
    pub async fn sync(&mut self) -> Result<(), String> {
        let url = format!("{}/fapi/v1/exchangeInfo", self.base_url);

        info!("Fetching exchange info from {}...", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch exchange info: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Exchange info request failed with status: {}",
                response.status()
            ));
        }

        let exchange_info: ExchangeInfoResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse exchange info: {}", e))?;

        info!(
            "Received exchange info for {} symbols",
            exchange_info.symbols.len()
        );

        // Parse and store symbol info
        for symbol_data in exchange_info.symbols {
            if symbol_data.status != "TRADING" {
                warn!(
                    "Symbol {} is not trading (status: {}), skipping",
                    symbol_data.symbol, symbol_data.status
                );
                continue;
            }

            let symbol_name = symbol_data.symbol.clone();
            match self.parse_symbol_info(symbol_data) {
                Ok(info) => {
                    let symbol_lower = info.symbol.to_lowercase();
                    info!(
                        symbol = %info.symbol,
                        tick_size = %info.price_tick_size,
                        step_size = %info.quantity_step_size,
                        min_notional = %info.min_notional,
                        "Symbol info loaded"
                    );
                    self.symbols.insert(symbol_lower, info);
                }
                Err(e) => {
                    warn!(
                        "Failed to parse symbol info for {}: {}",
                        symbol_name, e
                    );
                }
            }
        }

        info!("Exchange info sync completed: {} symbols loaded", self.symbols.len());

        Ok(())
    }

    /// Parse symbol data into SymbolInfo
    fn parse_symbol_info(&self, data: SymbolData) -> Result<SymbolInfo, String> {
        let mut price_tick_size = None;
        let mut min_price = None;
        let mut max_price = None;
        let mut quantity_step_size = None;
        let mut min_quantity = None;
        let mut max_quantity = None;
        let mut min_notional = None;

        for filter in data.filters {
            match filter {
                Filter::PriceFilter {
                    min_price: min,
                    max_price: max,
                    tick_size,
                } => {
                    price_tick_size = Some(Decimal::from_str(&tick_size).unwrap_or(Decimal::ZERO));
                    min_price = Some(Decimal::from_str(&min).unwrap_or(Decimal::ZERO));
                    max_price = Some(Decimal::from_str(&max).unwrap_or(Decimal::MAX));
                }
                Filter::LotSize {
                    min_qty,
                    max_qty,
                    step_size,
                } => {
                    quantity_step_size = Some(Decimal::from_str(&step_size).unwrap_or(Decimal::ZERO));
                    min_quantity = Some(Decimal::from_str(&min_qty).unwrap_or(Decimal::ZERO));
                    max_quantity = Some(Decimal::from_str(&max_qty).unwrap_or(Decimal::MAX));
                }
                Filter::MinNotional { notional } => {
                    min_notional = Some(Decimal::from_str(&notional).unwrap_or(Decimal::ZERO));
                }
                _ => {}
            }
        }

        Ok(SymbolInfo {
            symbol: data.symbol,
            status: data.status,
            base_asset: data.base_asset,
            quote_asset: data.quote_asset,
            price_tick_size: price_tick_size.ok_or("Missing price tick size")?,
            min_price: min_price.ok_or("Missing min price")?,
            max_price: max_price.ok_or("Missing max price")?,
            quantity_step_size: quantity_step_size.ok_or("Missing quantity step size")?,
            min_quantity: min_quantity.ok_or("Missing min quantity")?,
            max_quantity: max_quantity.ok_or("Missing max quantity")?,
            min_notional: min_notional.unwrap_or(Decimal::ZERO),
        })
    }

    /// Get symbol info by symbol name (case-insensitive)
    pub fn get_symbol_info(&self, symbol: &str) -> Option<&SymbolInfo> {
        self.symbols.get(&symbol.to_lowercase())
    }

    /// Check if symbol is available
    pub fn has_symbol(&self, symbol: &str) -> bool {
        self.symbols.contains_key(&symbol.to_lowercase())
    }

    /// Get all loaded symbols
    pub fn symbols(&self) -> &HashMap<String, SymbolInfo> {
        &self.symbols
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_exchange_info_sync() {
        let mut manager = ExchangeInfoManager::new("https://fapi.binance.com".to_string());

        match manager.sync().await {
            Ok(_) => {
                println!("Exchange info sync successful");

                // Check if common symbols exist
                if let Some(btc_info) = manager.get_symbol_info("btcusdt") {
                    println!("BTCUSDT info: {:?}", btc_info);
                    assert!(btc_info.price_tick_size > Decimal::ZERO);
                    assert!(btc_info.quantity_step_size > Decimal::ZERO);
                }
            }
            Err(e) => {
                println!("Exchange info sync failed: {}", e);
            }
        }
    }

    #[test]
    fn test_price_rounding() {
        let info = SymbolInfo {
            symbol: "BTCUSDT".to_string(),
            status: "TRADING".to_string(),
            base_asset: "BTC".to_string(),
            quote_asset: "USDT".to_string(),
            price_tick_size: Decimal::new(1, 1), // 0.1
            min_price: Decimal::from(100),
            max_price: Decimal::from(100000),
            quantity_step_size: Decimal::new(1, 3), // 0.001
            min_quantity: Decimal::new(1, 3),
            max_quantity: Decimal::from(1000),
            min_notional: Decimal::from(5),
        };

        // Test price rounding
        let price = Decimal::new(500025, 1); // 50002.5
        let rounded = info.round_price(price).unwrap();
        assert_eq!(rounded, Decimal::new(500030, 1)); // Should round to 50003.0

        // Test quantity rounding
        let qty = Decimal::new(12345, 4); // 1.2345
        let rounded_qty = info.round_quantity(qty).unwrap();
        assert_eq!(rounded_qty, Decimal::new(1234, 3)); // Should round to 1.234
    }
}
