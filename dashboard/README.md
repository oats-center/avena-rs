# Avena-OTR Dashboard

A modern web dashboard for managing and configuring LabJack devices in the Avena-OTR system. Built with SvelteKit and designed for real-time sensor monitoring and configuration.

## Features

- **LabJack Device Management**: Configure and monitor LabJack devices across multiple cabinets
- **Real-time Configuration**: Live updates through NATS messaging system
- **Multi-Cabinet Support**: Manage devices across different road cabinets
- **Channel Configuration**: Configure up to N channels per LabJack device
- **Status Monitoring**: Track cabinet status (online, offline, maintenance)
- **Responsive Design**: Modern UI optimized for desktop and mobile

## Getting Started

### Prerequisites

- Node.js
- pnpm
- NATS server (for development)

### Installation

1. Clone the repository:
```bash
git clone <repository-url>
cd dashboard
```

2. Install dependencies:
```bash
pnpm install
```

3. Start the development server:
```bash
pnpm dev
```

4. Open your browser to `http://localhost:5173`

### NATS Setup

For local development, you can start a NATS server:

```bash
# Start NATS server with JetStream enabled
./setup_nats.sh
```

To stop the NATS server:
```bash
./cleanup_nats.sh
```