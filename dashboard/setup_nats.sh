#!/bin/bash

# Avena-OTR Dashboard NATS Setup Script
# This script sets up NATS server with WebSocket support, JetStream, and sample data

set -e  # Exit on any error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
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

# Function to check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Function to check if NATS is running
nats_running() {
    pgrep nats-server >/dev/null 2>&1
}

# Function to stop NATS server
stop_nats() {
    if nats_running; then
        print_status "Stopping existing NATS server..."
        pkill nats-server || true
        sleep 2
        if nats_running; then
            print_warning "NATS server still running, force killing..."
            pkill -9 nats-server || true
            sleep 1
        fi
    fi
}

# Function to check if port is available
port_available() {
    local port=$1
    ! lsof -i :$port >/dev/null 2>&1
}

# Function to wait for NATS to be ready
wait_for_nats() {
    local max_attempts=30
    local attempt=1
    
    print_status "Waiting for NATS server to be ready..."
    
    while [ $attempt -le $max_attempts ]; do
        if curl -s http://localhost:8222/healthz >/dev/null 2>&1; then
            print_success "NATS server is ready!"
            return 0
        fi
        
        echo -n "."
        sleep 1
        attempt=$((attempt + 1))
    done
    
    print_error "NATS server failed to start within 30 seconds"
    return 1
}

# Function to create sample data
create_sample_data() {
    print_status "Creating sample data..."
    
    # Wait a bit for JetStream to initialize
    sleep 3
    
    # Create all_cabinets bucket
    if ! nats kv ls | grep -q "all_cabinets"; then
        print_status "Creating all_cabinets bucket..."
        nats kv add all_cabinets
    fi
    
    # Create sample Avena box data
    print_status "Adding sample Avena box data..."
    nats kv put all_cabinets avenabox_001 '{"status": "online"}' || true
    nats kv put all_cabinets avenabox_002 '{"status": "offline"}' || true
    nats kv put all_cabinets avenabox_003 '{"status": "maintenance"}' || true
    
    # Create Avena box-specific buckets and LabJack configurations
    for avenabox in avenabox_001 avenabox_002 avenabox_003; do
        print_status "Setting up $avenabox..."
        
        # Create Avena box bucket
        if ! nats kv ls | grep -q "$avenabox"; then
            nats kv add "$avenabox"
        fi
        
        # Create sample LabJack configurations
        case $avenabox in
            "avenabox_001")
                nats kv put "$avenabox" labjackd.config.TEST001 '{
                    "cabinet_id": "avenabox_001",
                    "labjack_name": "Main Sensor Hub",
                    "serial": "TEST001",
                    "sensor_settings": {
                        "sampling_rate": 1000,
                        "channels_enabled": [1, 2, 3],
                        "gains": 1,
                        "data_formats": ["voltage", "temperature", "pressure"],
                        "measurement_units": ["V", "°C", "PSI"],
                        "publish_raw_data": [true, true, true],
                        "measure_peaks": [false, true, false],
                        "publish_summary_peaks": true,
                        "labjack_reset": false
                    }
                }' || true
                ;;
            "avenabox_002")
                nats kv put "$avenabox" labjackd.config.TEST002 '{
                    "cabinet_id": "avenabox_002",
                    "labjack_name": "Environmental Monitor",
                    "serial": "TEST002",
                    "sensor_settings": {
                        "sampling_rate": 500,
                        "channels_enabled": [1, 2],
                        "gains": 2,
                        "data_formats": ["temperature", "humidity"],
                        "measurement_units": ["°C", "%RH"],
                        "publish_raw_data": [true, true],
                        "measurement_peaks": [true, false],
                        "publish_summary_peaks": true,
                        "labjack_reset": false
                    }
                }' || true
                ;;
            "avenabox_003")
                nats kv put "$avenabox" labjackd.config.TEST003 '{
                    "cabinet_id": "avenabox_003",
                    "labjack_name": "Traffic Sensor",
                    "serial": "TEST003",
                    "sensor_settings": {
                        "sampling_rate": 100,
                        "channels_enabled": [1, 4, 5],
                        "gains": 1,
                        "data_formats": ["voltage", "digital", "digital"],
                        "measurement_units": ["V", "count", "count"],
                        "publish_raw_data": [true, false, false],
                        "measure_peaks": [false, true, true],
                        "publish_summary_peaks": false,
                        "labjack_reset": false
                    }
                }' || true
                ;;
        esac
    done
    
    print_success "Sample data created successfully!"
}

# Function to display connection information
show_connection_info() {
    echo
    echo "=========================================="
    echo "🎉 NATS Server Setup Complete!"
    echo "=========================================="
    echo
    echo "📡 Connection Details:"
    echo "   • NATS Port: 4222"
    echo "   • HTTP Monitor: http://localhost:8222"
    echo "   • WebSocket: ws://localhost:4443"
    echo
    echo "🔐 Dashboard Login:"
    echo "   • Server: ws://localhost:4443"
    echo "   • Password: (leave empty)"
    echo
    echo "📊 Sample Data Created:"
echo "   • 3 Avena boxes (avenabox_001, avenabox_002, avenabox_003)"
echo "   • 3 LabJack configurations with different sensor setups"
    echo
    echo "🚀 Next Steps:"
    echo "   1. Start dashboard: npm run dev"
    echo "   2. Open browser: http://localhost:5173"
    echo "   3. Login with WebSocket URL"
    echo
    echo "🛠️  Management Commands:"
    echo "   • View buckets: nats kv ls"
    echo "   • View data: nats kv keys bucket_name"
    echo "   • Stop server: pkill nats-server"
    echo "   • Restart: ./setup_nats.sh"
    echo
    echo "=========================================="
}

# Main setup function
main() {
    echo "🚀 Avena-OTR Dashboard NATS Setup"
    echo "=================================="
    echo
    
    # Check prerequisites
    print_status "Checking prerequisites..."
    
    if ! command_exists nats-server; then
        print_error "NATS server not found. Please install it first:"
        echo "   brew install nats-io/nats-tools/nats"
        exit 1
    fi
    
    if ! command_exists nats; then
        print_error "NATS CLI not found. Please install it first:"
        echo "   brew install nats-io/nats-tools/nats"
        exit 1
    fi
    
    print_success "Prerequisites check passed"
    
    # Check if ports are available
    print_status "Checking port availability..."
    
    if ! port_available 4222; then
        print_warning "Port 4222 is in use. Stopping existing service..."
        stop_nats
    fi
    
    if ! port_available 4443; then
        print_error "Port 4443 is in use. Please free up this port."
        exit 1
    fi
    
    if ! port_available 8222; then
        print_error "Port 8222 is in use. Please free up this port."
        exit 1
    fi
    
    print_success "Ports are available"
    
    # Stop any existing NATS server
    stop_nats
    
    # Create JetStream directory
    print_status "Setting up JetStream storage..."
    mkdir -p ./jetstream
    
    # Start NATS server
    print_status "Starting NATS server with WebSocket and JetStream support..."
    nats-server -c nats.conf > nats.log 2>&1 &
    NATS_PID=$!
    
    # Wait for NATS to be ready
    if ! wait_for_nats; then
        print_error "Failed to start NATS server. Check nats.log for details."
        exit 1
    fi
    
    # Create sample data
    create_sample_data
    
    # Show connection information
    show_connection_info
    
    # Save PID for easy management
    echo $NATS_PID > nats.pid
    print_status "NATS server PID saved to nats.pid"
    
    print_success "Setup complete! NATS server is running in the background."
    print_status "Logs are available in nats.log"
}

# Cleanup function
cleanup() {
    if [ -n "$NATS_PID" ]; then
        print_status "Cleaning up..."
        kill $NATS_PID 2>/dev/null || true
    fi
}

# Set trap for cleanup
trap cleanup EXIT

# Run main function
main "$@"
