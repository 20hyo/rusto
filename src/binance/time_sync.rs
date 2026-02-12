use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

#[derive(Debug, Deserialize)]
struct ServerTime {
    #[serde(rename = "serverTime")]
    server_time: i64,
}

/// Network latency and time synchronization statistics
#[derive(Debug, Clone)]
pub struct NetworkStats {
    pub avg_latency_ms: f64,
    pub max_latency_ms: f64,
    pub min_latency_ms: f64,
    pub time_offset_ms: i64,
    pub samples: usize,
}

/// Checks time synchronization with Binance Futures API
pub struct TimeSyncChecker {
    client: Client,
    base_url: String,
    max_time_offset_ms: i64,
    max_latency_ms: f64,
    ping_samples: usize,
}

impl TimeSyncChecker {
    pub fn new(
        base_url: String,
        max_time_offset_ms: i64,
        max_latency_ms: f64,
        ping_samples: usize,
    ) -> Self {
        Self {
            client: Client::new(),
            base_url,
            max_time_offset_ms,
            max_latency_ms,
            ping_samples,
        }
    }

    /// Perform full network and time synchronization check
    pub async fn check(&self) -> Result<NetworkStats, String> {
        info!("Starting Binance time synchronization check...");

        // 1. Measure RTT (Round Trip Time)
        let rtt_stats = self.measure_rtt().await?;

        // 2. Check time offset
        let time_offset = self.check_time_offset().await?;

        let stats = NetworkStats {
            avg_latency_ms: rtt_stats.0,
            max_latency_ms: rtt_stats.1,
            min_latency_ms: rtt_stats.2,
            time_offset_ms: time_offset,
            samples: self.ping_samples,
        };

        // Validate results
        if stats.time_offset_ms.abs() > self.max_time_offset_ms {
            error!(
                "Time offset too large: {}ms (max: {}ms)",
                stats.time_offset_ms, self.max_time_offset_ms
            );
            return Err(format!(
                "Time offset {}ms exceeds maximum {}ms. Please sync your system clock.",
                stats.time_offset_ms, self.max_time_offset_ms
            ));
        }

        if stats.avg_latency_ms > self.max_latency_ms {
            warn!(
                "Average latency {}ms exceeds recommended maximum {}ms",
                stats.avg_latency_ms, self.max_latency_ms
            );
        }

        info!(
            "Time sync check passed: offset={}ms, avg_latency={:.2}ms, max_latency={:.2}ms",
            stats.time_offset_ms, stats.avg_latency_ms, stats.max_latency_ms
        );

        Ok(stats)
    }

    /// Measure RTT by pinging /fapi/v1/ping multiple times
    async fn measure_rtt(&self) -> Result<(f64, f64, f64), String> {
        let ping_url = format!("{}/fapi/v1/ping", self.base_url);
        let mut latencies = Vec::new();

        info!(
            "Measuring RTT with {} samples to {}...",
            self.ping_samples, ping_url
        );

        for i in 0..self.ping_samples {
            let start = Instant::now();

            match self.client.get(&ping_url).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        let elapsed = start.elapsed();
                        let latency_ms = elapsed.as_secs_f64() * 1000.0;
                        latencies.push(latency_ms);

                        if i == 0 {
                            info!("First ping successful: {:.2}ms", latency_ms);
                        }
                    } else {
                        warn!("Ping failed with status: {}", response.status());
                    }
                }
                Err(e) => {
                    error!("Ping request failed: {}", e);
                    return Err(format!("Failed to ping Binance API: {}", e));
                }
            }

            // Small delay between pings
            if i < self.ping_samples - 1 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }

        if latencies.is_empty() {
            return Err("No successful ping responses".to_string());
        }

        let avg = latencies.iter().sum::<f64>() / latencies.len() as f64;
        let max = latencies.iter().cloned().fold(f64::MIN, f64::max);
        let min = latencies.iter().cloned().fold(f64::MAX, f64::min);

        Ok((avg, max, min))
    }

    /// Check time offset between local and Binance server
    async fn check_time_offset(&self) -> Result<i64, String> {
        let time_url = format!("{}/fapi/v1/time", self.base_url);

        info!("Checking time offset with Binance server...");

        let local_before = Utc::now().timestamp_millis();

        let response = self
            .client
            .get(&time_url)
            .send()
            .await
            .map_err(|e| format!("Failed to get server time: {}", e))?;

        let local_after = Utc::now().timestamp_millis();

        if !response.status().is_success() {
            return Err(format!(
                "Server time request failed with status: {}",
                response.status()
            ));
        }

        let server_time: ServerTime = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse server time: {}", e))?;

        // Estimate local time at moment of server response
        let local_estimate = (local_before + local_after) / 2;
        let offset = server_time.server_time - local_estimate;

        info!(
            "Server time: {}, Local time: {}, Offset: {}ms",
            server_time.server_time, local_estimate, offset
        );

        Ok(offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_time_sync() {
        let checker = TimeSyncChecker::new(
            "https://fapi.binance.com".to_string(),
            500,
            15.0,
            5,
        );

        match checker.check().await {
            Ok(stats) => {
                println!("Time sync successful: {:?}", stats);
                assert!(stats.time_offset_ms.abs() <= 500);
            }
            Err(e) => {
                println!("Time sync failed: {}", e);
            }
        }
    }
}
