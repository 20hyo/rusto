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

# Create application user
echo "Creating rusto user..."
if ! id -u rusto > /dev/null 2>&1; then
    useradd -m -s /bin/bash rusto
fi

# Install Rust as rusto user
echo "Installing Rust..."
sudo -u rusto bash <<'EOF'
cd ~
if [ ! -d "$HOME/.cargo" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi
EOF

# Clone repository
echo "Cloning repository..."
sudo -u rusto bash <<'EOF'
cd ~
if [ ! -d "$HOME/fabio-trading" ]; then
    git clone https://github.com/20hyo/fabio-trading.git
else
    cd fabio-trading
    git pull
fi
EOF

# Setup environment variables
echo "Setting up environment variables..."
sudo -u rusto bash <<'EOF'
cd ~/fabio-trading
if [ ! -f .env ]; then
    cp .env.example .env
    echo "WARNING: Please update .env file with your Discord webhook URL!"
fi
EOF

# Build the project
echo "Building Rusto..."
sudo -u rusto bash <<'EOF'
cd ~/fabio-trading
source "$HOME/.cargo/env"
cargo build --release
EOF

# Create systemd service
echo "Creating systemd service..."
cat > /etc/systemd/system/rusto.service <<'EOF'
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

# Reload systemd
systemctl daemon-reload

# Don't start automatically - user needs to configure .env first
echo "=== Setup Complete ==="
echo ""
echo "IMPORTANT: Before starting the bot, you MUST:"
echo "1. Edit /home/rusto/fabio-trading/.env and set your DISCORD_WEBHOOK_URL"
echo "2. Review /home/rusto/fabio-trading/config.toml if needed"
echo ""
echo "To configure and start the bot:"
echo "  sudo -u rusto nano /home/rusto/fabio-trading/.env"
echo "  sudo systemctl enable rusto"
echo "  sudo systemctl start rusto"
echo ""
echo "To view logs:"
echo "  sudo journalctl -u rusto -f"
echo "  tail -f /home/rusto/fabio-trading/rusto.log"
echo ""
echo "To update the bot:"
echo "  cd /home/rusto/fabio-trading"
echo "  sudo -u rusto git pull"
echo "  sudo -u rusto cargo build --release"
echo "  sudo systemctl restart rusto"
echo ""
