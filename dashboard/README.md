# Avena-OTR LabJack Management Dashboard

A modern web dashboard for managing LabJack device configurations in the Avena-OTR system. Built with SvelteKit and designed for comprehensive LabJack device management through NATS JetStream.

## Features

- **LabJack Configuration Management**: Create, edit, and delete LabJack device configurations
- **NATS Integration**: Direct integration with NATS JetStream for configuration storage
- **Comprehensive Settings**: Configure all LabJack parameters including sensor settings, channels, and data formats
- **Real-time Updates**: Immediate reflection of configuration changes
- **Secure Authentication**: NATS credentials-based authentication
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