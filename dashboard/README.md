# Avena-OTR Dashboard

A real-time web dashboard for monitoring and configuring roadside Avena boxes - intelligent roadside units that house compute, storage, and LabJack data acquisition devices connected to road sensors. Built with SvelteKit, this dashboard provides an intuitive interface for managing sensor configurations, monitoring Avena box status, and visualizing data streams from road infrastructure.

## 🎯 Project Overview

The Avena-OTR Dashboard is a comprehensive monitoring and configuration system designed for roadside infrastructure management. It allows operators to:

- **Monitor Avena box status** in real-time across multiple locations
- **Configure LabJack devices** with custom sensor settings, sampling rates, and data formats
- **Visualize sensor data** through interactive maps and real-time displays
- **Manage system configurations** through an intuitive web interface
- **Stream data** via NATS messaging system for distributed operations

## 🏗️ Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Road Sensors  │    │   Avena Box     │    │   NATS Server   │    ┌─────────────────┐
│   (Embedded)    │───▶│   (LabJack +    │───▶│   (JetStream)   │◀───│   Dashboard     │
│                 │    │    Compute)     │    │                 │    │   (SvelteKit)   │
└─────────────────┘    └─────────────────┘    └─────────────────┘    └─────────────────┘
```

- **Road Sensors**: Embedded sensors collecting traffic, environmental, and infrastructure data
- **Avena Box**: Intelligent roadside unit housing LabJack devices, compute, and storage
- **LabJack Devices**: Data acquisition hardware collecting sensor readings from road sensors
- **NATS Server**: Message broker with JetStream for persistent data storage
- **Dashboard**: Web interface for configuration and monitoring

## 🚀 Quick Start

### Prerequisites
- **Node.js 18+** and npm
- **NATS CLI** for data management
- **Modern web browser**

### 1. Install Dependencies
```bash
cd dashboard
npm install
```

### 2. Start NATS Server with Sample Data
```bash
# Make the scripts executable
chmod +x setup_nats.sh cleanup_nats.sh

# Run the setup script
./setup_nats.sh
```

### 3. Start Dashboard
```bash
npm run dev
```

### 4. Access Dashboard
Open your browser and navigate to: `http://localhost:5173`

## 🔐 Login Instructions

### Authentication
The dashboard connects to NATS servers for real-time data access. **No traditional username/password is required.**

### Connection Details
- **Server URL**: `ws://localhost:4443` or `ws://<nats-server>:4443` (WebSocket connection to NATS)
- **Password**: Leave empty (NATS server runs without authentication by default)

### Login Flow
1. Enter server URL
2. Leave password field empty (support not added)
3. Click "Connect"
4. Select an Avena box from the list
5. Access LabJack configuration and monitoring tools

## 🛠️ Setup Script

The `setup_nats.sh` script automatically:
- Starts NATS server with WebSocket support
- Enables JetStream for persistent storage
- Creates sample Avena box data
- Populates LabJack configurations
- Sets up Key-Value stores

### Manual Setup (Alternative)
If you prefer to set up manually:

```bash
# Start NATS with WebSocket and JetStream
nats-server -c nats.conf

# Create sample data
nats kv add all_cabinets
nats kv put all_cabinets avenabox_001 '{"status": "online"}'
nats kv add avenabox_001
nats kv put avenabox_001 labjackd.config.TEST001 '{"cabinet_id":"avenabox_001","labjack_name":"Test LabJack 1","serial":"TEST001","sensor_settings":{"sampling_rate":1000,"channels_enabled":[1,2],"gains":1,"data_formats":["voltage","voltage"],"measurement_units":["V","V"],"publish_raw_data":[true,true],"measure_peaks":[false,false],"publish_summary_peaks":false,"labjack_reset":false}}'
```

## 📊 Dashboard Features

### Avena Box Management
- **Real-time status monitoring** of roadside Avena boxes
- **Geographic visualization** of Avena box locations
- **Status tracking** (online, offline, maintenance)

### LabJack Configuration
- **Device management** with unique serial numbers
- **Sensor settings** configuration:
  - Sampling rates (1Hz - 100kHz)
  - Channel enable/disable
  - Gain settings
  - Data formats (voltage, temperature, etc.)
  - Measurement units
  - Data publishing options
- **Real-time configuration updates**
- **Hot-reload support** for runtime changes

### Data Visualization
- **Interactive sensor maps**
- **Real-time data streams**
- **Historical data viewing**
- **Alert management**

## 🗄️ Data Management

### NATS Key-Value Stores
- **`all_cabinets`**: Avena box status and metadata
- **`{avenabox_name}`**: Avena box-specific LabJack configurations
- **Real-time updates** via NATS JetStream

### Data Structure
```json
{
  "cabinet_id": "avenabox_001",
  "labjack_name": "LabJack T7",
  "serial": "T7ABC123",
  "sensor_settings": {
    "sampling_rate": 1000,
    "channels_enabled": [1, 2, 3],
    "gains": 1,
    "data_formats": ["voltage", "temperature", "pressure"],
    "measurement_units": ["V", "°C", "PSI"]
  }
}
```

## 🧹 Clearing NATS Data

### Quick Cleanup (Recommended)
```bash
# Use the cleanup script for easy management
./cleanup_nats.sh
```

### Manual Cleanup
```bash
# Stop NATS server
pkill nats-server

# Remove JetStream data directory
rm -rf ./jetstream

# Restart with setup script
./setup_nats.sh
```

### Clear Specific Data
```bash
# Remove specific Avena box
nats kv del avenabox_001

# Remove specific LabJack config
nats kv del avenabox_001 labjackd.config.TEST001

# List all buckets
nats kv ls

# List keys in a bucket
nats kv keys bucket_name
```

### Reset to Default State
```bash
# Run the setup script again
./setup_nats.sh
```

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## 📄 License

MIT License

---
