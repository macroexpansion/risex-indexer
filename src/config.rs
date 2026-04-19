use std::env;

pub struct Config {
    pub listen_addr: String,
    pub upstream_rpc_url: String,
    pub upstream_ws_url: String,
    pub rocksdb_path: String,
    pub backfill_delay_ms: u64,
    pub preload_blocks: u64,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            listen_addr: env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8545".into()),
            upstream_rpc_url: env::var("UPSTREAM_RPC_URL")
                .unwrap_or_else(|_| "https://testnet.riselabs.xyz".into()),
            upstream_ws_url: env::var("UPSTREAM_WS_URL")
                .unwrap_or_else(|_| "wss://testnet.riselabs.xyz".into()),
            rocksdb_path: env::var("ROCKSDB_PATH").unwrap_or_else(|_| "./data".into()),
            backfill_delay_ms: env::var("BACKFILL_DELAY_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
            preload_blocks: env::var("PRELOAD_BLOCKS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
        }
    }
}
