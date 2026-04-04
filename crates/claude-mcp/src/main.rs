use anyhow::Result;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

mod protocol;
mod tools;
mod updater;

use protocol::{JsonRpcRequest, JsonRpcResponse};

#[tokio::main]
async fn main() -> Result<()> {
    updater::cleanup_old_binaries();
    updater::spawn_update_check();

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);

    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            break; // EOF
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<JsonRpcRequest>(trimmed) {
            Ok(request) => handle_request(request).await,
            Err(e) => JsonRpcResponse::error(Value::Null, -32700, format!("Parse error: {e}")),
        };

        let response_json = serde_json::to_string(&response)?;
        stdout.write_all(response_json.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn handle_request(request: JsonRpcRequest) -> JsonRpcResponse {
    let id = request.id.clone();

    match request.method.as_str() {
        "initialize" => protocol::handle_initialize(id),
        "tools/list" => protocol::handle_tools_list(id),
        "tools/call" => {
            let tool_name = request
                .params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");

            let arguments = request
                .params
                .as_ref()
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(Value::Object(serde_json::Map::new()));

            match tools::dispatch(tool_name, arguments).await {
                Ok(result) => JsonRpcResponse::success(id, result),
                Err(e) => JsonRpcResponse::error(id, -32000, format!("{e}")),
            }
        }
        "notifications/initialized" | "notifications/cancelled" => {
            // Notifications don't need responses, but we'll return success if ID present
            JsonRpcResponse::success(id, Value::Object(serde_json::Map::new()))
        }
        _ => JsonRpcResponse::error(id, -32601, format!("Method not found: {}", request.method)),
    }
}
