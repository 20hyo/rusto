# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rusto is an async order flow trading bot written in Rust that connects to Binance WebSocket feeds, builds range bars (price-movement-based, not time-based), analyzes volume profiles and order flow, generates trading signals, and simulates execution with a paper trading engine.

## Development Commands

### Build and Run
```bash
cargo build              # Debug build
cargo build --release    # Optimized build
cargo run                # Run with config.toml
cargo run --release      # Run optimized version
```

### Testing and Code Quality
```bash
cargo test               # Run all tests
cargo test <test_name>   # Run specific test
cargo check              # Fast type checking
cargo clippy             # Linting
cargo fmt                # Format code
```

### Logging
Set `RUST_LOG` environment variable to control log levels:
```bash
RUST_LOG=debug cargo run    # Debug level logging
RUST_LOG=info cargo run     # Info level logging (default in config.toml)
```

## Architecture

### Async Runtime and Task Structure
The application uses **tokio** with four independent async tasks communicating via channels:

1. **WebSocket Task** (`BinanceWebSocket`): Connects to Binance, receives trades and depth updates, broadcasts `MarketEvent`s
2. **Processing Pipeline Task**: Sequential processing chain:
   - Receives trades from WebSocket
   - Updates `VolumeProfiler` → generates `VolumeProfileSnapshot` (POC, VAH, VAL)
   - Feeds trades to `RangeBarBuilder` → generates `RangeBar` when price range threshold hit
   - When bar completes: `OrderFlowTracker` analyzes it → generates `OrderFlowMetrics`
   - `StrategyEngine` processes bar + profile + flow → generates `TradeSignal`s
   - Emits `ProcessingEvent`s downstream
3. **Simulator Task** (`SimulatorEngine`): Receives signals and market data, manages positions, simulates order book execution, logs trades, emits `ExecutionEvent`s
4. **Discord Task** (`DiscordBot`, optional): Monitors `ExecutionEvent` channel and sends notifications to Discord via webhook

### Channel Architecture
- **Broadcast channel** (`market_tx`): `MarketEvent`s (Trade, Depth) → multiple subscribers
- **MPSC channel** (`processing_tx/rx`): `ProcessingEvent`s → simulator
- **MPSC channel** (`execution_tx/rx`): `ExecutionEvent`s → Discord bot
- **Watch channel** (`shutdown_tx/rx`): Graceful shutdown signal to all tasks

### Core Data Flow
```
Binance WebSocket
  → NormalizedTrade
  → VolumeProfiler (updates POC/VAH/VAL)
  → RangeBarBuilder (accumulates until range threshold)
  → RangeBar (with embedded footprint: BTreeMap<price, FootprintLevel>)
  → OrderFlowTracker (detects absorption, calculates CVD)
  → StrategyEngine (checks AAA, MomentumSqueeze, AbsorptionReversal setups)
  → TradeSignal
  → SimulatorEngine (manages positions, simulates fills)
```

### Key Concepts

**Range Bars**: Bars close when price moves by a configured amount (e.g., 50 USDT for BTC), not by time. Each bar contains a `footprint` (BTreeMap mapping price levels to bid/ask volume).

**Volume Profile**: Tracks volume distribution across price levels. Calculates:
- **POC** (Point of Control): Price level with highest volume
- **VAH/VAL** (Value Area High/Low): Price range containing configured % of volume (default 70%)
- Resets every `session_reset_hours` (default 24h)

**Order Flow Metrics**:
- **CVD** (Cumulative Volume Delta): Running sum of (buy_volume - sell_volume)
- **Bar Delta**: Buy volume - sell volume for a single bar
- **Absorption Detection**: Large volume on one side without significant price movement (configurable thresholds)
- **Imbalance Ratio**: Measures bid/ask volume asymmetry in footprint

**Trading Setups** (in `StrategyEngine`):
1. **AAA (Absorption At Area)**: Absorption detected near VAL (long) or VAH (short), targeting opposite boundary
2. **MomentumSqueeze**: Breakout above/below session high/low with delta confirmation
3. **AbsorptionReversal**: Pure absorption signal → fade the absorbed side

**Risk Management** (`RiskManager`):
- Position sizing based on `max_risk_per_trade` percentage
- Stop loss distance in ticks
- Break-even stop movement after `break_even_ticks` profit
- Daily loss limit enforcement
- Max concurrent positions

**Simulator Engine**:
- Maintains simulated `OrderBook` from depth updates
- Fills orders with configurable slippage
- Applies maker/taker fees
- Logs all trades to CSV and JSON files
- Emits `ExecutionEvent`s for position lifecycle (opened, closed, stop moved)

**Discord Notifications**:
- Sends rich embeds to Discord webhook for trade alerts
- Position entry: Shows symbol, direction, strategy, entry/stop/target prices
- Position exit: Shows PnL in both dollar amount and percentage
- Break-even stop moves and daily limit alerts
- Can be toggled on/off via config

## Configuration

All parameters in `config.toml`:
- **[general]**: Symbols to trade, log level
- **[range_bar]**: Range size per symbol (price movement threshold)
- **[volume_profile]**: Tick size, value area %, session reset hours
- **[order_flow]**: Absorption detection thresholds, large volume multipliers
- **[strategy]**: Enabled setups, distance thresholds, lookback periods
- **[risk]**: Initial balance, risk per trade, daily limits, stop/target defaults
- **[simulator]**: Slippage, fees, order book depth
- **[logging]**: Output file paths for trades
- **[discord]**: Enable/disable notifications, webhook URL

## Data Storage

### Database
- **SQLite** (`trades.db`): All position data persisted to disk
  - Table: `positions` (id, symbol, side, setup, prices, pnl, timestamps, etc.)
  - UPSERT logic supports position updates
  - Auto-created on first run

### In-Memory
- Real-time data (active positions, volume profiles, order flow metrics)

### Legacy Logs (backup)
- **CSV**: Trade log (`trades.csv`)
- **JSON**: Trade log (`trades.json`)

## Environment Variables

Required in `.env` file:
- `DISCORD_WEBHOOK_URL`: Discord webhook URL for notifications

## Important Implementation Notes

- Use `rust_decimal::Decimal` for all price/quantity calculations (never `f64`)
- All timestamps are `chrono::DateTime<Utc>`
- Trade IDs and position IDs use UUID v4
- Range bar `footprint` keys are stringified prices (for BTreeMap ordering)
- Shutdown is graceful: `Ctrl+C` → watch channel → all tasks exit cleanly
- Order flow tracker and volume profiler maintain per-symbol state
- Strategy engine keeps recent 100 bars per symbol for lookback analysis
- SQLite database is thread-safe via `Arc<Mutex<Connection>>`
