use crate::types::Position;
use rust_decimal::Decimal;
use rusqlite::{params, Connection};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Arc, Mutex};
use tracing::{error, info};

/// Logs completed trades to CSV, JSON, and SQLite
pub struct TradeLogger {
    csv_path: String,
    json_path: String,
    csv_initialized: bool,
    db: Arc<Mutex<Connection>>,
}

impl TradeLogger {
    pub fn new(csv_path: String, json_path: String, db_path: String) -> Self {
        let conn = Connection::open(&db_path).unwrap_or_else(|e| {
            error!("Failed to open SQLite database: {}", e);
            panic!("Cannot continue without database");
        });

        // Create positions table
        if let Err(e) = conn.execute(
            "CREATE TABLE IF NOT EXISTS positions (
                id TEXT PRIMARY KEY,
                symbol TEXT NOT NULL,
                side TEXT NOT NULL,
                setup TEXT NOT NULL,
                entry_price REAL NOT NULL,
                exit_price REAL,
                quantity REAL NOT NULL,
                stop_loss REAL NOT NULL,
                take_profit REAL NOT NULL,
                pnl REAL NOT NULL,
                status TEXT NOT NULL,
                entry_time TEXT NOT NULL,
                exit_time TEXT,
                break_even_moved INTEGER NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        ) {
            error!("Failed to create positions table: {}", e);
            panic!("Cannot continue without database schema");
        }

        info!("SQLite database initialized at: {}", db_path);

        Self {
            csv_path,
            json_path,
            csv_initialized: false,
            db: Arc::new(Mutex::new(conn)),
        }
    }

    /// Log a closed position
    pub fn log_trade(&mut self, position: &Position) {
        self.log_csv(position);
        self.log_json(position);
        self.log_sqlite(position);
    }

    fn log_sqlite(&self, position: &Position) {
        let db = match self.db.lock() {
            Ok(db) => db,
            Err(e) => {
                error!("Failed to acquire database lock: {}", e);
                return;
            }
        };

        let exit_price = position.exit_price.map(|p| p.to_string());
        let exit_time = position.exit_time.map(|t| t.to_rfc3339());

        if let Err(e) = db.execute(
            "INSERT INTO positions (
                id, symbol, side, setup, entry_price, exit_price, quantity,
                stop_loss, take_profit, pnl, status, entry_time, exit_time, break_even_moved
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ON CONFLICT(id) DO UPDATE SET
                exit_price = excluded.exit_price,
                pnl = excluded.pnl,
                status = excluded.status,
                exit_time = excluded.exit_time",
            params![
                position.id,
                position.symbol,
                format!("{:?}", position.side),
                format!("{}", position.setup),
                position.entry_price.to_string(),
                exit_price,
                position.quantity.to_string(),
                position.stop_loss.to_string(),
                position.take_profit.to_string(),
                position.pnl.to_string(),
                format!("{:?}", position.status),
                position.entry_time.to_rfc3339(),
                exit_time,
                position.break_even_moved as i32,
            ],
        ) {
            error!("Failed to insert position into database: {}", e);
        }
    }

    fn log_csv(&mut self, position: &Position) {
        let file = if !self.csv_initialized {
            self.csv_initialized = true;
            match File::create(&self.csv_path) {
                Ok(mut f) => {
                    let _ = writeln!(
                        f,
                        "id,symbol,side,setup,entry_price,exit_price,quantity,pnl,entry_time,exit_time,break_even_moved"
                    );
                    Some(f)
                }
                Err(e) => {
                    error!("Failed to create CSV file: {}", e);
                    None
                }
            }
        } else {
            OpenOptions::new()
                .append(true)
                .open(&self.csv_path)
                .ok()
        };

        if let Some(mut f) = file {
            let exit_price = position
                .exit_price
                .map(|p| p.to_string())
                .unwrap_or_default();
            let exit_time = position
                .exit_time
                .map(|t| t.to_rfc3339())
                .unwrap_or_default();

            let _ = writeln!(
                f,
                "{},{},{:?},{},{},{},{},{},{},{}",
                position.id,
                position.symbol,
                position.side,
                position.setup,
                position.entry_price,
                exit_price,
                position.quantity,
                position.pnl,
                position.entry_time.to_rfc3339(),
                exit_time,
            );
        }
    }

    fn log_json(&self, position: &Position) {
        let mut file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.json_path)
        {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to open JSON file: {}", e);
                return;
            }
        };

        match serde_json::to_string(position) {
            Ok(json) => {
                let _ = writeln!(file, "{}", json);
            }
            Err(e) => {
                error!("Failed to serialize position: {}", e);
            }
        }
    }

    /// Print summary stats
    pub fn print_summary(&self, positions: &[Position]) {
        if positions.is_empty() {
            info!("No trades to summarize");
            return;
        }

        let total_trades = positions.len();
        let winners: Vec<&Position> = positions.iter().filter(|p| p.pnl > Decimal::ZERO).collect();
        let losers: Vec<&Position> = positions.iter().filter(|p| p.pnl < Decimal::ZERO).collect();
        let total_pnl: Decimal = positions.iter().map(|p| p.pnl).sum();
        let avg_win = if !winners.is_empty() {
            winners.iter().map(|p| p.pnl).sum::<Decimal>() / Decimal::from(winners.len() as u64)
        } else {
            Decimal::ZERO
        };
        let avg_loss = if !losers.is_empty() {
            losers.iter().map(|p| p.pnl).sum::<Decimal>() / Decimal::from(losers.len() as u64)
        } else {
            Decimal::ZERO
        };
        let win_rate = if total_trades > 0 {
            (winners.len() as f64 / total_trades as f64) * 100.0
        } else {
            0.0
        };

        info!("=== Trade Summary ===");
        info!("Total trades: {}", total_trades);
        info!("Winners: {} | Losers: {}", winners.len(), losers.len());
        info!("Win rate: {:.1}%", win_rate);
        info!("Total PnL: {}", total_pnl);
        info!("Avg win: {} | Avg loss: {}", avg_win, avg_loss);
        if avg_loss != Decimal::ZERO {
            let profit_factor = avg_win.abs() / avg_loss.abs();
            info!("Profit factor: {}", profit_factor);
        }
        info!("=====================");
    }
}
