use std::sync::Arc;
use std::time::Duration;

use risex_indexer::{backfill, cache, db, rpc, writer};

const UPSTREAM_RPC_URL: &str = "https://testnet.riselabs.xyz";
const ROCKSDB_PATH: &str = "./data";
const BACKFILL_DELAY_MIN_MS: u64 = 50;
const BACKFILL_DELAY_MAX_MS: u64 = 200;
const BACKFILL_MAX_BLOCKS: Option<u64> = Some(1);
const BACKFILL_BLOCK_CONCURRENCY: usize = 16;
const BACKFILL_CONCURRENCY: usize = 16;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("risex_indexer=info".parse().unwrap()),
        )
        .init();

    tracing::info!(
        UPSTREAM_RPC_URL,
        ROCKSDB_PATH,
        BACKFILL_DELAY_MIN_MS,
        BACKFILL_DELAY_MAX_MS,
        ?BACKFILL_MAX_BLOCKS,
        BACKFILL_BLOCK_CONCURRENCY,
        BACKFILL_CONCURRENCY,
        "starting backfill-only example"
    );

    let db = Arc::new(db::Db::open(ROCKSDB_PATH).expect("failed to open rocksdb"));
    let cache = cache::AppCache::new();
    let (sender, rx) = writer::create_writer_channel();
    let sender_drop = sender.clone();

    let writer_db = Arc::clone(&db);
    let writer_cache = Arc::clone(&cache);
    let writer_handle = tokio::spawn(async move {
        writer::run_writer_task(writer_db, writer_cache, rx).await;
    });

    let client = rpc::UpstreamClient::new(UPSTREAM_RPC_URL.to_string());
    let delay_min = Duration::from_millis(BACKFILL_DELAY_MIN_MS);
    let delay_max = Duration::from_millis(BACKFILL_DELAY_MAX_MS);
    let cancel = tokio_util::sync::CancellationToken::new();

    let cancel_handle = cancel.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("received ctrl+c, stopping backfill");
        cancel_handle.cancel();
    });

    backfill::run_backfill_worker(
        Arc::clone(&db),
        client.clone(),
        sender,
        (delay_min, delay_max),
        BACKFILL_MAX_BLOCKS,
        (BACKFILL_BLOCK_CONCURRENCY, BACKFILL_CONCURRENCY),
        cancel,
    )
    .await;

    drop(sender_drop);
    tracing::info!("waiting for writer to flush...");
    let _ = writer_handle.await;

    tracing::info!("querying backfilled data...");

    let cursor = db.get_backfill_cursor().expect("get cursor");
    tracing::info!(?cursor, "backfill cursor in db");

    let tx_count = db.iter_transactions().count();
    let receipt_count = db.iter_receipts().count();
    tracing::info!(tx_count, receipt_count, "entries in db");

    for entry in db.iter_transactions().take(5) {
        tracing::info!(%entry.hash, "tx: from={:?}, to={:?}", entry.value.get("from"), entry.value.get("to"));
    }
    for entry in db.iter_receipts().take(5) {
        tracing::info!(%entry.hash, "receipt: status={:?}, gasUsed={:?}", entry.value.get("status"), entry.value.get("gasUsed"));
    }

    tracing::info!("backfill example stopped");
}
