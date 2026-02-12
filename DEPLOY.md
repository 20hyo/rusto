# EC2 배포 가이드

이 가이드는 AWS EC2 인스턴스에서 Rusto 트레이딩 봇을 배포하는 방법을 설명합니다.

## 사전 요구사항

- AWS 계정
- SSH 키 페어
- Discord Webhook URL

## EC2 인스턴스 생성

### 1. 인스턴스 설정

#### 권장 사양
- **AMI**: Ubuntu 22.04 LTS 또는 Amazon Linux 2023
- **인스턴스 타입**: t3.small 이상 (메모리 2GB+)
- **스토리지**: 20GB gp3
- **보안 그룹**:
  - SSH (22) - 내 IP에서만 접근
  - 아웃바운드: 모두 허용 (Binance WebSocket, Discord Webhook 접근 필요)

#### 키 페어
- 기존 키 페어 선택 또는 새로 생성
- `.pem` 파일을 안전한 곳에 보관

### 2. User Data 스크립트 설정

EC2 인스턴스 생성 시 "고급 세부 정보" → "사용자 데이터" 섹션에 다음 내용을 붙여넣기:

```bash
#!/bin/bash
curl -fsSL https://raw.githubusercontent.com/20hyo/fabio-trading/main/setup-ec2.sh | bash > /var/log/rusto-setup.log 2>&1
```

또는 `setup-ec2.sh` 파일의 전체 내용을 직접 복사하여 붙여넣기

### 3. 인스턴스 시작

"인스턴스 시작" 버튼 클릭

## 설정

### 1. 인스턴스 접속

```bash
# 키 페어 권한 설정 (최초 1회)
chmod 400 your-key.pem

# SSH 접속
ssh -i your-key.pem ubuntu@<EC2-PUBLIC-IP>
```

### 2. 설치 확인

User Data 스크립트가 완료될 때까지 5-10분 정도 소요됩니다.

```bash
# 설치 로그 확인
sudo tail -f /var/log/rusto-setup.log

# 또는 cloud-init 로그 확인
sudo tail -f /var/log/cloud-init-output.log
```

### 3. 환경 변수 설정

**중요**: Discord Webhook URL을 설정해야 합니다.

```bash
# .env 파일 편집
sudo -u rusto nano /home/rusto/fabio-trading/.env
```

`.env` 파일 내용:
```bash
DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/YOUR_WEBHOOK_ID/YOUR_WEBHOOK_TOKEN
```

저장: `Ctrl+O`, `Enter`, 종료: `Ctrl+X`

### 4. 설정 파일 확인 (선택사항)

```bash
sudo -u rusto nano /home/rusto/fabio-trading/config.toml
```

거래할 심볼, 전략, 리스크 설정 등을 수정할 수 있습니다.

## 봇 실행

### 수동 시작 (테스트용)

```bash
# rusto 사용자로 전환
sudo -u rusto -i

# 프로젝트 디렉토리로 이동
cd ~/fabio-trading

# 실행
./target/release/rusto
```

종료: `Ctrl+C`

### systemd 서비스로 실행 (권장)

```bash
# 서비스 활성화 (부팅 시 자동 시작)
sudo systemctl enable rusto

# 서비스 시작
sudo systemctl start rusto

# 상태 확인
sudo systemctl status rusto
```

## 로그 확인

### systemd 로그 (실시간)
```bash
sudo journalctl -u rusto -f
```

### 애플리케이션 로그
```bash
# 표준 출력 로그
tail -f /home/rusto/fabio-trading/rusto.log

# 에러 로그
tail -f /home/rusto/fabio-trading/rusto.error.log
```

### 로그 레벨 변경
```bash
# config.toml 수정
sudo -u rusto nano /home/rusto/fabio-trading/config.toml

# log_level을 "debug" 또는 "trace"로 변경
[general]
log_level = "debug"

# 서비스 재시작
sudo systemctl restart rusto
```

## 데이터베이스 확인

```bash
# SQLite 데이터베이스 접속
sudo -u rusto sqlite3 /home/rusto/fabio-trading/trades.db

# 최근 거래 조회
SELECT * FROM positions ORDER BY entry_time DESC LIMIT 10;

# 통계 조회
SELECT
  COUNT(*) as total_trades,
  SUM(CASE WHEN pnl > 0 THEN 1 ELSE 0 END) as wins,
  SUM(CASE WHEN pnl < 0 THEN 1 ELSE 0 END) as losses,
  SUM(pnl) as total_pnl
FROM positions WHERE status = 'Closed';

# 종료
.exit
```

## 봇 관리

### 재시작
```bash
sudo systemctl restart rusto
```

### 중지
```bash
sudo systemctl stop rusto
```

### 자동 시작 비활성화
```bash
sudo systemctl disable rusto
```

## 업데이트

새 버전으로 업데이트하려면:

```bash
# 봇 중지
sudo systemctl stop rusto

# 코드 업데이트
cd /home/rusto/fabio-trading
sudo -u rusto git pull

# 재빌드
sudo -u rusto bash -c 'source ~/.cargo/env && cargo build --release'

# 봇 재시작
sudo systemctl start rusto
```

## 백업

### 데이터베이스 백업
```bash
# 로컬로 다운로드
scp -i your-key.pem ubuntu@<EC2-IP>:/home/rusto/fabio-trading/trades.db ./trades-backup.db

# 또는 S3로 백업
sudo apt-get install -y awscli
aws s3 cp /home/rusto/fabio-trading/trades.db s3://your-bucket/backups/trades-$(date +%Y%m%d).db
```

### 설정 파일 백업
```bash
scp -i your-key.pem ubuntu@<EC2-IP>:/home/rusto/fabio-trading/.env ./.env.backup
scp -i your-key.pem ubuntu@<EC2-IP>:/home/rusto/fabio-trading/config.toml ./config.backup.toml
```

## 모니터링

### CPU/메모리 사용량
```bash
# 실시간 모니터링
top

# rusto 프로세스만
top -u rusto

# 또는 htop (더 보기 좋음)
sudo apt-get install -y htop
htop -u rusto
```

### 디스크 사용량
```bash
df -h
du -sh /home/rusto/fabio-trading/*
```

### 네트워크 연결 확인
```bash
# Binance WebSocket 연결 확인
sudo netstat -tnp | grep rusto
```

## 문제 해결

### 봇이 시작되지 않는 경우

1. **로그 확인**
```bash
sudo journalctl -u rusto -n 100 --no-pager
tail -100 /home/rusto/fabio-trading/rusto.error.log
```

2. **.env 파일 확인**
```bash
sudo -u rusto cat /home/rusto/fabio-trading/.env
```

3. **설정 파일 검증**
```bash
cd /home/rusto/fabio-trading
sudo -u rusto bash -c 'source ~/.cargo/env && cargo run -- --help'
```

### WebSocket 연결 실패

1. **아웃바운드 보안 그룹 확인**: 443 포트가 열려있는지 확인
2. **DNS 확인**: `ping stream.binance.com:9443`
3. **로그 확인**: WebSocket 연결 에러 메시지 확인

### Discord 알림이 오지 않는 경우

1. **.env 파일의 Webhook URL 확인**
2. **Discord에서 직접 테스트**:
```bash
curl -H "Content-Type: application/json" \
  -d '{"content": "Test message"}' \
  "YOUR_WEBHOOK_URL"
```

### 메모리 부족

인스턴스 타입을 t3.medium 이상으로 업그레이드하거나 스왑 메모리 추가:

```bash
# 2GB 스왑 파일 생성
sudo fallocate -l 2G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile

# 부팅 시 자동 마운트
echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
```

## 보안 권장사항

1. **SSH 포트 변경** (선택사항)
2. **Fail2ban 설치** (무차별 대입 공격 방지)
```bash
sudo apt-get install -y fail2ban
sudo systemctl enable fail2ban
```

3. **자동 업데이트 활성화**
```bash
sudo apt-get install -y unattended-upgrades
sudo dpkg-reconfigure --priority=low unattended-upgrades
```

4. **방화벽 설정**
```bash
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw allow ssh
sudo ufw enable
```

## 비용 최적화

- **스팟 인스턴스** 사용 고려 (최대 90% 절감)
- **예약 인스턴스** 장기 운영 시 고려
- **CloudWatch 알람** 설정하여 비정상 동작 감지
- **자동 종료 스크립트** 테스트 기간에는 야간 자동 종료

## 지원

문제가 발생하면 GitHub Issues에 보고해주세요:
https://github.com/20hyo/fabio-trading/issues
