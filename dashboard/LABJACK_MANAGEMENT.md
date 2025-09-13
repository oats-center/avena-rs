# LabJack Management Dashboard

This Svelte application provides a comprehensive interface for managing LabJack device configurations stored in NATS JetStream's avenabox bucket.

## Features

### 🔐 Authentication
- Secure NATS server connection with credentials
- Session-based authentication
- Automatic redirect to login if not authenticated

### 📋 LabJack Management
- **View All LabJacks**: Display all LabJack configurations from the avenabox bucket
- **Add New LabJack**: Create new LabJack device configurations
- **Edit Configuration**: Modify existing LabJack settings and sensor parameters
- **Delete LabJack**: Remove LabJack configurations from the bucket

### ⚙️ Configuration Options

#### Basic Settings
- **LabJack Name**: Unique identifier for the device (must be unique, case-insensitive)
- **Asset Number**: Asset tracking number (must be unique)
- **Max Channels**: Maximum number of channels (1-16)
- **Rotate Interval**: Data rotation interval in seconds
- **NATS Subject**: NATS subject for data publishing
- **NATS Stream**: NATS stream name

#### Sensor Settings
- **Scan Rate**: Data scanning frequency in Hz
- **Sampling Rate**: Data sampling frequency in Hz
- **Gains**: Signal amplification factor
- **LabJack Status**: Online/Offline status
- **Enabled Channels**: Select which channels are active (0 to max_channels-1)
- **Data Formats**: Choose from voltage, temperature, pressure, current, resistance
- **Measurement Units**: Select appropriate units (V, °C, PSI, A, Ω, Pa, kPa, bar)

## Usage

### 1. Login
1. Navigate to the application
2. Enter your NATS server URL (e.g., `ws://localhost:4443`)
3. Upload your NATS credentials file (.creds)
4. Click "Connect with Credentials"

### 2. Manage LabJacks
1. **View LabJacks**: All configured LabJacks are displayed in a grid layout
2. **Add New**: Click "Add New LabJack" to create a new configuration
3. **Edit**: Click the edit icon on any LabJack card to modify settings
4. **Delete**: Click the delete icon to remove a LabJack configuration

### 3. Configuration Modal
- Fill in all required fields (marked with *)
- **LabJack Name**: Must be unique (case-insensitive)
- **Asset Number**: Must be unique across all LabJacks
- Select enabled channels from the available range
- Choose data formats and measurement units
- Set sensor parameters
- Click "Save Changes" or "Add LabJack" to persist changes

## Technical Details

### File Structure
```
src/
├── routes/
│   ├── +page.svelte          # Login page
│   ├── +layout.svelte        # Authentication layout
│   └── labjacks/
│       └── +page.svelte      # LabJack management page
├── lib/
│   ├── nats.svelte.ts        # NATS connection and CRUD operations
│   └── components/
│       └── LabJackConfigModal.svelte  # Configuration modal
```

### NATS Operations
- **Read**: Fetches all keys from avenabox bucket
- **Create**: Adds new LabJack configuration
- **Update**: Modifies existing configuration
- **Delete**: Removes configuration from bucket

### Data Format
LabJack configurations are stored as JSON in the avenabox bucket with keys in the format `labjackd.config.{labjack_name}` (where labjack_name is lowercase) and the following structure:

**Example Key**: `labjackd.config.macbook`
```json
{
  "labjack_name": "device_name",
  "asset_number": 1234,
  "max_channels": 8,
  "nats_subject": "avenabox",
  "nats_stream": "labjacks",
  "rotate_secs": 60,
  "sensor_settings": {
    "scan_rate": 200,
    "sampling_rate": 1000,
    "channels_enabled": [0, 1, 2],
    "gains": 1,
    "data_formats": ["voltage", "temperature", "pressure"],
    "measurement_units": ["V", "°C", "PSI"],
    "labjack_on_off": false
  }
}
```

## Development

### Prerequisites
- Node.js and pnpm
- NATS server with JetStream enabled
- NATS credentials file

### Running the Application
```bash
cd dashboard
pnpm install
pnpm dev
```

### Building for Production
```bash
pnpm build
pnpm preview
```

## Security Notes
- Credentials are stored in sessionStorage (cleared on browser close)
- All NATS operations require valid authentication
- Input validation prevents invalid configurations
- Confirmation dialogs for destructive operations

## Browser Support
- Modern browsers with ES6+ support
- WebSocket support required for NATS connection
- Responsive design works on desktop and mobile devices
