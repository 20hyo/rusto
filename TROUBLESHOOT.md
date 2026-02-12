# EC2 User Data 스크립트 문제 해결

## 현재 상황

Cloud-init이 User Data 스크립트 실행에 실패했습니다.

## 진단 단계

### 1. 상세 로그 확인

```bash
# User Data 스크립트 로그
sudo cat /var/log/cloud-init-output.log

# Cloud-init 에러 로그
sudo cat /var/log/cloud-init.log | grep -i error

# User Data 스크립트 자체
sudo cat /var/lib/cloud/instance/user-data.txt

# 스크립트 실행 로그
sudo cat /var/log/rusto-setup.log
```

### 2. 수동 설치 (권장)

User Data가 실패했다면, 수동으로 설치하는 것이 가장 빠릅니다:

```bash
# 1. 설치 스크립트 다운로드
curl -fsSL https://raw.githubusercontent.com/20hyo/fabio-trading/main/setup-ec2.sh -o setup-ec2.sh

# 2. 권한 부여
chmod +x setup-ec2.sh

# 3. 실행
sudo ./setup-ec2.sh
```

### 3. 단계별 수동 설치

스크립트도 실패한다면, 하나씩 수동으로:

```bash
# 1. 시스템 업데이트
sudo yum update -y

# 2. 필수 패키지 설치
sudo yum groupinstall -y "Development Tools"
sudo yum install -y git openssl-devel

# 3. rusto 사용자 생성
sudo useradd -m -s /bin/bash rusto

# 4. Rust 설치 (rusto 사용자로)
sudo -u rusto bash -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y'

# 5. 저장소 클론
sudo -u rusto bash -c 'cd ~ && git clone https://github.com/20hyo/fabio-trading.git'

# 6. .env 파일 생성
sudo -u rusto bash -c 'cd ~/fabio-trading && cp .env.example .env'

# 7. 빌드 (15-20분 소요)
sudo -u rusto bash -c 'cd ~/fabio-trading && source ~/.cargo/env && cargo build --release'

# 8. systemd 서비스 생성
sudo tee /etc/systemd/system/rusto.service > /dev/null <<EOF
[Unit]
Description=Rusto Trading Bot
After=network.target

[Service]
Type=simple
User=rusto
WorkingDirectory=/home/rusto/fabio-trading
Environment="PATH=/home/rusto/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
ExecStart=/home/rusto/fabio-trading/target/release/rusto
Restart=always
RestartSec=10
StandardOutput=append:/home/rusto/fabio-trading/rusto.log
StandardError=append:/home/rusto/fabio-trading/rusto.error.log

[Install]
WantedBy=multi-user.target
EOF

# 9. systemd 리로드
sudo systemctl daemon-reload

echo "설치 완료! 이제 .env 파일을 수정하고 서비스를 시작하세요."
```

## 설치 후 설정

```bash
# 1. Discord Webhook URL 설정
sudo -u rusto nano /home/rusto/fabio-trading/.env
# DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/...

# 2. 설정 확인 (선택)
sudo -u rusto nano /home/rusto/fabio-trading/config.toml

# 3. 서비스 시작
sudo systemctl enable rusto
sudo systemctl start rusto

# 4. 로그 확인
sudo journalctl -u rusto -f
```

## 자주 발생하는 문제

### 1. curl 명령 실패

```bash
# 원인: GitHub 접근 불가
# 해결: 보안 그룹 아웃바운드 HTTPS 허용 확인

# 테스트
curl -I https://github.com
curl -I https://raw.githubusercontent.com
```

### 2. Rust 설치 실패

```bash
# 원인: 네트워크 또는 권한 문제
# 해결: 수동으로 설치

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
rustc --version
```

### 3. 빌드 실패 (메모리 부족)

```bash
# t4g.small은 2GB RAM - 빌드 시 부족할 수 있음
# 해결: 스왑 메모리 추가

# 4GB 스왑 생성
sudo dd if=/dev/zero of=/swapfile bs=1M count=4096
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile
echo '/swapfile swap swap defaults 0 0' | sudo tee -a /etc/fstab

# 확인
free -h

# 다시 빌드
sudo -u rusto bash -c 'cd ~/fabio-trading && source ~/.cargo/env && cargo build --release'
```

### 4. Git clone 실패

```bash
# 원인: 저장소 private 또는 네트워크 문제
# 해결: HTTPS로 클론

sudo -u rusto bash -c 'cd ~ && git clone https://github.com/20hyo/fabio-trading.git'
```

## 검증

설치가 완료되었는지 확인:

```bash
# 1. 파일 존재 확인
ls -la /home/rusto/fabio-trading/target/release/rusto

# 2. 실행 가능 확인
file /home/rusto/fabio-trading/target/release/rusto
# 출력: ELF 64-bit LSB executable, ARM aarch64 ...

# 3. 수동 실행 테스트
sudo -u rusto /home/rusto/fabio-trading/target/release/rusto --help

# 4. 서비스 상태
sudo systemctl status rusto

# 5. .env 파일 확인
sudo -u rusto cat /home/rusto/fabio-trading/.env
```

## 완전히 새로 시작

모든 것을 삭제하고 처음부터:

```bash
# 1. 서비스 중지 및 삭제
sudo systemctl stop rusto 2>/dev/null
sudo systemctl disable rusto 2>/dev/null
sudo rm -f /etc/systemd/system/rusto.service

# 2. 사용자 및 파일 삭제
sudo userdel -r rusto 2>/dev/null

# 3. 위의 "단계별 수동 설치" 따라하기
```

## 빠른 복구 스크립트

모든 것을 한 번에:

```bash
curl -fsSL https://raw.githubusercontent.com/20hyo/fabio-trading/main/setup-ec2.sh | sudo bash
```

또는 스크립트를 다운로드 후 수정:

```bash
curl -fsSL https://raw.githubusercontent.com/20hyo/fabio-trading/main/setup-ec2.sh -o setup.sh
chmod +x setup.sh
# 필요시 편집
nano setup.sh
# 실행
sudo ./setup.sh
```
