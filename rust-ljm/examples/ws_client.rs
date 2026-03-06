use std::{fs::File, io::Write, path::PathBuf};

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::main]
async fn main() -> Result<()> {
    let ws_url =
        std::env::var("EXPORT_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:9001/export".into());
    let parquet_dir = std::env::var("PARQUET_DIR").unwrap_or_else(|_| "parquet".into());
    let output_path = PathBuf::from("target/export_test.csv");

    println!("connecting to exporter at {ws_url}");
    println!("PARQUET_DIR={parquet_dir}");

    let (mut ws, _) = connect_async(ws_url).await?;
    let request = json!({
        "asset": 1456,
        "channels": [0, 1, 4],
        "start": "2025-10-24T08:30:00-04:00",
        "end": "2025-10-24T08:35:00-04:00",
        "format": "csv"
    });
    ws.send(Message::Text(request.to_string())).await?;

    let mut file = File::create(&output_path)?;
    let mut total_bytes = 0usize;

    while let Some(frame) = ws.next().await {
        match frame? {
            Message::Binary(data) => {
                total_bytes += data.len();
                file.write_all(&data)?;
            }
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text)?;
                if value.get("type") == Some(&serde_json::Value::String("complete".into())) {
                    break;
                }
                println!("frame: {text}");
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    ws.close(None).await.ok();
    println!("wrote {} bytes to {}", total_bytes, output_path.display());
    Ok(())
}
