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
- Channel selection: the plot page renders at most 2 selected channels at a time
- Config-aware routing: if multiple configs share the same asset number, open
  the plot route with the config key query parameter, for example
  `/labjacks/plots/1001?key=labjackd.config.i69-mu1`

### Plot Routing

The configuration list is loaded from the central `avenabox` KV bucket. Asset
numbers are not guaranteed to be unique across boxes, so the plot route should
carry the KV key for disambiguation.

Example:

```text
/labjacks/plots/1001?key=labjackd.config.i69-mu1
```

### FlatBuffer WebSocket Decode

The browser receives live LabJack samples as FlatBuffer payloads over a NATS
WebSocket connection. Some browsers expose message data as `Uint8Array` slices
with non-8-byte-aligned offsets. The generated `valuesArray()` helper can throw
on those payloads when constructing a `Float64Array`.

`src/lib/flatbuffer-parser.ts` therefore:

- tries the generated `valuesArray()` fast path first
- falls back to per-element scalar extraction when the payload is misaligned

Without that fallback, some selected channels may stay at `0 pts` or grow more
slowly than others even though the NATS subjects are active.


### Key Dependencies

- `@nats-io/nats-core` - NATS client
- `flatbuffers` - Data serialization
- `downsample-lttb` - Data downsampling
- `daisyui` - UI component library
