# Rusto - Order Flow Trading Bot

Rust로 작성된 비동기 오더플로우 트레이딩 봇입니다. Binance WebSocket을 통해 실시간 시장 데이터를 수신하고, Range Bar 차트와 Volume Profile, Order Flow 분석을 통해 매매 신호를 생성하며, 가상 매매를 시뮬레이션합니다.

## 주요 기능

### 📊 차트 분석
- **Range Bars**: 시간이 아닌 가격 움직임 기반 차트
- **Volume Profile**: POC (Point of Control), VAH/VAL (Value Area) 계산
- **Order Flow**: CVD (누적 거래량 델타), 흡수 패턴 감지

### 🎯 매매 전략
1. **AAA (Absorption At Area)**: VAL/VAH에서 흡수 감지 후 반대 방향 진입
2. **Momentum Squeeze**: 세션 고점/저점 돌파 + 델타 확인
3. **Absorption Reversal**: 순수 흡수 패턴 기반 역추세

### 🛡️ 리스크 관리
- 포지션 사이즈 자동 계산
- 손익분기점 자동 이동
- 일일 손실 한도 관리
- 동시 포지션 수 제한

### 💾 데이터 저장
- **SQLite**: 모든 포지션 데이터 영구 저장
- **CSV/JSON**: 백업 로그

### 📢 Discord 알림
- 포지션 진입/청산 알림
- 손익률 자동 계산 및 표시
- 손익분기점 이동 알림

## 빠른 시작

### 1. 의존성 설치
```bash
# Rust 설치 (아직 설치하지 않았다면)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. 환경 변수 설정
```bash
# .env 파일 생성
cp .env.example .env

# .env 파일 편집하여 Discord Webhook URL 설정
# DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/YOUR_WEBHOOK_ID/YOUR_WEBHOOK_TOKEN
```

### 3. 설정 파일 편집
`config.toml`에서 매매할 심볼과 전략 설정:

```toml
[general]
symbols = ["btcusdt", "ethusdt"]
log_level = "info"

[discord]
enabled = true  # Discord 알림 활성화
```

### 4. 실행
```bash
cargo run --release
```

## 설정 가이드

### Range Bar 설정
심볼별로 다른 Range 크기 지정 가능:
```toml
[range_bar]
btcusdt = 50.0   # BTC는 50 USDT 움직임마다 바 생성
ethusdt = 3.0    # ETH는 3 USDT 움직임마다 바 생성
default = 10.0   # 기본값
```

### 전략 설정
```toml
[strategy]
enabled_setups = ["AAA", "MomentumSqueeze", "AbsorptionReversal"]
aaa_poc_distance_ticks = 5
momentum_lookback_bars = 20
min_delta_confirmation = 1.5
```

### 리스크 설정
```toml
[risk]
initial_balance = 10000.0
max_risk_per_trade = 0.01        # 거래당 1% 리스크
daily_loss_limit_pct = 0.03      # 일일 손실 한도 3%
max_concurrent_positions = 3      # 최대 동시 포지션 수
break_even_ticks = 3             # 3틱 이익 후 손익분기점 이동
default_stop_ticks = 10          # 기본 손절 거리
default_target_multiplier = 2.0  # 목표가 배수
```

## Discord Webhook 설정

1. Discord 서버 설정 → 연동 → 웹후크
2. 새 웹후크 생성
3. Webhook URL 복사
4. `.env` 파일에 추가:
```bash
DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/...
```

## 개발

### 빌드
```bash
cargo build              # 디버그 빌드
cargo build --release    # 릴리스 빌드
```

### 테스트
```bash
cargo test               # 모든 테스트 실행
cargo test <test_name>   # 특정 테스트 실행
```

### 코드 품질
```bash
cargo clippy             # 린트
cargo fmt                # 포맷
```

### 로그 레벨 설정
```bash
RUST_LOG=debug cargo run     # 디버그 레벨
RUST_LOG=info cargo run      # 인포 레벨 (기본값)
```

## 데이터베이스

SQLite 데이터베이스는 `trades.db`에 자동으로 생성됩니다.

### 포지션 테이블 스키마
```sql
CREATE TABLE positions (
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
);
```

### 데이터 조회 예시
```bash
sqlite3 trades.db "SELECT symbol, pnl, entry_time FROM positions WHERE status = 'Closed' ORDER BY entry_time DESC LIMIT 10;"
```

## 아키텍처

```
Binance WebSocket
      ↓
  MarketEvent (broadcast channel)
      ↓
Processing Pipeline
  - Volume Profiler
  - Range Bar Builder
  - Order Flow Tracker
  - Strategy Engine
      ↓
  TradeSignal (mpsc channel)
      ↓
Simulator Engine
  - Risk Manager
  - Position Manager
  - Order Book Simulator
      ↓
  ExecutionEvent (mpsc channel)
      ↓
Discord Bot → Webhook
Trade Logger → SQLite/CSV/JSON
```

4개의 독립적인 비동기 태스크:
1. **WebSocket Task**: 시장 데이터 수신
2. **Processing Task**: 분석 및 신호 생성
3. **Simulator Task**: 매매 시뮬레이션
4. **Discord Task**: 알림 전송

## 라이선스

MIT

## 주의사항

⚠️ **이 봇은 가상 매매(시뮬레이션) 전용입니다.**
실제 거래소 계정과 연결되지 않으며, 실제 주문을 전송하지 않습니다.
실제 매매에 사용하기 전에 충분한 백테스팅과 검증이 필요합니다.
