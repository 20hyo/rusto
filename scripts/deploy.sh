#!/bin/bash
set -e

# Manual deployment script for Rusto Trading Bot
# Usage: ./scripts/deploy.sh [EC2_HOST] [EC2_USER]

EC2_HOST=${1:-$EC2_HOST}
EC2_USER=${2:-$EC2_USER}

if [ -z "$EC2_HOST" ] || [ -z "$EC2_USER" ]; then
    echo "Usage: ./scripts/deploy.sh <EC2_HOST> <EC2_USER>"
    echo "Or set EC2_HOST and EC2_USER environment variables"
    exit 1
fi

echo "Deploying to ${EC2_USER}@${EC2_HOST}..."

ssh ${EC2_USER}@${EC2_HOST} << 'ENDSSH'
    set -e

    echo "=== Starting Rusto deployment ==="

    # Navigate to application directory
    cd ~/rusto || cd ~/fabio-trading || {
        echo "Error: Application directory not found"
        exit 1
    }

    # Stop service before updating
    echo "Stopping service..."
    sudo systemctl stop rusto || true

    # Pull latest changes
    echo "Pulling latest code..."
    git fetch origin
    git reset --hard origin/main

    # Load Rust environment
    source "$HOME/.cargo/env"

    # Build release binary
    echo "Building release binary..."
    cargo build --release

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
