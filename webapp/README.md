# Avena-OTR Web Dashboard

A modern SvelteKit-based web dashboard for monitoring and managing LabJack data acquisition systems in roadside infrastructure applications.

## Overview

The Avena-OTR dashboard provides real-time monitoring and configuration management for LabJack devices deployed in roadside infrastructure. It features a clean, responsive interface built with SvelteKit, TailwindCSS, and DaisyUI, with real-time data streaming via NATS WebSocket connections.

## Prerequisites

- **Node.js** (v18 or higher)
- **pnpm** package manager
- **NATS server** with JetStream support
- **NATS CLI tools** (for setup script)

### Installing Prerequisites

```bash
# Install Node.js (if not already installed)
# Visit https://nodejs.org/ or use a version manager like nvm

# Install pnpm
npm install -g pnpm
```

## Quick Start

### 1. Install Dependencies

```bash
cd webapp
pnpm install
```

### 2. Start Development Server

```bash
pnpm run dev
```

### 3. Access the Dashboard

Open your browser and navigate to: `http://localhost:5173`


## Usage

### 1. Login

1. Enter the NATS WebSocket server URL (example: `ws://abc.com:8424`)
2. Upload your NATS credentials file
3. Click "Connect with Credentials"

### 2. LabJack Management

- View connected LabJack devices
- Configure sensor settings

### 3. Real-time Monitoring

- Live data visualization with interactive charts
- Automatic data downsampling for performance
- Trigger mode like oscilloscope and freezing plots


### Key Dependencies

- `@nats-io/nats-core` - NATS client
- `flatbuffers` - Data serialization
- `downsample-lttb` - Data downsampling
- `daisyui` - UI component library
