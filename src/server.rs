use std::sync::Arc;

use alloy::primitives::B256;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::cache::AppCache;
use crate::db::Db;
use crate::rpc::UpstreamClient;
use crate::writer::WriterMessage;

#[derive(Clone)]
pub struct AppState {
    pub cache: Arc<AppCache>,
    pub db: Arc<Db>,
    pub client: UpstreamClient,
    pub sender: mpsc::Sender<WriterMessage>,
}

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub fn create_app(state: AppState) -> axum::Router {
    axum::Router::new().route("/", post(handle_rpc)).with_state(state)
}

async fn handle_rpc(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return axum::Json(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32700, "message": format!("parse error: {e}") }
            }));
        }
    };

    if parsed.is_array() {
        let result = state.client.proxy_raw(&body).await;
        return axum::Json(result);
    }

    let req: JsonRpcRequest = match serde_json::from_value(parsed) {
        Ok(r) => r,
        Err(e) => {
            return axum::Json(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32600, "message": format!("invalid request: {e}") }
            }));
        }
    };

    let response = match req.method.as_str() {
        "eth_getTransactionByHash" => handle_get_transaction_by_hash(&state, &req).await,
        "eth_getTransactionReceipt" => handle_get_transaction_receipt(&state, &req).await,
        _ => handle_proxy(&state, &req, &body).await,
    };

    axum::Json(response)
}

async fn handle_get_transaction_by_hash(state: &AppState, req: &JsonRpcRequest) -> Value {
    let hash = match extract_hash(req) {
        Some(h) => h,
        None => return error_response(&req.id, -32602, "invalid params: expected transaction hash"),
    };

    if let Some(tx) = state.cache.get_transaction(&hash) {
        tracing::info!(%hash, "cache hit for transaction");
        return success_response(&req.id, tx);
    }

    tracing::info!(%hash, "cache miss for transaction, checking db");

    if let Ok(Some(tx)) = state.db.get_transaction(hash) {
        tracing::info!(%hash, "db hit for transaction, promoting to cache");
        state.cache.insert_transaction(hash, tx.clone());
        return success_response(&req.id, tx);
    }

    tracing::info!(%hash, "db miss for transaction, fetching from upstream");

    match state.client.get_transaction_by_hash(hash).await {
        Ok(Some(tx)) => {
            let _ = state.sender.try_send(WriterMessage::StoreTransaction { hash, tx: tx.clone() });
            success_response(&req.id, tx)
        }
        Ok(None) => success_response(&req.id, Value::Null),
        Err(e) => error_response(&req.id, -32603, &format!("upstream error: {e}")),
    }
}

async fn handle_get_transaction_receipt(state: &AppState, req: &JsonRpcRequest) -> Value {
    let hash = match extract_hash(req) {
        Some(h) => h,
        None => return error_response(&req.id, -32602, "invalid params: expected transaction hash"),
    };

    if let Some(receipt) = state.cache.get_receipt(&hash) {
        tracing::info!(%hash, "cache hit for receipt");
        return success_response(&req.id, receipt);
    }

    tracing::info!(%hash, "cache miss for receipt, checking db");

    if let Ok(Some(receipt)) = state.db.get_receipt(hash) {
        tracing::info!(%hash, "db hit for receipt, promoting to cache");
        state.cache.insert_receipt(hash, receipt.clone());
        return success_response(&req.id, receipt);
    }

    tracing::info!(%hash, "db miss for receipt, fetching from upstream");

    match state.client.get_transaction_receipt(hash).await {
        Ok(Some(receipt)) => {
            let _ = state.sender.try_send(WriterMessage::StoreReceipt { hash, receipt: receipt.clone() });
            success_response(&req.id, receipt)
        }
        Ok(None) => success_response(&req.id, Value::Null),
        Err(e) => error_response(&req.id, -32603, &format!("upstream error: {e}")),
    }
}

async fn handle_proxy(state: &AppState, _req: &JsonRpcRequest, body: &[u8]) -> Value {
    let result = state.client.proxy_raw(body).await;
    match result.get("result") {
        Some(_) => result,
        None => result,
    }
}

fn extract_hash(req: &JsonRpcRequest) -> Option<B256> {
    let params = req.params.as_ref()?;
    let arr = params.as_array()?;
    let hash_str = arr.first()?.as_str()?;
    hash_str.parse().ok()
}

fn success_response(id: &Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}