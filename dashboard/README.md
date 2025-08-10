# Avena-OTR Dashboard

A modern web-based dashboard for configuring and monitoring LabJack devices inside Avena cabinet boxes located alongside highways. This system provides real-time configuration management for sensors deployed under highway roads, enabling comprehensive infrastructure monitoring and data collection.

## 🚀 Overview

The Avena-OTR (Off-The-Road) Dashboard is a sophisticated web application built with **SvelteKit** that provides centralized management for roadside infrastructure monitoring systems. It connects to **NATS** messaging system to communicate with Avena cabinet boxes containing LabJack data acquisition devices.

### Key Features

- **🔧 LabJack Device Configuration**: Configure sampling rates, channels, gains, and data formats
- **📊 Real-time Monitoring**: Live status monitoring of cabinet boxes and devices
- **🛠️ Maintenance Mode**: Controlled access during maintenance operations
- **🌐 WebSocket Communication**: Real-time updates via NATS WebSocket interface
- **📱 Responsive Design**: Modern UI that works on desktop and mobile devices

## 🏗️ Architecture

```
┌─────────────────┐    WebSocket    ┌─────────────────┐    NATS    ┌─────────────────┐
│   Dashboard     │ ──────────────► │   NATS Server   │ ─────────► │  Avena Cabinet  │
│   (SvelteKit)   │                 │   (JetStream)   │            │   (LabJack)     │
└─────────────────┘                 └─────────────────┘            └─────────────────┘
```

### Technology Stack

- **Frontend**: SvelteKit 5.0, TypeScript, Tailwind CSS, DaisyUI
- **Backend**: NATS Server with JetStream for persistent storage
- **Communication**: WebSocket over NATS for real-time updates
- **Data Storage**: NATS Key-Value store for configuration persistence

## 🚀 Quick Start

### Prerequisites

- **Node.js** (v18 or higher)
- **NATS Server** (installed via Homebrew or download)

### Installation

1. **Clone the repository**
   ```bash
   git clone <repository-url>
   cd avena-rs/dashboard
   ```

2. **Install dependencies**
   ```bash
   npm install
   ```

3. **Setup NATS Server**
   ```bash
   # Make setup script executable
   chmod +x setup_nats.sh
   
   # Run NATS setup (creates server and sample data)
   ./setup_nats.sh
   ```

4. **Start the dashboard**
   ```bash
   npm run dev
   ```

5. **Access the dashboard**
   - Open browser: `http://localhost:5173`
   - Login with WebSocket URL: `ws://localhost:4443`
   - Password: (leave empty for local development)

## 📋 Dashboard Features

### 1. Cabinet Selection
- View all available Avena cabinet boxes
- Real-time status monitoring (Online/Offline/Maintenance)
- Quick access to device configurations

### 2. LabJack Configuration
- **Device Management**: Add, edit, and delete LabJack devices
- **Channel Configuration**: Configure up to 8 analog/digital channels
- **Sensor Settings**: Set sampling rates, gains, and data formats
- **Status-aware**: Different access levels based on cabinet status

### 3. Sensor Mapping
TODO - Unclear

### 4. Cabinet Status Management
- **Status Control**: Set cabinets to Online/Offline/Maintenance modes
- **Access Control**: Restrict modifications during maintenance
- **Real-time Updates**: Live status changes across the system

## 🔧 Configuration

### NATS Server Configuration

The dashboard uses a custom NATS configuration (`nats.conf`) with:

- **WebSocket Support**: Port 4443 for browser connections
- **JetStream**: Persistent storage for configurations
- **HTTP Monitor**: Port 8222 for server monitoring
- **NATS Core**: Port 4222 for standard NATS clients

### Sample Data Structure

The setup script creates sample data including:

```json
{
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
}
```

## 🛠️ Development

### Project Structure

```
dashboard/
├── src/
│   ├── lib/
│   │   ├── components/          # Reusable UI components
│   │   ├── nats.svelte.ts       # NATS communication utilities
│   │   └── MapTypes.ts          # TypeScript type definitions
│   ├── routes/
│   │   ├── +page.svelte         # Login page
│   │   └── config/              # Configuration pages
│   │       ├── cabinet-select/   # Cabinet selection
│   │       ├── lj-config/        # LabJack configuration
│   │       ├── sensor-map/       # Sensor mapping interface
│   │       └── cabinet-status/   # Status management
│   └── app.css                  # Global styles
├── nats.conf                    # NATS server configuration
├── setup_nats.sh               # NATS setup script
└── package.json                # Dependencies and scripts
```

### Available Scripts

```bash
npm run dev          # Start development server
npm run build        # Build for production
npm run preview      # Preview production build
```

### NATS Management

```bash
./setup_nats.sh              # Start NATS server with sample data
./setup_nats.sh status       # Check NATS server status
./cleanup_nats.sh            # Stop and cleanup NATS server
nats kv ls                   # List all buckets
nats kv keys bucket_name     # View keys in a bucket
```

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Test thoroughly with NATS server
5. Submit a pull request

## 📄 License

This project is licensed under the terms specified in the main repository license.
