#!/bin/bash

# Avena-OTR Dashboard NATS Cleanup Script
# This script stops NATS server and cleans up data

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

echo "🧹 Avena-OTR Dashboard NATS Cleanup"
echo "===================================="
echo

# Check if NATS is running
if pgrep nats-server >/dev/null 2>&1; then
    print_status "Stopping NATS server..."
    pkill nats-server
    sleep 2
    
    # Force kill if still running
    if pgrep nats-server >/dev/null 2>&1; then
        print_warning "Force killing NATS server..."
        pkill -9 nats-server
        sleep 1
    fi
    
    print_success "NATS server stopped"
else
    print_status "NATS server is not running"
fi

# Remove PID file if exists
if [ -f "nats.pid" ]; then
    rm -f nats.pid
    print_status "Removed PID file"
fi

# Ask user what to clean up
echo
echo "What would you like to clean up?"
echo "1) Clear all data (JetStream + KV stores)"
echo "2) Clear only JetStream data"
echo "3) Clear only specific Avena box data"
echo "4) Just stop server (keep data)"
echo "5) Exit without changes"
echo
read -p "Enter your choice (1-5): " choice

case $choice in
    1)
        print_status "Clearing all data..."
        if [ -d "./jetstream" ]; then
            rm -rf ./jetstream
            print_success "JetStream data cleared"
        fi
        print_success "All data cleared. Run ./setup_nats.sh to restart with fresh data."
        ;;
    2)
        print_status "Clearing JetStream data..."
        if [ -d "./jetstream" ]; then
            rm -rf ./jetstream
            print_success "JetStream data cleared"
        fi
        print_success "JetStream data cleared. Run ./setup_nats.sh to restart."
        ;;
    3)
        echo
        echo "Available Avena boxes:"
        if command -v nats >/dev/null 2>&1; then
            nats kv ls 2>/dev/null | grep -v "all_cabinets" || echo "No Avena box buckets found"
        else
            echo "NATS CLI not available"
        fi
        echo
        read -p "Enter Avena box name to clear (or 'all' for all): " box_name
        if [ "$box_name" = "all" ]; then
            print_status "Clearing all Avena box data..."
            if command -v nats >/dev/null 2>&1; then
                for bucket in $(nats kv ls 2>/dev/null | grep -v "all_cabinets"); do
                    nats kv del "$bucket" 2>/dev/null || true
                done
                print_success "All Avena box data cleared"
            fi
        elif [ -n "$box_name" ]; then
            print_status "Clearing $box_name data..."
            if command -v nats >/dev/null 2>&1; then
                nats kv del "$box_name" 2>/dev/null || true
                print_success "$box_name data cleared"
            fi
        fi
        ;;
    4)
        print_success "Server stopped. Data preserved."
        ;;
    5)
        print_status "Exiting without changes"
        exit 0
        ;;
    *)
        print_error "Invalid choice. Exiting."
        exit 1
        ;;
esac

echo
echo "Cleanup complete! 🎉"
echo
echo "To restart NATS server:"
echo "  ./setup_nats.sh"
echo
echo "To start dashboard:"
echo "  npm run dev"
