use crate::types::Position;
use rust_decimal::Decimal;
use rusqlite::{params, Connection};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Arc, Mutex};
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    pub total_trades: usize,
    pub winners: usize,
    pub losers: usize,
    pub win_rate_pct: Decimal,
    pub total_pnl: Decimal,
    pub gross_profit: Decimal,
    pub gross_loss_abs: Decimal,
    pub profit_factor: Option<Decimal>,
    pub avg_win: Decimal,
    pub avg_loss: Decimal,
    pub max_drawdown_abs: Decimal,
    pub max_drawdown_pct: Decimal,
}

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

        // Create performance summary table (one row per completed run)
        if let Err(e) = conn.execute(
            "CREATE TABLE IF NOT EXISTS performance_metrics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                total_trades INTEGER NOT NULL,
                winners INTEGER NOT NULL,
                losers INTEGER NOT NULL,
                win_rate_pct REAL NOT NULL,
                total_pnl REAL NOT NULL,
                gross_profit REAL NOT NULL,
                gross_loss_abs REAL NOT NULL,
                profit_factor REAL,
                avg_win REAL NOT NULL,
                avg_loss REAL NOT NULL,
                max_drawdown_abs REAL NOT NULL,
                max_drawdown_pct REAL NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        ) {
            error!("Failed to create performance_metrics table: {}", e);
            panic!("Cannot continue without metrics schema");
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

    pub fn calculate_metrics(
        &self,
        positions: &[Position],
        initial_balance: Decimal,
    ) -> Option<PerformanceMetrics> {
        if positions.is_empty() {
            return None;
        }

        let mut ordered: Vec<&Position> = positions.iter().collect();
        ordered.sort_by_key(|p| p.exit_time.unwrap_or(p.entry_time));

        let total_trades = ordered.len();
        let winners = ordered.iter().filter(|p| p.pnl > Decimal::ZERO).count();
        let losers = ordered.iter().filter(|p| p.pnl < Decimal::ZERO).count();
        let total_pnl: Decimal = positions.iter().map(|p| p.pnl).sum();
        let gross_profit: Decimal = ordered
            .iter()
            .filter(|p| p.pnl > Decimal::ZERO)
            .map(|p| p.pnl)
            .sum();
        let gross_loss_abs: Decimal = ordered
            .iter()
            .filter(|p| p.pnl < Decimal::ZERO)
            .map(|p| p.pnl.abs())
            .sum();
        let avg_win = if winners > 0 {
            gross_profit / Decimal::from(winners as u64)
        } else {
            Decimal::ZERO
        };
        let avg_loss = if losers > 0 {
            -(gross_loss_abs / Decimal::from(losers as u64))
        } else {
            Decimal::ZERO
        };
        let win_rate_pct = if total_trades > 0 {
            Decimal::from(winners as u64) * Decimal::from(100) / Decimal::from(total_trades as u64)
        } else {
            Decimal::ZERO
        };
        let profit_factor = if gross_loss_abs > Decimal::ZERO {
            Some(gross_profit / gross_loss_abs)
        } else if gross_profit > Decimal::ZERO {
            Some(Decimal::from(999))
        } else {
            None
        };

        let mut equity = initial_balance;
        let mut peak = initial_balance;
        let mut max_drawdown_abs = Decimal::ZERO;
        let mut max_drawdown_pct = Decimal::ZERO;

        for p in ordered {
            equity += p.pnl;
            if equity > peak {
                peak = equity;
            }
            let dd_abs = peak - equity;
            if dd_abs > max_drawdown_abs {
                max_drawdown_abs = dd_abs;
            }
            if peak > Decimal::ZERO {
                let dd_pct = (dd_abs / peak) * Decimal::from(100);
                if dd_pct > max_drawdown_pct {
                    max_drawdown_pct = dd_pct;
                }
            }
        }

        Some(PerformanceMetrics {
            total_trades,
            winners,
            losers,
            win_rate_pct,
            total_pnl,
            gross_profit,
            gross_loss_abs,
            profit_factor,
            avg_win,
            avg_loss,
            max_drawdown_abs,
            max_drawdown_pct,
        })
    }

    fn log_metrics_sqlite(&self, m: &PerformanceMetrics) {
        let db = match self.db.lock() {
            Ok(db) => db,
            Err(e) => {
                error!("Failed to acquire database lock for metrics: {}", e);
                return;
            }
        };

        if let Err(e) = db.execute(
            "INSERT INTO performance_metrics (
                total_trades, winners, losers, win_rate_pct, total_pnl,
                gross_profit, gross_loss_abs, profit_factor, avg_win, avg_loss,
                max_drawdown_abs, max_drawdown_pct
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                m.total_trades as i64,
                m.winners as i64,
                m.losers as i64,
                m.win_rate_pct.to_string(),
                m.total_pnl.to_string(),
                m.gross_profit.to_string(),
                m.gross_loss_abs.to_string(),
                m.profit_factor.map(|v| v.to_string()),
                m.avg_win.to_string(),
                m.avg_loss.to_string(),
                m.max_drawdown_abs.to_string(),
                m.max_drawdown_pct.to_string(),
            ],
        ) {
            error!("Failed to insert performance metrics into database: {}", e);
        }
    }

    /// Print summary stats
    pub fn print_summary(&self, positions: &[Position], initial_balance: Decimal) {
        let Some(m) = self.calculate_metrics(positions, initial_balance) else {
            info!("No trades to summarize");
            return;
        };
        self.log_metrics_sqlite(&m);

        info!("=== Trade Summary ===");
        info!("Total trades: {}", m.total_trades);
        info!("Winners: {} | Losers: {}", m.winners, m.losers);
        info!("Win rate: {}%", m.win_rate_pct.round_dp(2));
        info!("Total PnL: {}", m.total_pnl.round_dp(4));
        info!("Gross profit: {} | Gross loss: -{}", m.gross_profit.round_dp(4), m.gross_loss_abs.round_dp(4));
        info!("Avg win: {} | Avg loss: {}", m.avg_win.round_dp(4), m.avg_loss.round_dp(4));
        match m.profit_factor {
            Some(v) => info!("Profit factor: {}", v.round_dp(4)),
            None => info!("Profit factor: N/A"),
        }
        info!(
            "Max drawdown: {} ({:.2}%)",
            m.max_drawdown_abs.round_dp(4),
            m.max_drawdown_pct.round_dp(2)
        );
        info!(
            "BACKTEST_METRICS wr_pct={} pf={} mdd_pct={} mdd_abs={} trades={} pnl={}",
            m.win_rate_pct.round_dp(4),
            m.profit_factor.unwrap_or(Decimal::ZERO).round_dp(4),
            m.max_drawdown_pct.round_dp(4),
            m.max_drawdown_abs.round_dp(4),
            m.total_trades,
            m.total_pnl.round_dp(4)
        );
        info!("=====================");
    }
}
