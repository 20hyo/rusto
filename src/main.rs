use rusto::config::AppConfig;
use rusto::discord::DiscordBot;
use rusto::market_data::BinanceWebSocket;
use rusto::order_flow::OrderFlowTracker;
use rusto::range_bar::RangeBarBuilder;
use rusto::risk::RiskManager;
use rusto::simulator::trade_log::TradeLogger;
use rusto::simulator::SimulatorEngine;
use rusto::strategy::StrategyEngine;
use rusto::types::{ExecutionEvent, MarketEvent, ProcessingEvent};
use rusto::volume_profile::VolumeProfiler;
use tokio::sync::{broadcast, mpsc, watch};
use tracing::info;

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
    info!("Symbols: {:?}", config.general.symbols);

    // Channels
    let (market_tx, _) = broadcast::channel::<MarketEvent>(10_000);
    let (processing_tx, processing_rx) = mpsc::channel::<ProcessingEvent>(1_000);
    let (execution_tx, execution_rx) = mpsc::channel::<ExecutionEvent>(1_000);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Market data feed
    let ws = BinanceWebSocket::new(config.general.symbols.clone(), market_tx.clone());
    let ws_shutdown = shutdown_rx.clone();

    // Processing components
    let mut range_bar_builder = RangeBarBuilder::new(config.range_bar.clone());
    let mut volume_profiler = VolumeProfiler::new(&config.volume_profile);
    let mut order_flow_tracker = OrderFlowTracker::new(&config.order_flow);
    let mut strategy_engine =
        StrategyEngine::new(config.strategy.clone(), config.risk.clone());

    let mut market_rx_processing = market_tx.subscribe();
    let processing_shutdown = shutdown_rx.clone();
    let processing_tx_clone = processing_tx.clone();

    // Simulator engine
    let risk_manager = RiskManager::new(&config.risk);
    let trade_logger = TradeLogger::new(
        config.logging.trades_csv_path.clone(),
        config.logging.trades_json_path.clone(),
        config.logging.trades_db_path.clone(),
    );
    let mut simulator = SimulatorEngine::new(config.simulator.clone(), risk_manager, trade_logger);
    simulator.set_execution_channel(execution_tx.clone());
    let market_rx_simulator = market_tx.subscribe();
    let sim_shutdown = shutdown_rx.clone();

    // Discord bot (optional)
    let discord_handle = if config.discord.enabled {
        match config.discord.webhook_url() {
            Ok(webhook_url) => {
                let discord_bot = DiscordBot::new(webhook_url);
                let discord_shutdown = shutdown_rx.clone();
                info!("Discord notifications enabled");

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
        let _ = tokio::join!(ws_handle, processing_handle, sim_handle, discord_handle);
    } else {
        let _ = tokio::join!(ws_handle, processing_handle, sim_handle);
    }

    info!("Rusto shut down cleanly.");
    Ok(())
}
