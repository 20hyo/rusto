# Rusto 매매 로직 설명서 (운영 기준)

## 1. 전체 파이프라인
1. Binance 선물 `aggTrade/depth` 수신
2. Range Bar 생성
3. Order Flow(CVD/흡수/불균형/거래량 burst) 계산
4. Volume Profile(VAL/VAH/VWAP/HVN) 갱신
5. StrategyEngine이 신호 생성
6. SimulatorEngine이 진입 전 필터링(집행 품질/expectancy/슬리피지)
7. 리스크 검사 후 포지션 오픈
8. 실시간으로 TP/SL/SoftStop/청산/브레이크이븐 관리
9. 체결/성과/피처를 SQLite에 저장

---

## 2. 심볼 선정
- 자동 모드에서 Binance 선물 USDT 거래대금 상위 10개 심볼 사용
- KST 09:00 기준 재선정 스케줄이 동작하며, 재시작 후 새 top-10 반영

---

## 3. 진입 전략 (AdvancedOrderFlow)
기본 조건:
- Zone: VAL/VAH/HVN 근접
- CVD 급변
- 흡수 패턴 감지
- 매수/매도 불균형 비율 조건
- 최소 바 변동률 조건
- 심볼별 거래량 burst 조건

### 3.1 심볼별 burst 자동 튜닝
- 최근 샘플 롤링 백테스트로 후보 burst 임계치들을 평가
- 기대값(Expectancy) 최대 임계치를 심볼별로 선택
- 결과는 `volume_burst_tuning_logs` 테이블에 기록

### 3.2 레짐(시장상태) 자동 전환
- 최근 바를 기반으로 `추세/횡보` + `고변동/저변동` 분류
- 분류 결과에 따라 아래 파라미터를 동적 조정:
  - 최소 imbalance
  - 최소 CVD 변화량
  - 최소 바 변동률
  - 최소 burst 비율
  - signal cooldown bars

---

## 4. 진입 전 실행 필터 (Execution Gate)
포지션 오픈 직전에 3단계 필터를 통과해야 진입:

1. 오더북 품질 필터
- 오더북 데이터 존재 여부
- 스프레드(bps)가 허용치 이내
- 진입 방향에 유리한 depth imbalance 충족

2. 시간대(UTC hour) expectancy 필터
- `symbol + UTC hour` 단위의 최근 성과 평균(PnL)을 추적
- 샘플 수가 충분하고 평균 PnL이 기준보다 낮으면 진입 차단

3. 슬리피지 모델 필터
- `half spread + (수량/상위 depth) 기반 impact`로 예상 슬리피지 계산
- 예상 슬리피지(bps)가 상한 초과 시 진입 취소

---

## 5. 포지션 관리 / 청산
AdvancedOrderFlow 기준:
- TP1 도달 시 50% 부분청산
- TP1 이후 보호성 스탑(본절+알파)으로 이동
- TP2 도달 시 잔량 청산
- 일정 시간 이후 의미 있는 역행 시 SoftStop
- 강제청산 가격 도달 시 Liquidation

공통:
- 브레이크이븐 이동은 최소 보유시간 + RR 조건 충족 필요
- 신뢰도 기반 사이징 적용 가능
- 심볼별 연속 손실 누적 시 cooldown 동안 신규 진입 차단

---

## 6. 저장되는 데이터 (SQLite)
`positions`
- 진입/청산 가격, 수량, 손익, 상태
- `exit_reason` (StopLoss/TakeProfit/TP2/SoftStop/Liquidation)
- `mfe_pct`, `mae_pct`, `time_to_mfe_secs`, `time_to_mae_secs`

`entry_features`
- 진입 시점 피처:
  - imbalance_ratio
  - cvd_1min_change
  - volume_burst_ratio
  - bar_range_pct
  - zone_distance_pct
  - near_val/near_vah/near_hvn

`volume_burst_tuning_logs`
- 심볼별 burst 튜닝 결과:
  - tuned_ratio
  - trades
  - win_rate_pct
  - expectancy_pct
  - 백테스트 파라미터 및 변경 여부

---

## 7. 운영/개선 루프 권장
1. `entry_features + positions`로 기대값 음수 구간 식별
2. UTC hour / regime / symbol 단위로 필터 임계치 재조정
3. SoftStop/TP1/TP2 규칙의 MFE/MAE 적합도 점검
4. A/B 파라미터를 소액/페이퍼에서 먼저 검증 후 본운영 반영
