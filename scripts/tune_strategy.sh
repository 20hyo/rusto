#!/usr/bin/env bash
set -euo pipefail

# Strategy tuning loop for paper-trading runs.
# Runs multiple parameter sets for a fixed duration and collects:
# - Win rate
# - Profit factor
# - Max drawdown (MDD)
#
# Usage:
#   ./scripts/tune_strategy.sh [run_seconds]
#
# Example:
#   ./scripts/tune_strategy.sh 1800

RUN_SECONDS="${1:-900}"
CONFIG_FILE="config.toml"
BACKUP_FILE="config.toml.tune.bak"
RESULTS_DB="tuning_results.db"

if [[ ! -f "$CONFIG_FILE" ]]; then
  echo "config.toml not found in current directory"
  exit 1
fi

if ! command -v timeout >/dev/null 2>&1; then
  echo "timeout command not found. Install coreutils (GNU timeout) first."
  exit 1
fi

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "sqlite3 command not found."
  exit 1
fi

if [[ -f .env ]]; then
  # shellcheck disable=SC1091
  source .env
fi

if [[ -z "${DISCORD_WEBHOOK_URL:-}" ]]; then
  echo "DISCORD_WEBHOOK_URL is not set (.env or environment)."
  exit 1
fi

cp "$CONFIG_FILE" "$BACKUP_FILE"
cleanup() {
  mv "$BACKUP_FILE" "$CONFIG_FILE"
}
trap cleanup EXIT

set_key() {
  local key="$1"
  local value="$2"
  local tmp
  tmp="$(mktemp)"

  awk -v k="$key" -v v="$value" '
    BEGIN { done=0 }
    $0 ~ "^[[:space:]]*" k "[[:space:]]*=" {
      print k " = " v
      done=1
      next
    }
    { print }
    END {
      if (!done) {
        print k " = " v
      }
    }
  ' "$CONFIG_FILE" > "$tmp"
  mv "$tmp" "$CONFIG_FILE"
}

echo "Building release binary once..."
cargo build --release >/dev/null

sqlite3 "$RESULTS_DB" "
CREATE TABLE IF NOT EXISTS tuning_results (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_name TEXT NOT NULL,
  run_seconds INTEGER NOT NULL,
  advanced_min_imbalance_ratio REAL NOT NULL,
  advanced_min_cvd_1min_change REAL NOT NULL,
  advanced_zone_ticks INTEGER NOT NULL,
  advanced_cooldown_bars INTEGER NOT NULL,
  wr_pct REAL,
  pf REAL,
  mdd_pct REAL,
  mdd_abs REAL,
  trades INTEGER,
  pnl REAL,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);"

send_discord() {
  local text="$1"
  local escaped
  escaped="$(printf '%s' "$text" | sed 's/\\/\\\\/g; s/"/\\"/g' | awk '{printf "%s\\n",$0}' | sed 's/\\n$//')"
  curl -sS -H "Content-Type: application/json" \
    -X POST \
    -d "{\"content\":\"${escaped}\"}" \
    "$DISCORD_WEBHOOK_URL" >/dev/null
}

# name|advanced_min_imbalance_ratio|advanced_min_cvd_1min_change|advanced_zone_ticks|advanced_cooldown_bars
PARAM_SETS=(
  "baseline|2.2|12.0|3|8"
  "tight_a|2.6|14.0|3|10"
  "tight_b|2.8|16.0|2|12"
  "balanced|2.4|13.0|3|9"
)

for row in "${PARAM_SETS[@]}"; do
  IFS="|" read -r name imbalance cvd zone cooldown <<< "$row"

  echo ""
  echo "=== Running ${name} ==="
  echo "imbalance=${imbalance}, cvd=${cvd}, zone=${zone}, cooldown=${cooldown}"

  set_key "advanced_min_imbalance_ratio" "$imbalance"
  set_key "advanced_min_cvd_1min_change" "$cvd"
  set_key "advanced_zone_ticks" "$zone"
  set_key "advanced_cooldown_bars" "$cooldown"

  rm -f trades.db trades.csv trades.json "tune_${name}.log"

  timeout --signal=INT --kill-after=15s "${RUN_SECONDS}s" \
    ./target/release/rusto > "tune_${name}.log" 2>&1 || true

  metrics_row="$(
    sqlite3 -csv trades.db "
      SELECT
        win_rate_pct,
        COALESCE(profit_factor, 0),
        max_drawdown_pct,
        max_drawdown_abs,
        total_trades,
        total_pnl
      FROM performance_metrics
      ORDER BY id DESC
      LIMIT 1;
    " 2>/dev/null || true
  )"

  if [[ -z "$metrics_row" ]]; then
    echo "No performance_metrics row found for ${name} (likely no finalized trades)."
    sqlite3 "$RESULTS_DB" "
      INSERT INTO tuning_results (
        run_name, run_seconds, advanced_min_imbalance_ratio, advanced_min_cvd_1min_change,
        advanced_zone_ticks, advanced_cooldown_bars, wr_pct, pf, mdd_pct, mdd_abs, trades, pnl
      ) VALUES (
        '${name}', ${RUN_SECONDS}, ${imbalance}, ${cvd}, ${zone}, ${cooldown},
        NULL, NULL, NULL, NULL, 0, 0
      );
    "
    send_discord "Tune result [${name}] | ${RUN_SECONDS}s | WR=NA PF=NA MDD=NA Trades=0 PnL=0"
    continue
  fi

  IFS="," read -r wr pf mdd_pct mdd_abs trades pnl <<< "$metrics_row"

  sqlite3 "$RESULTS_DB" "
    INSERT INTO tuning_results (
      run_name, run_seconds, advanced_min_imbalance_ratio, advanced_min_cvd_1min_change,
      advanced_zone_ticks, advanced_cooldown_bars, wr_pct, pf, mdd_pct, mdd_abs, trades, pnl
    ) VALUES (
      '${name}', ${RUN_SECONDS}, ${imbalance}, ${cvd}, ${zone}, ${cooldown},
      ${wr}, ${pf}, ${mdd_pct}, ${mdd_abs}, ${trades}, ${pnl}
    );
  "
  send_discord "Tune result [${name}] | ${RUN_SECONDS}s | WR=${wr}% PF=${pf} MDD=${mdd_pct}% Trades=${trades} PnL=${pnl}"
done

best_row="$(
  sqlite3 -csv "$RESULTS_DB" "
    SELECT run_name, wr_pct, pf, mdd_pct, trades, pnl
    FROM tuning_results
    WHERE wr_pct IS NOT NULL
    ORDER BY wr_pct DESC, pf DESC, mdd_pct ASC
    LIMIT 1;
  " 2>/dev/null || true
)"

if [[ -n "$best_row" ]]; then
  IFS="," read -r best_name best_wr best_pf best_mdd best_trades best_pnl <<< "$best_row"
  send_discord "Tuning complete. BEST=${best_name} | WR=${best_wr}% PF=${best_pf} MDD=${best_mdd}% Trades=${best_trades} PnL=${best_pnl}"
else
  send_discord "Tuning complete. No valid run with finalized trades."
fi

echo "SQLite results saved to ${RESULTS_DB} (table: tuning_results)"
