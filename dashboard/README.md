# avena-rs - Highway Infrastructure Monitoring

Real-time road sensor data acquisition system for monitoring strain gauges and pressure cells embedded under highway concrete. Progressive Web App (PWA) that can be installed as a native executable.

## Architecture

```
Highway Sensors → LabJack → Rust Sampler → NATS → Parquet Files
                                    ↓
                              Svelte Dashboard
```

## Quick Start

### Prerequisites
- Node.js 18+ 
- Rust toolchain
- NATS server
- LabJack device connected to highway sensors

### Frontend
```bash
pnpm install
pnpm dev
```

### Backend
```bash
# Set environment variables
export NATS_URL="nats://localhost:4222"
export CFG_BUCKET="sampler_cfg"
export CFG_KEY="active"

# Run sampler
./target/debug/streamer

# Run parquet generator (in another terminal)
cargo run --bin store
```

### Environment Variables

**Required:**
- `NATS_URL` - NATS server address (e.g., `nats://localhost:4222`)
- `NATS_USERNAME` - NATS username (if authentication enabled)
- `NATS_PASSWORD` - NATS password (if authentication enabled)
- `CFG_BUCKET` - Configuration bucket name (default: `sampler_cfg`)
- `CFG_KEY` - Configuration key name (default: `active`)

**Example .env file (development only):**
```bash
NATS_URL=nats://your-nats-server:4222
NATS_USERNAME=your_username
NATS_PASSWORD=your_password
CFG_BUCKET=sampler_cfg
CFG_KEY=active
```

**⚠️ Security Warning:** Never commit .env files to git! Use system environment variables or secure keychains for production.

## Configuration

The system uses NATS KV store for configuration. Main config structure:

```json
{
  "scans_per_read": 200,
  "suggested_scan_rate": 7000.0,
  "channels": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
  "asset_number": 1,
  "nats_url": "nats://0.0.0.0:4222",
  "nats_subject": "labjack",
  "nats_stream": "LABJACK"
}
```

## Features

- **Real-time monitoring** of strain gauges and pressure cells
- **High-frequency sampling** up to 7kHz
- **Multi-channel support** (14+ channels)
- **Live configuration updates** via web dashboard
- **Automatic data archival** in Parquet format
- **Multi-cabinet management**
- **Progressive Web App** - installable as native executable
- **Offline-capable** dashboard for field use
- **Cross-platform** - Windows, Mac, Linux, mobile

## Troubleshooting

- **NATS Connection Failed**: Check server is running
- **Config Not Loading**: Verify KV bucket exists
- **Real-time Updates**: Check NATS permissions
