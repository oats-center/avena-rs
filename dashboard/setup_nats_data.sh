#!/bin/bash

# Setup NATS KV data for avenabox_001 sensor map testing
# This script creates the necessary KV entries for the sensor map to work

echo "Setting up NATS KV data for avenabox_001..."

# Create the mapconfig with sample sensors
nats kv put avenabox_001 mapconfig '{
  "backgroundImage": "data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iODAwIiBoZWlnaHQ9IjYwMCIgdmlld0JveD0iMCAwIDgwMCA2MDAiIGZpbGw9Im5vbmUiIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyI+CjxyZWN0IHdpZHRoPSI4MDAiIGhlaWdodD0iNjAwIiBmaWxsPSIjZjNmNGY2Ii8+CjxyZWN0IHg9IjUwIiB5PSI1MCIgd2lkdGg9IjcwMCIgaGVpZ2h0PSI1MDAiIGZpbGw9IiNlNWU3ZWIiIHN0cm9rZT0iIzlmYTJhOCIgc3Ryb2tlLXdpZHRoPSIyIi8+Cjx0ZXh0IHg9IjQwMCIgeT0iMzAiIGZvbnQtZmFtaWx5PSJBcmlhbCwgc2Fucy1zZXJpZiIgZm9udC1zaXplPSIyNCIgZm9udC13ZWlnaHQ9ImJvbGQiIGZpbGw9IiMzNzQxNTEiIHRleHQtYW5jaG9yPSJtaWRkbGUiPkF2ZW5hYm94IDAwMSBGbG9vciBQbGFuPC90ZXh0Pgo8IS0tIE1haW4gQ29udHJvbCBQYW5lbCBzcGFubmluZyBmdWxsIHdpZHRoIC0tPgo8cmVjdCB4PSIxMDAiIHk9IjE1MCIgd2lkdGg9IjYwMCIgaGVpZ2h0PSIzMDAiIGZpbGw9IiNmZmZmZmYiIHN0cm9rZT0iIzM3NDE1MSIgc3Ryb2tlLXdpZHRoPSIzIi8+Cjx0ZXh0IHg9IjQwMCIgeT0iMTgwIiBmb250LWZhbWlseT0iQXJpYWwsIHNhbnMtc2VyaWYiIGZvbnQtc2l6ZT0iMTgiIGZvbnQtd2VpZ2h0PSJib2xkIiBmaWxsPSIjMzc0MTUxIiB0ZXh0LWFuY2hvcj0ibWlkZGxlIj5NYWluIENvbnRyb2wgUGFuZWw8L3RleHQ+CjwhLS0gR3JpZCBMaW5lcyAtLT4KPGRlZnM+CjxwYXR0ZXJuIGlkPSJncmlkIiB3aWR0aD0iNTAiIGhlaWdodD0iNTAiIHBhdHRlcm5Vbml0cz0idXNlclNwYWNlT25Vc2UiPgo8cGF0aCBkPSJNIDUwIDAgTCAwIDAgMCA1MCIgZmlsbD0ibm9uZSIgc3Ryb2tlPSIjZDFkNWRiIiBzdHJva2Utd2lkdGg9IjAuNSIgb3BhY2l0eT0iMC4zIi8+CjwvcGF0dGVybj4KPC9kZWZzPgo8cmVjdCB4PSI1MCIgeT0iNTAiIHdpZHRoPSI3MDAiIGhlaWdodD0iNTAwIiBmaWxsPSJ1cmwoI2dyaWQpIi8+Cjwvc3ZnPg==",
  "labjackd.1.ch1": {
    "cabinet_id": "avenabox_001",
    "labjack_name": "Main Sensor Hub",
    "serial": "1",
    "sensor_name": "Temperature Sensor A",
    "sensor_type": "Temperature",
    "x_pos": 18,
    "y_pos": 50,
    "color": "red",
    "connected_channel": "1"
  },
  "labjackd.1.ch2": {
    "cabinet_id": "avenabox_001",
    "labjack_name": "Main Sensor Hub", 
    "serial": "1",
    "sensor_name": "Pressure Sensor B",
    "sensor_type": "Pressure",
    "x_pos": 44,
    "y_pos": 33,
    "color": "blue",
    "connected_channel": "2"
  },
  "labjackd.1.ch3": {
    "cabinet_id": "avenabox_001",
    "labjack_name": "Main Sensor Hub", 
    "serial": "1",
    "sensor_name": "Temperature Sensor C",
    "sensor_type": "Temperature",
    "x_pos": 69,
    "y_pos": 58,
    "color": "green",
    "connected_channel": "3"
  }
}'

# Create sensor types configuration
nats kv put avenabox_001 sensor_types '{
  "Temperature": {
    "icon": "data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMjQiIGhlaWdodD0iMjQiIHZpZXdCb3g9IjAgMCAyNCAyNCIgZmlsbD0ibm9uZSIgeG1sbnM9Imh0dHA6Ly93d3cudzMub3JnLzIwMDAvc3ZnIj4KPHBhdGggZD0iTTEyIDJDMTMuMSAyIDE0IDIuOSAxNCA0VjEzLjVDMTUuNzggMTQuNzggMTcgMTYuNzggMTcgMTlDMTcgMjIuMzEgMTQuMzEgMjUgMTEgMjVTNSAyMi4zMSA1IDE5QzUgMTYuNzggNi4yMiAxNC43OCA4IDEzLjVWNEM4IDIuOSA4LjkgMiAxMCAySDEyWk0xMSA0VjEzLjVDOS4yMiAxNC43OCA4IDE2Ljc4IDggMTlDOCAyMS4yMSA5Ljc5IDIzIDEyIDIzUzE2IDIxLjIxIDE2IDE5QzE2IDE2Ljc4IDE0Ljc4IDE0Ljc4IDEzIDEzLjVWNEgxMVoiIGZpbGw9IiNGRjAwMDAiLz4KPC9zdmc+",
    "size_px": 50
  },
  "Pressure": {
    "icon": "data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMjQiIGhlaWdodD0iMjQiIHZpZXdCb3g9IjAgMCAyNCAyNCIgZmlsbD0ibm9uZSIgeG1sbnM9Imh0dHA6Ly93d3cudzMub3JnLzIwMDAvc3ZnIj4KPHBhdGggZD0iTTEyIDJMMTMuMDkgOC4yNkwyMCA5TDEzLjA5IDE1Ljc0TDEyIDIyTDEwLjkxIDE1Ljc0TDQgOUwxMC45MSA4LjI2TDEyIDJaIiBmaWxsPSIjMDA4OEZGIi8+Cjwvc3ZnPg==",
    "size_px": 50
  }
}'

# Create the labjackd config that matches the sensor map
nats kv put avenabox_001 labjackd.config.TEST001 '{
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
}'

echo "NATS KV data setup complete!"
echo "You can now test the sensor map at http://localhost:5173/config/sensor-map"
echo ""
echo "To verify the data was set correctly, run:"
echo "nats kv get avenabox_001 mapconfig --raw"
echo "nats kv get avenabox_001 sensor_types --raw"
echo "nats kv get avenabox_001 labjackd.config.TEST001 --raw"
