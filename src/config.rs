use std::env;

pub struct Config {
    pub listen_addr: String,
    pub upstream_rpc_url: String,
    pub upstream_ws_url: String,
    pub rocksdb_path: String,
    pub backfill_delay_min_ms: u64,
    pub backfill_delay_max_ms: u64,
    pub backfill_max_blocks: Option<u64>,
    pub backfill_block_concurrency: usize,
    pub backfill_concurrency: usize,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            listen_addr: env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8545".into()),
            upstream_rpc_url: env::var("UPSTREAM_RPC_URL")
                .unwrap_or_else(|_| "https://testnet.riselabs.xyz".into()),
            upstream_ws_url: env::var("UPSTREAM_WS_URL")
                .unwrap_or_else(|_| "wss://testnet.riselabs.xyz/ws".into()),
            rocksdb_path: env::var("ROCKSDB_PATH").unwrap_or_else(|_| "./data".into()),
            backfill_delay_min_ms: env::var("BACKFILL_DELAY_MIN_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),
            backfill_delay_max_ms: env::var("BACKFILL_DELAY_MAX_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(200),
            backfill_max_blocks: env::var("BACKFILL_MAX_BLOCKS")
                .ok()
                .and_then(|v| v.parse().ok()),
            backfill_block_concurrency: env::var("BACKFILL_BLOCK_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
            backfill_concurrency: env::var("BACKFILL_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(16),
        }
    }
}
