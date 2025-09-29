# NATS subject to subscribe to for data streams
export NATS_SUBJECT=avenabox

# Asset number identifier for this LabJack device
export ASSET_NUMBER=1456

# Directory where subscriber stores CSV files for data logging
# These CSV files are used by plotting scripts in the scripts/ folder
export OUTPUT_DIR=outputs

# NATS credentials file for authentication
export NATS_CREDS_FILE=apt.creds

# NATS KV bucket containing LabJack configurations
export CFG_BUCKET=avenabox

# Configuration key for this specific LabJack device
export CFG_KEY=labjackd.config.macbook

# Then run the program
cargo run --bin streamer