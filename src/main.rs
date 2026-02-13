use rusto::binance::{ExchangeInfoManager, TimeSyncChecker};
use rusto::config::AppConfig;
use rusto::discord::DiscordBot;
use rusto::market_data::BinanceWebSocket;
use rusto::order_flow::OrderFlowTracker;
use rusto::range_bar::RangeBarBuilder;
use rusto::risk::RiskManager;
use rusto::simulator::trade_log::TradeLogger;
use rusto::simulator::SimulatorEngine;
use rusto::strategy::StrategyEngine;
use rusto::types::{BotStats, ExecutionEvent, MarketEvent, ProcessingEvent};
use rusto::volume_profile::VolumeProfiler;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Load config
    let config = AppConfig::load("config.toml").unwrap_or_else(|e| {
        eprintln!("Failed to load config: {}", e);
        std::process::exit(1);
    });

    // Initialize tracing
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.general.log_level));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .init();

    info!("Rusto - Order Flow Trading Bot starting...");
    if config.general.auto_select_symbols {
        info!("Mode: Auto-select top {} symbols by volume", config.general.top_n_symbols);
    } else {
        info!("Symbols: {:?}", config.general.symbols);
    }

    // === Binance Pre-flight Checks ===
    info!("Running Binance pre-flight checks...");

    // 1. Time synchronization check
    let time_checker = TimeSyncChecker::new(
        config.binance.api_url.clone(),
        config.binance.max_time_offset_ms,
        config.binance.max_latency_ms,
        config.binance.ping_samples,
    );

    let network_stats = match time_checker.check().await {
        Ok(stats) => {
            info!(
                "✓ Time sync OK: offset={}ms, latency={:.2}ms (max: {:.2}ms)",
                stats.time_offset_ms, stats.avg_latency_ms, stats.max_latency_ms
            );
            stats
        }
        Err(e) => {
            error!("✗ Time sync failed: {}", e);
            eprintln!("\n❌ Time synchronization check failed!");
            eprintln!("   {}", e);
            eprintln!("\n   Please ensure:");
            eprintln!("   1. Your system clock is synchronized (use NTP)");
            eprintln!("   2. Your network connection to Binance is stable");
            eprintln!("   3. Check your system time with: date");
            std::process::exit(1);
        }
    };

    // 2. Exchange info sync (symbol filters)
    let mut exchange_info = ExchangeInfoManager::new(config.binance.api_url.clone());

    match exchange_info.sync().await {
        Ok(_) => {
            info!("✓ Exchange info synced: {} symbols loaded", exchange_info.symbols().len());
        }
        Err(e) => {
            error!("✗ Exchange info sync failed: {}", e);
            eprintln!("\n❌ Failed to fetch exchange information from Binance!");
            eprintln!("   {}", e);
            std::process::exit(1);
        }
    }

    // 3. Determine symbols: auto-select top-N or use config
    // symbol_prices: map of symbol → last price (used for dynamic range calculation)
    // auto_select_warning: holds an error message to send to Discord if auto-selection failed
    let mut auto_select_warning: Option<String> = None;
    let (symbols, symbol_prices): (Vec<String>, std::collections::HashMap<String, rust_decimal::Decimal>) =
        if config.general.auto_select_symbols {
            match exchange_info.fetch_top_symbols(config.general.top_n_symbols).await {
                Ok(top) if !top.is_empty() => {
                    let syms: Vec<String> = top.iter().map(|(s, _)| s.clone()).collect();
                    let prices: std::collections::HashMap<String, rust_decimal::Decimal> =
                        top.into_iter().collect();
                    info!("✓ Auto-selected {} symbols by volume", syms.len());
                    (syms, prices)
                }
                Ok(_) => {
                    let msg = "fetch_top_symbols returned empty list — no USDT futures matched TRADING filter".to_string();
                    error!("✗ Auto-selection returned 0 symbols: {}", msg);
                    auto_select_warning = Some(msg);
                    (config.general.symbols.clone(), std::collections::HashMap::new())
                }
                Err(e) => {
                    let msg = format!("fetch_top_symbols failed: {}", e);
                    error!("✗ {}", msg);
                    eprintln!("\n❌ Auto symbol selection failed, falling back to config symbols\n   {}", e);
                    auto_select_warning = Some(msg);
                    (config.general.symbols.clone(), std::collections::HashMap::new())
                }
            }
        } else {
            (config.general.symbols.clone(), std::collections::HashMap::new())
        };

    // Validate all symbols against exchange info
    for symbol in &symbols {
        match exchange_info.get_symbol_info(symbol) {
            Some(info) => {
                info!(
                    symbol = %symbol,
                    tick_size = %info.price_tick_size,
                    step_size = %info.quantity_step_size,
                    min_notional = %info.min_notional,
                    "✓ Symbol validated"
                );
            }
            None => {
                error!("✗ Symbol {} not found in exchange info", symbol);
                eprintln!("\n❌ Symbol {} is not available on Binance Futures!", symbol);
                std::process::exit(1);
            }
        }
    }

    info!("✓ All pre-flight checks passed ({} symbols)", symbols.len());

    // Wrap exchange info in Arc for sharing
    let exchange_info = std::sync::Arc::new(exchange_info);

    // Channels
    let (market_tx, _) = broadcast::channel::<MarketEvent>(10_000);
    let (processing_tx, processing_rx) = mpsc::channel::<ProcessingEvent>(1_000);
    let (execution_tx, execution_rx) = mpsc::channel::<ExecutionEvent>(1_000);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Market data feed
    let ws = BinanceWebSocket::new(symbols.clone(), market_tx.clone());
    let ws_shutdown = shutdown_rx.clone();

    // Processing components
    let mut range_bar_builder = RangeBarBuilder::new(config.range_bar.clone());
    let mut volume_profiler = VolumeProfiler::new(&config.volume_profile);

    // Set per-symbol range bar sizes and volume profile tick sizes
    for symbol in &symbols {
        if let Some(sym_info) = exchange_info.get_symbol_info(symbol) {
            // Dynamic range bar size: use price if available
            if let Some(&price) = symbol_prices.get(symbol) {
                let range = config.range_bar.range_for_with_price(symbol, price);
                range_bar_builder.set_range(symbol, range);
                info!(symbol = %symbol, range = %range, price = %price, "Range bar size set");
            }
            // Per-symbol VP tick size = exchange tick_size × multiplier
            let vp_tick = sym_info.price_tick_size * rust_decimal::Decimal::from(config.volume_profile.tick_multiplier);
            volume_profiler.set_tick_size(symbol, vp_tick);
            info!(symbol = %symbol, vp_tick = %vp_tick, "Volume profile tick size set");
        }
    }
    let mut order_flow_tracker = OrderFlowTracker::new(&config.order_flow);
    let mut strategy_engine =
        StrategyEngine::new(config.strategy.clone(), config.risk.clone());

    let mut market_rx_processing = market_tx.subscribe();
    let processing_shutdown = shutdown_rx.clone();
    let processing_tx_clone = processing_tx.clone();

    // Simulator engine
    let leverage = rust_decimal::Decimal::try_from(config.simulator.leverage)
        .unwrap_or(rust_decimal::Decimal::from(100));
    let risk_manager = RiskManager::new(&config.risk, leverage);
    let trade_logger = TradeLogger::new(
        config.logging.trades_csv_path.clone(),
        config.logging.trades_json_path.clone(),
        config.logging.trades_db_path.clone(),
    );
    let mut simulator = SimulatorEngine::new(config.simulator.clone(), risk_manager, trade_logger);
    simulator.set_execution_channel(execution_tx.clone());
    simulator.set_exchange_info(exchange_info.clone());

    // Shared state between simulator and hourly reporter
    let bot_stats = Arc::new(Mutex::new(BotStats::default()));
    simulator.set_bot_stats(bot_stats.clone());
    let market_rx_simulator = market_tx.subscribe();
    let sim_shutdown = shutdown_rx.clone();

    // Discord bot (optional)
    let discord_handle = if config.discord.enabled {
        match config.discord.webhook_url() {
            Ok(webhook_url) => {
                let discord_bot = DiscordBot::new(webhook_url);
                let discord_shutdown = shutdown_rx.clone();
                info!("Discord notifications enabled");

                // Send startup message with network stats
                info!("Sending startup notification to Discord...");
                discord_bot.send_startup_message(&network_stats, &symbols).await;

                // Warn if auto-selection fell back to config symbols
                if let Some(ref warn_msg) = auto_select_warning {
                    discord_bot.send_warning(
                        "⚠️ 심볼 자동 선택 실패 — config 심볼로 fallback",
                        &format!("**오류**: {}\n\n**fallback 심볼**: {}\n\n> EC2 로그에서 상세 원인 확인:\n> `sudo journalctl -u rusto -n 50 --no-pager`",
                            warn_msg,
                            symbols.iter().map(|s| s.to_uppercase()).collect::<Vec<_>>().join(", ")
                        ),
                    ).await;
                }

                Some(tokio::spawn(async move {
                    discord_bot.run(execution_rx, discord_shutdown).await;
                }))
            }
            Err(e) => {
                eprintln!("Discord enabled but webhook URL not configured: {}", e);
                eprintln!("Please set DISCORD_WEBHOOK_URL in .env file");
                std::process::exit(1);
            }
        }
    } else {
        info!("Discord notifications disabled");
        None
    };

    // Spawn hourly reporter task (independent of market-data loop)
    let hourly_execution_tx = execution_tx.clone();
    let hourly_stats = bot_stats.clone();
    let hourly_ping_url = format!("{}/fapi/v1/ping", config.binance.api_url);
    let hourly_shutdown = shutdown_rx.clone();
    let hourly_handle = tokio::spawn(async move {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        // Wait until the next whole-hour boundary (:00)
        let now = chrono::Utc::now();
        let secs_past_hour = (now.timestamp() % 3600) as u64;
        let secs_until_next = if secs_past_hour == 0 { 3600 } else { 3600 - secs_past_hour };
        info!("Hourly reporter: first report in {}s (next :00)", secs_until_next);

        let start = tokio::time::Instant::now()
            + tokio::time::Duration::from_secs(secs_until_next);
        let mut timer = tokio::time::interval_at(start, tokio::time::Duration::from_secs(3600));
        let mut shutdown = hourly_shutdown;

        loop {
            tokio::select! {
                _ = timer.tick() => {
                    // Ping Binance with timeout
                    let ping_ms = {
                        let t = std::time::Instant::now();
                        match http_client.get(&hourly_ping_url).send().await {
                            Ok(_) => t.elapsed().as_secs_f64() * 1000.0,
                            Err(e) => {
                                warn!("Hourly ping failed: {}", e);
                                -1.0
                            }
                        }
                    };

                    let (balance, daily_pnl, open_positions, total_trades, symbol_stats) = {
                        let s = hourly_stats.lock().unwrap();
                        (s.balance, s.daily_pnl, s.open_positions, s.total_trades, s.symbol_stats.clone())
                    };

                    info!(
                        balance = %balance,
                        daily_pnl = %daily_pnl,
                        open_positions = open_positions,
                        total_trades = total_trades,
                        ping_ms = ping_ms,
                        "Hourly report"
                    );

                    let _ = hourly_execution_tx.send(ExecutionEvent::HourlyReport {
                        balance,
                        daily_pnl,
                        open_positions,
                        ping_ms,
                        total_trades,
                        symbol_stats,
                    }).await;
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Hourly reporter shutting down");
                        return;
                    }
                }
            }
        }
    });

    // Spawn WebSocket task
    let ws_handle = tokio::spawn(async move {
        ws.run(ws_shutdown).await;
    });

    // Spawn processing task
    let processing_handle = tokio::spawn(async move {
        let mut shutdown = processing_shutdown;
        info!("Processing pipeline started");

        loop {
            tokio::select! {
                Ok(event) = market_rx_processing.recv() => {
                    match event {
                        MarketEvent::Trade(ref trade) => {
                            // 1. Update volume profile
                            if let Some(vp) = volume_profiler.process_trade(trade) {
                                strategy_engine.update_profile(vp.clone());
                                let _ = processing_tx_clone.send(ProcessingEvent::VolumeProfile(vp)).await;
                            }

                            // 2. Build range bars
                            if let Some(bar) = range_bar_builder.process_trade(trade) {
                                // 3. Analyze order flow
                                let flow = order_flow_tracker.analyze_bar(&bar);
                                strategy_engine.update_flow(flow.clone());
                                let _ = processing_tx_clone.send(ProcessingEvent::OrderFlow(flow)).await;

                                // 4. Generate signals
                                let signals = strategy_engine.process_bar(&bar);
                                let _ = processing_tx_clone.send(ProcessingEvent::NewBar(bar)).await;

                                for signal in signals {
                                    info!(
                                        symbol = %signal.symbol,
                                        setup = %signal.setup,
                                        side = ?signal.side,
                                        entry = %signal.entry_price,
                                        "Signal generated"
                                    );
                                    let _ = processing_tx_clone.send(ProcessingEvent::Signal(signal)).await;
                                }
                            }
                        }
                        MarketEvent::Depth(_) => {
                            // Depth handled by simulator directly
                        }
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Processing pipeline shutting down");
                        return;
                    }
                }
            }
        }
    });

    // Spawn simulator task
    let sim_handle = tokio::spawn(async move {
        simulator
            .run(processing_rx, market_rx_simulator, sim_shutdown)
            .await;
    });

    // Wait for Ctrl+C
    info!("Bot running. Press Ctrl+C to stop.");
    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received...");
    let _ = shutdown_tx.send(true);

    // Wait for all tasks to complete
    if let Some(discord_handle) = discord_handle {
        let _ = tokio::join!(ws_handle, processing_handle, sim_handle, discord_handle, hourly_handle);
    } else {
        let _ = tokio::join!(ws_handle, processing_handle, sim_handle, hourly_handle);
    }

    info!("Rusto shut down cleanly.");
    Ok(())
}
