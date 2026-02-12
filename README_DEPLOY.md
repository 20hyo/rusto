# 🚀 Rusto 자동 배포 가이드

## 📋 필수 준비사항

### 1. EC2 인스턴스 생성 (Tokyo 리전)

**인스턴스 스펙:**
- **리전**: ap-northeast-1 (Tokyo)
- **AMI**: Amazon Linux 2023 ARM64
- **인스턴스 타입**: t4g.small
- **스토리지**: 20GB gp3
- **보안 그룹**: SSH (22) 포트 허용

### 2. GitHub Secrets 설정 (필수 3개만!)

Repository → Settings → Secrets and variables → Actions

```
EC2_SSH_PRIVATE_KEY        # ⭐ EC2 PEM 키 전체 내용 (-----BEGIN부터 -----END까지)
EC2_HOST                   # ⭐ EC2 Public IP (예: 13.230.123.456)
EC2_USER                   # ⭐ SSH 사용자명 (rusto)
DISCORD_WEBHOOK_URL        # Discord 웹훅 URL (선택)
```

**EC2_SSH_PRIVATE_KEY 설정 방법:**
```bash
# 로컬 터미널에서 PEM 파일 내용 복사
cat your-key.pem

# 출력된 내용 전체를 GitHub Secret에 붙여넣기
# -----BEGIN RSA PRIVATE KEY----- 부터
# -----END RSA PRIVATE KEY----- 까지 전부!
```

**EC2_HOST 확인:**
```bash
# AWS Console > EC2 > Instances
# 또는 터미널에서:
ssh -i your-key.pem ec2-user@13.230.xxx.xxx  # 이 IP 주소가 EC2_HOST
```

**EC2_USER:**
```
rusto  # setup-ec2.sh 실행 후 생성된 사용자
```

## 🔧 초기 설정 (EC2)

### SSH 접속:
```bash
ssh -i your-key.pem ec2-user@YOUR_EC2_IP
```

### 초기 설정 실행:
```bash
# 1. setup 스크립트 다운로드
curl -o setup-ec2.sh https://raw.githubusercontent.com/YOUR_USERNAME/rusto/main/setup-ec2.sh
chmod +x setup-ec2.sh

# 2. 초기 설정 실행 (sudo 필요)
sudo ./setup-ec2.sh

# 3. rusto 사용자로 전환
sudo su - rusto

# 4. 디렉토리 확인 및 이름 변경
ls -la
# fabio-trading이 있다면:
mv fabio-trading rusto
cd rusto

# 5. .env 파일 설정
nano .env
# DISCORD_WEBHOOK_URL=your_webhook_url 입력 후 Ctrl+X, Y, Enter

# 6. 서비스 시작
exit  # rusto 사용자에서 나가기
sudo systemctl enable rusto
sudo systemctl start rusto

# 7. 상태 확인
sudo systemctl status rusto
sudo journalctl -u rusto -f
```

## 🎯 자동 배포 사용법

### 자동 배포 (main 브랜치 push 시):
```bash
git add .
git commit -m "Update strategy"
git push origin main
```
→ GitHub Actions가 자동으로 EC2에 배포!

### 수동 배포 (GitHub UI):
1. GitHub Repository 접속
2. **Actions** 탭
3. **Deploy to EC2** workflow 선택
4. **Run workflow** 클릭

### 로컬에서 수동 배포:
```bash
export EC2_HOST="13.230.xxx.xxx"
export EC2_USER="rusto"
./scripts/deploy.sh
```

## 📊 모니터링

```bash
# 실시간 로그 (Ctrl+C로 종료)
sudo journalctl -u rusto -f

# 최근 100줄
sudo journalctl -u rusto -n 100

# 에러만 보기
sudo journalctl -u rusto -p err

# 서비스 상태
sudo systemctl status rusto
```

## 🔍 트러블슈팅

### 1. 배포가 실패한다면?

**GitHub Actions 로그 확인:**
- GitHub > Actions > 실패한 workflow 클릭
- 에러 메시지 확인

**일반적인 원인:**
```bash
# ❌ SSH 키 형식 오류
→ EC2_SSH_PRIVATE_KEY에 전체 내용 복사했는지 확인
→ -----BEGIN RSA PRIVATE KEY----- 부터 -----END까지 전부

# ❌ EC2 접속 불가
→ EC2_HOST가 정확한지 확인
→ EC2 보안 그룹에서 SSH (22) 포트 열렸는지 확인
→ 로컬에서 테스트: ssh -i your-key.pem ec2-user@YOUR_EC2_IP

# ❌ 권한 오류
→ rusto 사용자가 생성되었는지 확인: id rusto
→ 디렉토리 권한 확인: ls -la /home/rusto/rusto
```

### 2. 서비스가 시작 안 된다면?

```bash
# 에러 로그 확인
sudo journalctl -u rusto -n 100

# 일반적인 원인:
# ❌ .env 파일 없음
cd /home/rusto/rusto
ls -la .env
cat .env  # DISCORD_WEBHOOK_URL 확인

# ❌ config.toml 오류
cat config.toml

# ❌ 빌드 실패
sudo -u rusto bash
cd ~/rusto
source ~/.cargo/env
cargo build --release
```

### 3. 메모리 부족 (빌드 중 죽는다면)

```bash
# 스왑 파일 생성 (2GB)
sudo dd if=/dev/zero of=/swapfile bs=1G count=2
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile

# 영구 적용
echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab

# 확인
free -h
```

### 4. 시간 동기화 오류

```bash
# NTP 동기화
sudo chronyc -a makestep

# 확인
timedatectl status
```

## 💰 예상 비용

**t4g.small Tokyo 리전:**
- 온디맨드: $0.0168/시간 = **~$12/월**
- 1년 예약: **~$7/월** (40% 할인)
- 3년 예약: **~$5/월** (60% 할인)

## 📁 디렉토리 구조

```
/home/rusto/rusto/
├── target/release/rusto   # 실행 파일
├── config.toml            # 설정 파일
├── .env                   # 환경 변수 (웹훅)
├── trades.db              # SQLite DB
├── rusto.log              # 일반 로그
└── rusto.error.log        # 에러 로그
```

## ⚙️ systemd 서비스

```bash
# 서비스 상태
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

## 🔐 보안 체크리스트

- [x] EC2 SSH 키는 GitHub Secrets에만 저장
- [x] Discord 웹훅 URL은 .env 파일 또는 GitHub Secrets
- [x] .env 파일은 .gitignore에 등록됨
- [x] EC2 보안 그룹: SSH는 필요한 IP만 허용 권장
- [x] AWS IAM 키 불필요 (SSH만 사용)

## ✅ 배포 프로세스

```
Code Push (main branch)
    ↓
GitHub Actions 트리거
    ↓
SSH로 EC2 접속
    ↓
git pull origin main
    ↓
cargo build --release
    ↓
systemctl restart rusto
    ↓
상태 확인
    ↓
Discord 알림 (선택)
```

## 💡 팁

### 로그 실시간 모니터링:
```bash
# 2개 터미널 창 열어서:
# 터미널 1:
sudo journalctl -u rusto -f

# 터미널 2:
tail -f /home/rusto/rusto/rusto.log
```

### 빠른 재배포:
```bash
# EC2에서 직접
cd /home/rusto/rusto
sudo -u rusto git pull
sudo -u rusto cargo build --release
sudo systemctl restart rusto
```

### 백업:
```bash
# 데이터베이스 백업
cp /home/rusto/rusto/trades.db ~/backup_$(date +%Y%m%d).db
```

---

## 📞 도움말

**문제가 해결 안 되면:**
1. `docs/DEPLOYMENT.md` 상세 가이드 참고
2. GitHub Issues에 로그 첨부하여 질문
3. 로그 명령어: `sudo journalctl -u rusto -n 200`

**완료!** 이제 main 브랜치에 push만 하면 자동으로 Tokyo EC2에 배포됩니다! 🎉
