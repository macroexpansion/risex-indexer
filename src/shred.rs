use std::time::Duration;

use alloy::primitives::B256;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::writer::WriterMessage;

const INITIAL_BACKOFF_SECS: u64 = 1;
const MAX_BACKOFF_SECS: u64 = 30;

pub async fn run_shred_subscriber(
    ws_url: String,
    sender: mpsc::Sender<WriterMessage>,
    cancel: CancellationToken,
) {
    let mut backoff = INITIAL_BACKOFF_SECS;
    loop {
        if cancel.is_cancelled() {
            break;
        }
        match subscribe_shreds(&ws_url, &sender, &cancel).await {
            Ok(()) => {
                tracing::warn!("shred subscriber disconnected, reconnecting...");
                backoff = INITIAL_BACKOFF_SECS;
            }
            Err(e) => {
                tracing::error!(error = %e, "shred subscriber error");
            }
        }
        if cancel.is_cancelled() {
            break;
        }
        tracing::info!(backoff_secs = backoff, "reconnecting shred subscriber");
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(backoff)) => {
                backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
            }
            _ = cancel.cancelled() => break,
        }
    }
    tracing::info!("shred subscriber stopped");
}

async fn subscribe_shreds(
    ws_url: &str,
    sender: &mpsc::Sender<WriterMessage>,
    cancel: &CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url).await?;
    let (mut write, mut read) = ws_stream.split();

    let subscribe_msg = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_subscribe",
        "params": ["shreds"]
    });
    write
        .send(Message::Text(subscribe_msg.to_string().into()))
        .await?;

    tracing::info!("shred subscriber connected, awaiting subscription confirmation");

    while let Some(msg) = tokio::select! {
        msg = read.next() => msg,
        _ = cancel.cancelled() => None,
    } {
        let msg = msg?;
        match msg {
            Message::Text(text) => {
                if let Ok(notification) = serde_json::from_str::<Value>(&text) {
                    if is_subscription_confirmation(&notification) {
                        tracing::info!("shred subscription confirmed");
                        continue;
                    }
                    let block_number = notification
                        .get("params")
                        .and_then(|p| p.get("result"))
                        .and_then(|r| r.get("blockNumber"))
                        .and_then(|n| n.as_u64());
                    let tx_count = notification
                        .get("params")
                        .and_then(|p| p.get("result"))
                        .and_then(|r| r.get("transactions"))
                        .and_then(|t| t.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    tracing::info!(block_number, tx_count, "shred notification");
                    process_notification(&notification, sender).await;
                }
            }
            Message::Ping(data) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Message::Close(_) => {
                tracing::warn!("shred WebSocket closed by server");
                return Ok(());
            }
            _ => {}
        }
    }

    Ok(())
}

fn is_subscription_confirmation(notification: &Value) -> bool {
    notification.get("id").is_some() && notification.get("result").is_some()
}

async fn process_notification(notification: &Value, sender: &mpsc::Sender<WriterMessage>) {
    let txs = notification
        .get("params")
        .and_then(|p| p.get("result"))
        .and_then(|r| r.get("transactions"))
        .and_then(|t| t.as_array());

    let Some(txs) = txs else {
        tracing::debug!("shred notification contained no transactions array");
        return;
    };

    tracing::debug!(count = txs.len(), "processing shred transactions");

    for tx_obj in txs {
        let hash_val = match tx_obj
            .get("transaction")
            .and_then(|t| t.get("hash"))
            .and_then(|h| h.as_str())
        {
            Some(h) => h,
            None => {
                tracing::debug!("shred transaction missing hash field");
                continue;
            }
        };

        let hash: B256 = match hash_val.parse() {
            Ok(h) => h,
            Err(_) => {
                tracing::warn!(hash = hash_val, "failed to parse transaction hash");
                continue;
            }
        };

        if let Some(tx) = tx_obj.get("transaction").cloned() {
            if let Err(e) = sender
                .send(WriterMessage::StoreTransaction { hash, tx })
                .await
            {
                tracing::error!(error = %e, "failed to send StoreTransaction");
            }
        }

        if let Some(receipt) = tx_obj.get("receipt").cloned() {
            if let Err(e) = sender
                .send(WriterMessage::StoreReceipt { hash, receipt })
                .await
            {
                tracing::error!(error = %e, "failed to send StoreReceipt");
            }
        }
    }
}
