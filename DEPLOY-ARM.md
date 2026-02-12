# EC2 ARM 배포 가이드 (t4g.small + Amazon Linux)

AWS Graviton2 (ARM) 기반 t4g.small 인스턴스에서 Rusto 트레이딩 봇을 배포하는 가이드입니다.

## 왜 ARM (t4g)?

- ✅ **비용 효율**: 같은 성능 대비 x86보다 20% 저렴
- ✅ **전력 효율**: 낮은 전력 소비
- ✅ **Rust 완벽 지원**: ARM 네이티브 빌드 지원

## EC2 인스턴스 설정

### 인스턴스 사양
- **AMI**: Amazon Linux 2023 ARM64
- **인스턴스 타입**: t4g.small
  - 2 vCPU (ARM Graviton2)
  - 2GB RAM
  - 최대 5 Gbps 네트워크
- **스토리지**: 20GB gp3
- **보안 그룹**:
  ```
  인바운드:
  - SSH (22) - 내 IP만 (예: 1.2.3.4/32)

  아웃바운드:
  - 모두 허용 (Binance WebSocket, Discord Webhook 필요)
  ```

### User Data 스크립트

EC2 인스턴스 생성 시 **"고급 세부 정보"** → **"사용자 데이터"**에 입력:

```bash
#!/bin/bash
curl -fsSL https://raw.githubusercontent.com/20hyo/fabio-trading/main/setup-ec2.sh | bash > /var/log/rusto-setup.log 2>&1
```

**중요**: 스크립트는 ARM과 x86 모두 자동 감지하여 지원합니다.

### 인스턴스 시작

1. "인스턴스 시작" 클릭
2. 약 5-10분 후 설치 완료

## 설정 단계

### 1. SSH 접속

```bash
# 키 권한 설정 (최초 1회)
chmod 400 your-key.pem

# SSH 접속 (Amazon Linux는 ec2-user 사용)
ssh -i your-key.pem ec2-user@<EC2-PUBLIC-IP>
```

### 2. 설치 상태 확인

```bash
# 설치 로그 실시간 확인
sudo tail -f /var/log/cloud-init-output.log

# 또는 설치 완료 확인
sudo cat /var/log/rusto-setup.log | tail -20
```

"Setup Complete" 메시지가 보이면 설치 완료!

### 3. Discord Webhook 설정

```bash
# .env 파일 편집
sudo -u rusto nano /home/rusto/fabio-trading/.env
```

다음 내용을 입력:
```bash
DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/YOUR_WEBHOOK_ID/YOUR_WEBHOOK_TOKEN
```

**저장**: `Ctrl+O` → `Enter` → `Ctrl+X`

### 4. 설정 파일 확인 (선택사항)

```bash
sudo -u rusto nano /home/rusto/fabio-trading/config.toml
```

원하는 심볼, 전략, 리스크 설정을 수정할 수 있습니다.

## 봇 실행

### 방법 1: systemd 서비스 (권장)

```bash
# 서비스 활성화 (부팅 시 자동 시작)
sudo systemctl enable rusto

# 서비스 시작
sudo systemctl start rusto

# 상태 확인
sudo systemctl status rusto
```

### 방법 2: 수동 실행 (테스트용)

```bash
# rusto 사용자로 전환
sudo su - rusto

# 실행
cd ~/fabio-trading
./target/release/rusto
```

종료: `Ctrl+C`

## 로그 확인

### 실시간 로그 모니터링

```bash
# systemd 로그
sudo journalctl -u rusto -f

# 또는 파일 로그
tail -f /home/rusto/fabio-trading/rusto.log
```

### 에러 로그

```bash
tail -f /home/rusto/fabio-trading/rusto.error.log
```

### 최근 100줄 보기

```bash
sudo journalctl -u rusto -n 100 --no-pager
```

## 봇 관리 명령어

```bash
# 상태 확인
sudo systemctl status rusto

# 시작
sudo systemctl start rusto

# 중지
sudo systemctl stop rusto

# 재시작
sudo systemctl restart rusto

# 자동 시작 활성화
sudo systemctl enable rusto

# 자동 시작 비활성화
sudo systemctl disable rusto
```

## 데이터베이스 조회

### SQLite 접속

```bash
sudo -u rusto sqlite3 /home/rusto/fabio-trading/trades.db
```

### 유용한 쿼리

```sql
-- 최근 10개 거래
SELECT symbol, side, setup, pnl, entry_time
FROM positions
WHERE status = 'Closed'
ORDER BY entry_time DESC
LIMIT 10;

-- 전체 통계
SELECT
  COUNT(*) as total_trades,
  SUM(CASE WHEN pnl > 0 THEN 1 ELSE 0 END) as wins,
  SUM(CASE WHEN pnl < 0 THEN 1 ELSE 0 END) as losses,
  ROUND(SUM(pnl), 2) as total_pnl,
  ROUND(AVG(pnl), 2) as avg_pnl
FROM positions
WHERE status = 'Closed';

-- 전략별 성과
SELECT
  setup,
  COUNT(*) as trades,
  ROUND(AVG(pnl), 2) as avg_pnl,
  ROUND(SUM(pnl), 2) as total_pnl
FROM positions
WHERE status = 'Closed'
GROUP BY setup;

-- 심볼별 성과
SELECT
  symbol,
  COUNT(*) as trades,
  ROUND(SUM(pnl), 2) as total_pnl
FROM positions
WHERE status = 'Closed'
GROUP BY symbol;

-- 종료
.exit
```

## 성능 모니터링

### CPU/메모리 사용량

```bash
# 실시간 모니터링
top

# rusto 프로세스만
top -p $(pgrep -f rusto)

# 메모리 상세
free -h

# 프로세스 상세 정보
ps aux | grep rusto
```

### 디스크 사용량

```bash
# 전체 디스크
df -h

# 프로젝트 디렉토리
du -sh /home/rusto/fabio-trading/*

# 데이터베이스 크기
ls -lh /home/rusto/fabio-trading/trades.db
```

### 네트워크 연결

```bash
# Binance WebSocket 연결 확인
sudo netstat -tnp | grep rusto

# 또는
sudo ss -tnp | grep rusto
```

## 업데이트

새 버전으로 업데이트:

```bash
# 봇 중지
sudo systemctl stop rusto

# 코드 업데이트
cd /home/rusto/fabio-trading
sudo -u rusto git pull

# ARM 네이티브 재빌드
sudo -u rusto bash -c 'source ~/.cargo/env && cargo build --release'

# 재시작
sudo systemctl start rusto

# 로그 확인
sudo journalctl -u rusto -f
```

## 백업

### 데이터베이스 백업 (로컬)

```bash
# 로컬로 다운로드
scp -i your-key.pem ec2-user@<EC2-IP>:/home/rusto/fabio-trading/trades.db ./trades-backup-$(date +%Y%m%d).db
```

### S3로 자동 백업 (선택사항)

```bash
# 인스턴스에서 실행
# 1. AWS CLI 설정 (IAM Role 권장)
# 2. 백업 스크립트 생성

# 백업 스크립트
cat > /home/rusto/backup.sh <<'EOF'
#!/bin/bash
aws s3 cp /home/rusto/fabio-trading/trades.db \
  s3://your-bucket/backups/trades-$(date +%Y%m%d-%H%M%S).db
EOF

chmod +x /home/rusto/backup.sh

# 크론탭 설정 (매일 자정)
sudo crontab -u rusto -e
# 다음 라인 추가:
# 0 0 * * * /home/rusto/backup.sh
```

## 문제 해결

### 봇이 시작되지 않음

```bash
# 1. 에러 로그 확인
sudo journalctl -u rusto -n 50 --no-pager

# 2. 설정 파일 확인
sudo -u rusto cat /home/rusto/fabio-trading/.env
sudo -u rusto cat /home/rusto/fabio-trading/config.toml

# 3. 수동 실행으로 테스트
sudo su - rusto
cd ~/fabio-trading
./target/release/rusto
```

### WebSocket 연결 실패

```bash
# 1. 네트워크 연결 확인
ping -c 3 8.8.8.8

# 2. DNS 확인
nslookup stream.binance.com

# 3. HTTPS 포트 확인
curl -I https://api.binance.com/api/v3/ping

# 4. 보안 그룹 확인 (아웃바운드 HTTPS 허용?)
```

### Discord 알림 미수신

```bash
# 1. Webhook URL 직접 테스트
curl -H "Content-Type: application/json" \
  -d '{"content": "Test from EC2"}' \
  "YOUR_WEBHOOK_URL"

# 2. .env 파일 확인
sudo -u rusto cat /home/rusto/fabio-trading/.env | grep DISCORD

# 3. config.toml에서 Discord 활성화 확인
sudo -u rusto cat /home/rusto/fabio-trading/config.toml | grep -A2 "\[discord\]"
```

### 메모리 부족

t4g.small은 2GB RAM이므로 스왑 추가 권장:

```bash
# 2GB 스왑 생성
sudo dd if=/dev/zero of=/swapfile bs=1M count=2048
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile

# 영구 설정
echo '/swapfile swap swap defaults 0 0' | sudo tee -a /etc/fstab

# 확인
free -h
```

### 빌드 실패

```bash
# Rust 버전 확인
sudo -u rusto bash -c 'source ~/.cargo/env && rustc --version'

# Rust 업데이트
sudo -u rusto bash -c 'source ~/.cargo/env && rustup update'

# 클린 빌드
cd /home/rusto/fabio-trading
sudo -u rusto bash -c 'source ~/.cargo/env && cargo clean && cargo build --release'
```

## ARM 최적화 팁

### 1. 컴파일 최적화 확인

Cargo.toml에 이미 최적화 설정이 포함되어 있습니다:

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
```

### 2. 성능 확인

ARM Graviton2는 x86과 비교해 동등하거나 더 나은 성능을 제공합니다:

```bash
# CPU 정보
lscpu | grep -E "Architecture|Model name|CPU\(s\)"

# 벤치마크 (선택사항)
sudo -u rusto bash -c 'cd ~/fabio-trading && source ~/.cargo/env && cargo bench'
```

## 비용 절감 팁

### t4g.small 월 예상 비용 (서울 리전)

- **온디맨드**: ~$13/월
- **1년 예약**: ~$8/월 (38% 할인)
- **3년 예약**: ~$5/월 (62% 할인)

### 추가 절감

1. **스팟 인스턴스**: 최대 70% 할인 (중단 가능성 있음)
2. **Savings Plans**: 유연한 할인 플랜
3. **야간 자동 중지**: 테스트 기간에만

### CloudWatch 알람 설정

비정상 동작 감지:

```bash
# AWS CLI로 설정 (예: CPU 90% 이상)
aws cloudwatch put-metric-alarm \
  --alarm-name rusto-high-cpu \
  --alarm-description "Rusto CPU > 90%" \
  --metric-name CPUUtilization \
  --namespace AWS/EC2 \
  --statistic Average \
  --period 300 \
  --threshold 90 \
  --comparison-operator GreaterThanThreshold \
  --dimensions Name=InstanceId,Value=i-xxxxx \
  --evaluation-periods 2
```

## 보안 권장사항

### 1. SSH 보안 강화

```bash
# SSH 설정 편집
sudo nano /etc/ssh/sshd_config

# 다음 설정 권장:
# PermitRootLogin no
# PasswordAuthentication no
# PubkeyAuthentication yes

# SSH 재시작
sudo systemctl restart sshd
```

### 2. 자동 업데이트

```bash
# Amazon Linux 2023 자동 업데이트 활성화
sudo dnf install -y dnf-automatic
sudo systemctl enable --now dnf-automatic.timer
```

### 3. 방화벽

Amazon Linux 2023는 firewalld 사용:

```bash
# 방화벽 활성화
sudo systemctl enable --now firewalld

# SSH만 허용
sudo firewall-cmd --permanent --add-service=ssh
sudo firewall-cmd --reload

# 상태 확인
sudo firewall-cmd --list-all
```

## 체크리스트

### 설치 후 확인사항

- [ ] SSH 접속 가능
- [ ] 설치 로그에 "Setup Complete" 표시
- [ ] .env 파일에 Discord Webhook URL 설정
- [ ] config.toml 설정 확인
- [ ] systemd 서비스 시작됨
- [ ] 로그에서 WebSocket 연결 확인
- [ ] Discord 알림 수신 확인
- [ ] 데이터베이스 생성 확인 (trades.db)

### 일일 체크사항

- [ ] `sudo systemctl status rusto` - 서비스 정상 동작
- [ ] `tail /home/rusto/fabio-trading/rusto.log` - 에러 없음
- [ ] Discord 알림 정상 수신
- [ ] 데이터베이스 조회 가능

## 요약

```bash
# 🚀 빠른 시작 (EC2 생성 후)
ssh -i your-key.pem ec2-user@<EC2-IP>
sudo -u rusto nano /home/rusto/fabio-trading/.env  # Webhook URL 설정
sudo systemctl enable --now rusto
sudo journalctl -u rusto -f

# 📊 모니터링
sudo systemctl status rusto
tail -f /home/rusto/fabio-trading/rusto.log
sudo -u rusto sqlite3 /home/rusto/fabio-trading/trades.db "SELECT * FROM positions LIMIT 10;"

# 🔄 업데이트
sudo systemctl stop rusto
cd /home/rusto/fabio-trading
sudo -u rusto git pull
sudo -u rusto bash -c 'source ~/.cargo/env && cargo build --release'
sudo systemctl start rusto
```

## 지원

문제가 발생하면:
1. 로그 확인: `sudo journalctl -u rusto -n 100`
2. GitHub Issues: https://github.com/20hyo/fabio-trading/issues
3. DEPLOY.md 문서 참고

**Happy Trading! 🎯**
