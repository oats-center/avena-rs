# Rust-LJM: LabJack Data Acquisition in Rust

A high-performance, async data acquisition system for LabJack devices written in Rust. This project provides real-time streaming capabilities with configurable sampling rates, multiple analog input channels, and automatic CSV data logging.

## Features

- **Real-time Data Streaming**: High-speed analog input streaming with configurable scan rates
- **Multi-channel Support**: Simultaneous sampling from multiple analog input channels
- **Dynamic Configuration**: Hot-reloadable configuration files for runtime parameter changes
- **Automatic Data Logging**: CSV output with timestamps
- **Async Architecture**: Built with Tokio for efficient, non-blocking I/O
- **LabJack Support**: Compatible with T4, T7, and T8 devices
- **Flexible Linking**: Support for both static and dynamic library linking

## Prerequisites

- Rust 1.70+ with Cargo
- LabJack device (T4, T7, or T8)
- LabJack Modbus (LJM) library
- Network connection to your LabJack device

## Installation

1. Clone the repository:
```bash
git clone <repository-url>
cd rust-ljm
```

2. Build the project:
```bash
cargo build --release
```

## Configuration

The system uses a JSON configuration file located at `config/sample.json`:

```json
{
  "scans_per_read": 1000,
  "suggested_scan_rate": 7000,
  "channels": [0, 1, 2, 3, 4]
}
```

### Configuration Parameters

- **`scans_per_read`**: Number of scans to read in each batch
- **`suggested_scan_rate`**: Target sampling rate in Hz
- **`channels`**: Array of analog input channel numbers to sample

## Usage

### Main Application

Run the main streaming application:

```bash
cargo run --release
```

The application will:
1. Connect to the first available LabJack device
2. Start streaming data from configured channels
3. Save data to CSV files in `outputs/csv/`
4. Monitor for configuration changes and restart streaming automatically
5. Gracefully shutdown on Ctrl+C

### Examples

#### Device Information
```bash
cargo run --example info
```
Displays device type, IP address, and connection details.

#### Single Channel Reading
```bash
cargo run --example read
```
Reads analog input values from AIN0 and AIN1 at ~5 Hz.

#### Basic Streaming
```bash
cargo run --example stream
```
Demonstrates basic streaming functionality with AIN0, AIN1, and AIN4.

## Output

Data is automatically saved to CSV files with the following naming convention:
```
outputs/csv/YYYY-MM-DD_HH-MM-SS_scans{scans_per_read}_rate{scan_rate}_ch{channels}.csv
```

Each CSV file contains:
- `timestamp`: ISO 8601 timestamp in New York timezone
- `values`: JSON array of analog input values for each scan

## Features

### Hot Configuration Reload

The system automatically detects configuration file changes and restarts the data stream with new parameters. This allows for runtime adjustments without stopping the application.

### Error Handling

Comprehensive error handling with graceful degradation:
- Automatic reconnection on connection loss
- Detailed error logging
- Graceful shutdown on critical errors

## Project Structure

```
rust-ljm/
├── src/
│   └── main.rs          # Main streaming application
├── examples/
│   ├── info.rs          # Device information example
│   ├── read.rs          # Single channel reading example
│   └── stream.rs        # Basic streaming example
├── config/
│   └── sample.json      # Configuration file
├── outputs/
│   └── csv/             # CSV output directory
└── Cargo.toml           # Project dependencies
```

## Dependencies

- **`ljmrs`**: Rust bindings for LabJack Modbus library
- **`tokio`**: Async runtime for high-performance I/O
- **`serde`**: Serialization/deserialization for configuration
- **`chrono`**: Timezone-aware timestamp handling
- **`csv`**: CSV file writing capabilities

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## License

TBD

## Acknowledgments

- LabJack Corporation for the LJM library
- The Rust community for excellent async ecosystem
- Contributors to the ljmrs crate
