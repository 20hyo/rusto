use crate::config::SimulatorConfig;
use crate::risk::RiskManager;
use crate::simulator::order_book::LocalOrderBook;
use crate::simulator::position::PositionManager;
use crate::simulator::trade_log::TradeLogger;
use crate::types::{
    DepthUpdate, ExecutionEvent, MarketEvent, NormalizedTrade, ProcessingEvent, TradeSignal,
};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Paper trading execution engine
pub struct SimulatorEngine {
    config: SimulatorConfig,
    risk_manager: RiskManager,
    position_manager: PositionManager,
    trade_logger: TradeLogger,
    order_books: BTreeMap<String, LocalOrderBook>,
    fee_rate: Decimal,
    execution_tx: Option<mpsc::Sender<ExecutionEvent>>,
}

impl SimulatorEngine {
    pub fn new(
        config: SimulatorConfig,
        risk_manager: RiskManager,
        trade_logger: TradeLogger,
    ) -> Self {
        let fee_rate = Decimal::try_from(config.taker_fee).unwrap_or_else(|_| Decimal::new(4, 4));
        Self {
            config,
            risk_manager,
            position_manager: PositionManager::new(),
            trade_logger,
            order_books: BTreeMap::new(),
            fee_rate,
            execution_tx: None,
        }
    }

    pub fn set_execution_channel(&mut self, tx: mpsc::Sender<ExecutionEvent>) {
        self.execution_tx = Some(tx);
    }

    /// Main loop: consume processing events and market events
    pub async fn run(
        &mut self,
        mut processing_rx: mpsc::Receiver<ProcessingEvent>,
        mut market_rx: tokio::sync::broadcast::Receiver<MarketEvent>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!("Simulator engine started");

        loop {
            tokio::select! {
                // Processing events (signals)
                Some(event) = processing_rx.recv() => {
                    self.handle_processing_event(event);
                }
                // Market events (for position management)
                Ok(event) = market_rx.recv() => {
                    self.handle_market_event(event);
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Simulator engine shutting down");
                        self.shutdown_summary();
                        return;
                    }
                }
            }
        }
    }

    fn handle_processing_event(&mut self, event: ProcessingEvent) {
        match event {
            ProcessingEvent::Signal(signal) => {
                self.execute_signal(signal);
            }
            _ => {
                // Other events (bars, profiles, flow) handled by processing task
            }
        }
    }

    fn handle_market_event(&mut self, event: MarketEvent) {
        match event {
            MarketEvent::Trade(trade) => {
                self.on_trade(&trade);
            }
            MarketEvent::Depth(depth) => {
                self.on_depth(&depth);
            }
        }
    }

    fn execute_signal(&mut self, signal: TradeSignal) {
        if !self.risk_manager.can_trade(&signal) {
            warn!(
                symbol = %signal.symbol,
                setup = %signal.setup,
                "Signal rejected by risk manager"
            );
            return;
        }

        let quantity = self.risk_manager.calculate_position_size(&signal);
        if quantity <= Decimal::ZERO {
            warn!("Position size is zero, skipping");
            return;
        }

        let position = self.position_manager.open_position(&signal, quantity);
        self.risk_manager.register_position(&position);

        info!(
            id = %position.id,
            symbol = %position.symbol,
            side = ?position.side,
            setup = %position.setup,
            entry = %position.entry_price,
            stop = %position.stop_loss,
            target = %position.take_profit,
            qty = %position.quantity,
            "Position opened"
        );

        // Send execution event
        if let Some(tx) = &self.execution_tx {
            let _ = tx.try_send(ExecutionEvent::PositionOpened(position));
        }
    }

    fn on_trade(&mut self, trade: &NormalizedTrade) {
        // Check exits for open positions
        let closed = self.position_manager.check_exits(
            &trade.symbol,
            trade.price,
            self.fee_rate,
        );

        for position in &closed {
            self.risk_manager.close_position(position);
            self.trade_logger.log_trade(position);

            info!(
                id = %position.id,
                symbol = %position.symbol,
                pnl = %position.pnl,
                exit_price = %position.exit_price.unwrap_or_default(),
                "Position closed"
            );

            // Send execution event
            if let Some(tx) = &self.execution_tx {
                let _ = tx.try_send(ExecutionEvent::PositionClosed(position.clone()));
            }
        }

        // Check break-even moves
        let open_positions: Vec<_> = self
            .position_manager
            .open_positions_for(&trade.symbol)
            .into_iter()
            .map(|p| p.id.clone())
            .collect();

        for pos_id in open_positions {
            if let Some(pos) = self
                .position_manager
                .open_positions()
                .iter()
                .find(|p| p.id == pos_id)
            {
                if self
                    .risk_manager
                    .should_move_to_break_even(pos, trade.price)
                {
                    let entry_price = pos.entry_price; // Copy before mutable borrow
                    if self.position_manager.move_stop_to_break_even(&pos_id) {
                        info!(
                            position_id = %pos_id,
                            "Stop moved to break-even"
                        );

                        // Send execution event
                        if let Some(tx) = &self.execution_tx {
                            let _ = tx.try_send(ExecutionEvent::StopMoved {
                                position_id: pos_id.clone(),
                                new_stop: entry_price,
                            });
                        }
                    }
                }
            }
        }
    }

    fn on_depth(&mut self, depth: &DepthUpdate) {
        let book = self
            .order_books
            .entry(depth.symbol.clone())
            .or_insert_with(|| {
                LocalOrderBook::new(depth.symbol.clone(), self.config.order_book_depth)
            });
        book.update(depth);
    }

    fn shutdown_summary(&mut self) {
        let closed: Vec<_> = self
            .position_manager
            .closed_positions()
            .into_iter()
            .cloned()
            .collect();
        self.trade_logger.print_summary(&closed);

        info!(
            balance = %self.risk_manager.balance(),
            daily_pnl = %self.risk_manager.daily_pnl(),
            total_trades = closed.len(),
            "Final summary"
        );
    }
}
