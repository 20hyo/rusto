#!/bin/bash
set -e

# EC2 User Data Script for Rusto Trading Bot
# This script runs as root on instance launch

echo "=== Rusto Trading Bot EC2 Setup Starting ==="

# Detect OS
if [ -f /etc/os-release ]; then
    . /etc/os-release
    OS=$ID
else
    echo "Cannot detect OS"
    exit 1
fi

# App runtime settings
APP_USER="ec2-user"
APP_DIR="/home/ec2-user/rusto"
REPO_URL="https://github.com/20hyo/rusto.git"

# Update system
echo "Updating system packages..."
if [ "$OS" = "ubuntu" ] || [ "$OS" = "debian" ]; then
    apt-get update
    apt-get upgrade -y
    apt-get install -y curl git build-essential pkg-config libssl-dev
elif [ "$OS" = "amzn" ]; then
    yum update -y
    yum groupinstall -y "Development Tools"
    yum install -y git openssl-devel
fi

echo "Ensuring $APP_USER user exists..."
if ! id -u "$APP_USER" > /dev/null 2>&1; then
    useradd -m -s /bin/bash "$APP_USER"
fi

mkdir -p "/home/$APP_USER"
chown -R "$APP_USER:$APP_USER" "/home/$APP_USER"

echo "Installing Rust..."
sudo -u "$APP_USER" bash -lc "
  if [ ! -d /home/$APP_USER/.cargo ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  fi
"

echo "Cloning or pulling repository..."
sudo -u "$APP_USER" bash -lc "
  if [ -d /home/$APP_USER/rusto/.git ]; then
    cd /home/$APP_USER/rusto && git pull
  else
    if [ -d /home/$APP_USER/rusto ] && [ \"\$(ls -A /home/$APP_USER/rusto)\" ]; then
      echo \"Directory exists but is not a git repository: /home/$APP_USER/rusto\"
      exit 1
    fi
    git clone $REPO_URL /home/$APP_USER/rusto
  fi
"

echo "Setting up environment variables..."
sudo -u "$APP_USER" bash -lc "
  if [ ! -f $APP_DIR/.env ]; then
    cp $APP_DIR/.env.example $APP_DIR/.env 2>/dev/null || echo \"DISCORD_WEBHOOK_URL=\" > $APP_DIR/.env
    echo \"WARNING: Please update $APP_DIR/.env with your Discord webhook URL!\"
  fi
"

echo "Building Rusto..."
sudo -u "$APP_USER" bash -lc "
  source /home/$APP_USER/.cargo/env
  cd $APP_DIR
  cargo build --release
"

echo "Creating systemd service..."
cat > /etc/systemd/system/rusto.service <<'EOF'
[Unit]
Description=Rusto Trading Bot
After=network.target

[Service]
Type=simple
User=ec2-user
WorkingDirectory=/home/ec2-user/rusto
Environment="PATH=/home/ec2-user/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
ExecStart=/home/ec2-user/rusto/target/release/rusto
Restart=always
RestartSec=10
StandardOutput=append:/home/ec2-user/rusto/rusto.log
StandardError=append:/home/ec2-user/rusto/rusto.error.log

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload

echo "=== Setup Complete ==="
echo ""
echo "IMPORTANT: Before starting the bot, you MUST:"
echo "1. Edit /home/ec2-user/rusto/.env and set your DISCORD_WEBHOOK_URL"
echo "2. Review /home/ec2-user/rusto/config.toml if needed"
echo ""
echo "To configure and start the bot:"
echo "  sudo -u ec2-user nano /home/ec2-user/rusto/.env"
echo "  sudo systemctl enable rusto"
echo "  sudo systemctl start rusto"
echo ""
echo "To view logs:"
echo "  sudo journalctl -u rusto -f"
echo "  tail -f /home/ec2-user/rusto/rusto.log"
echo ""
echo "To update the bot:"
echo "  cd /home/ec2-user/rusto"
echo "  sudo -u ec2-user git pull"
echo "  sudo -u ec2-user bash -lc 'source /home/ec2-user/.cargo/env && cd /home/ec2-user/rusto && cargo build --release'"
echo "  sudo systemctl restart rusto"
echo ""
