# Rusto Deployment Guide

## 자동 배포 (GitHub Actions)

### 1. GitHub Secrets 설정

Repository Settings > Secrets and variables > Actions에서 다음 secrets을 추가:

#### 필수 Secrets:
- `AWS_ACCESS_KEY_ID`: AWS IAM 사용자 Access Key ID
- `AWS_SECRET_ACCESS_KEY`: AWS IAM 사용자 Secret Access Key
- `EC2_SSH_PRIVATE_KEY`: EC2 인스턴스 SSH 개인키 (PEM 파일 내용)
- `EC2_HOST`: EC2 인스턴스 퍼블릭 IP 또는 도메인
- `EC2_USER`: EC2 SSH 사용자명 (기본: `ec2-user`)
- `DISCORD_WEBHOOK_URL`: Discord 웹훅 URL

### 2. EC2 인스턴스 초기 설정

#### 인스턴스 스펙:
- **리전**: Tokyo (ap-northeast-1)
- **인스턴스 타입**: t4g.small (ARM 아키텍처)
- **AMI**: Amazon Linux 2023 ARM64
- **스토리지**: 20GB gp3
- **보안 그룹**: SSH (22) 허용

#### SSH 접속 및 설정:
```bash
# SSH 접속
ssh -i your-key.pem ec2-user@YOUR_EC2_IP

# 초기 설정 실행
curl -o setup-ec2.sh https://raw.githubusercontent.com/YOUR_USERNAME/rusto/main/setup-ec2.sh
chmod +x setup-ec2.sh
sudo ./setup-ec2.sh

# .env 파일 수정
sudo -u ec2-user nano /home/ec2-user/rusto/.env

# 서비스 시작
sudo systemctl enable rusto
sudo systemctl start rusto
```

### 3. 자동 배포 트리거

main 브랜치에 push하면 자동 배포:
```bash
git push origin main
```

## 모니터링

```bash
# 로그 확인
sudo journalctl -u rusto -f

# 서비스 상태
sudo systemctl status rusto
```
