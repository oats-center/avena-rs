# Rust-LJM: LabJack Data Acquisition in Rust

A Rust-based data acquisition system for LabJack devices with NATS integration.

## Prerequisites

- Rust (latest stable)
- LabJack LJM library
- NATS server

## Quick Start

1. Install LabJack LJM library:
2. Build the project:
   ```bash
   cargo build --release
   ```

3. Run examples:
   ```bash
   # Stream data from LabJack
   cargo run --bin streamer
   
   # Archive data to CSV
   cargo run --bin archiver
   
   # Subscribe to NATS data
   cargo run --bin subscriber
   ```

## Available Binaries

- `streamer` - Stream data from LabJack to NATS
- `archiver` - Archive data to CSV files
- `subscriber` - Subscribe to NATS data streams

## Configuration

Edit `config/macbook.json` to configure your LabJack device settings.

## Features

- Real-time data streaming
- NATS message publishing
- CSV data archiving
- FlatBuffer serialization
- Multi-channel support