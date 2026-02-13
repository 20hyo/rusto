#!/bin/bash
set -e

# Manual deployment script for Rusto Trading Bot
# Usage: ./scripts/deploy.sh [EC2_HOST] [EC2_USER] [EC2_KEY]

EC2_HOST=${1:-$EC2_HOST}
EC2_USER=${2:-$EC2_USER}
EC2_KEY=${3:-${EC2_KEY:-${EC2_SSH_KEY:-}}}

if [ -z "$EC2_HOST" ] || [ -z "$EC2_USER" ]; then
    echo "Usage: ./scripts/deploy.sh <EC2_HOST> <EC2_USER> [EC2_KEY]"
    echo "Or set EC2_HOST, EC2_USER, and EC2_KEY environment variables"
    exit 1
fi

SSH_CMD=(ssh)
if [ -n "$EC2_KEY" ]; then
    if [ ! -f "$EC2_KEY" ]; then
        echo "ERROR: SSH key not found: $EC2_KEY"
        exit 1
    fi
    SSH_CMD+=( -i "$EC2_KEY" )
fi

echo "Deploying to ${EC2_USER}@${EC2_HOST}..."
if [ -n "$EC2_KEY" ]; then
    echo "Using SSH key: $EC2_KEY"
fi

"${SSH_CMD[@]}" "${EC2_USER}@${EC2_HOST}" << 'ENDSSH'
    set -e

    APP_DIR="/home/ec2-user/rusto"
    APP_USER="ec2-user"
    REPO_URL="https://github.com/20hyo/rusto.git"

    echo "=== Starting Rusto deployment ==="

    # Navigate to application directory
    if [ -d "$APP_DIR/.git" ]; then
        cd "$APP_DIR"
    elif [ -d "$APP_DIR" ]; then
        echo "Error: $APP_DIR exists but is not a git repository"
        exit 1
    else
        echo "Repository not found, cloning into $APP_DIR"
        sudo -u "$APP_USER" bash -lc "mkdir -p /home/ec2-user && git clone $REPO_URL $APP_DIR"
        cd "$APP_DIR"
    fi

    # Stop service before updating
    echo "Stopping service..."
    sudo systemctl stop rusto || true

    # Pull latest changes
    echo "Pulling latest code..."
    sudo -u "$APP_USER" bash -lc "cd $APP_DIR && git fetch origin && git reset --hard origin/main"

    # Sync config override from CI/local temp if present
    if [ -f /tmp/rusto.config.toml ]; then
        cp /tmp/rusto.config.toml "$APP_DIR/config.toml"
        chown "$APP_USER":"$APP_USER" "$APP_DIR/config.toml"
    fi

    if [ -f /tmp/rusto.env ]; then
        # shellcheck disable=SC1091
        . /tmp/rusto.env
    fi

    # Initialize env file if missing
    if [ ! -f "$APP_DIR/.env" ]; then
        if [ -f "$APP_DIR/.env.example" ]; then
            cp "$APP_DIR/.env.example" "$APP_DIR/.env"
        else
            echo "DISCORD_WEBHOOK_URL=" > "$APP_DIR/.env"
        fi
        chown "$APP_USER":"$APP_USER" "$APP_DIR/.env"
    fi

    if [ -n "${DISCORD_WEBHOOK_URL:-}" ]; then
        if grep -q "^DISCORD_WEBHOOK_URL=" "$APP_DIR/.env"; then
            sed -i "s|^DISCORD_WEBHOOK_URL=.*|DISCORD_WEBHOOK_URL=${DISCORD_WEBHOOK_URL}|" "$APP_DIR/.env"
        else
            echo "DISCORD_WEBHOOK_URL=${DISCORD_WEBHOOK_URL}" >> "$APP_DIR/.env"
        fi
    fi

    # Ensure APP_USER can run the application
    sudo -u "$APP_USER" bash -lc "mkdir -p $APP_DIR"

    # Load Rust environment
    if [ ! -f "/home/$APP_USER/.cargo/env" ]; then
        sudo -u "$APP_USER" bash -lc "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
    fi

    # Build release binary
    echo "Building release binary..."
    sudo -u "$APP_USER" bash -lc "source /home/$APP_USER/.cargo/env && cd $APP_DIR && cargo build --release"

    # Replace system service to force /home/ec2-user/rusto runtime path
    sudo tee /etc/systemd/system/rusto.service > /dev/null <<'RUSTO_SERVICE'
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
RUSTO_SERVICE

    echo "Reloading systemd daemon..."
    sudo systemctl daemon-reload

    # Start service
    echo "Starting rusto service..."
    sudo systemctl start rusto

    # Check status
    sleep 3
    sudo systemctl status rusto --no-pager

    echo "=== Deployment completed successfully ==="
    echo ""
    echo "To view logs:"
    echo "  sudo journalctl -u rusto -f"
ENDSSH

echo ""
echo "✓ Deployment completed!"
