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

## 🔧 Development

### Available Scripts
```bash
npm run dev          # Start development server
npm run build        # Build for production
npm run preview      # Preview production build
npm run check        # Type check with svelte-check
npm run lint         # Lint with ESLint
```

### Project Structure
```
dashboard/
├── src/
│   ├── lib/
│   │   ├── components/          # Reusable UI components
│   │   │   ├── Alert.svelte     # Alert notifications
│   │   │   ├── SensorMap.svelte # Interactive sensor map
│   │   │   └── basic_modals/    # Modal dialogs
│   │   └── nats.svelte.ts       # NATS connection utilities
│   ├── routes/                  # SvelteKit routing
│   │   ├── +page.svelte        # Login page
│   │   └── config/             # Configuration pages
│   │       ├── cabinet-select/  # Avena box selection
│   │       ├── lj-config/       # LabJack configuration
│   │       └── sensor-map/      # Sensor visualization
│   └── app.css                 # Global styles
├── static/                     # Static assets
├── setup_nats.sh              # NATS setup script with sample data
├── cleanup_nats.sh            # NATS cleanup and management script
└── nats.conf                  # NATS configuration
```

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

## 🐛 Troubleshooting

### Common Issues

#### Connection Errors
- **"WebSocket connection failed"**: Ensure NATS is running with `--ws` flag
- **"JetStream not enabled"**: Check NATS configuration includes JetStream
- **"Port already in use"**: Use different ports or stop conflicting services

#### Data Not Loading
- **Check NATS logs**: Look for error messages
- **Verify KV stores**: Use `nats kv ls` to check buckets
- **Check browser console**: Look for JavaScript errors

#### Dashboard Issues
- **Clear browser cache**: Hard refresh (Ctrl+F5)
- **Check session storage**: Verify server URL is stored
- **Restart development server**: `npm run dev`

### Debug Mode
```bash
# Start NATS with debug logging
nats-server -c nats.conf -D

# Check dashboard console (F12 → Console tab)
# Look for connection and data loading logs
```

## 🔗 Integration

### LabJack Devices
- **Supported models**: T4, T7, T8
- **Connection types**: USB, Ethernet
- **Data formats**: Analog, digital, I2C, SPI

### External Systems
- **NATS messaging** for distributed communication
- **REST APIs** for external integrations
- **WebSocket streaming** for real-time updates

## 📚 Additional Resources

- **NATS Documentation**: https://docs.nats.io/
- **SvelteKit Guide**: https://kit.svelte.dev/
- **LabJack Documentation**: https://labjack.com/support
- **Project Issues**: Check the main repository

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## 📄 License

This project is licensed under the terms specified in the main repository license.

---

**Avena-OTR Dashboard**: Advanced Vehicle Network Architecture - Off-The-Road Monitoring System
