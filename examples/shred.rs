use std::sync::Arc;

use risex_indexer::{cache, db, shred, writer};

const UPSTREAM_WS_URL: &str = "wss://testnet.riselabs.xyz/ws";
const ROCKSDB_PATH: &str = "./data";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("risex_indexer=info".parse().unwrap()),
        )
        .init();

    tracing::info!(
        UPSTREAM_WS_URL,
        ROCKSDB_PATH,
        "starting shred subscriber example"
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

    let cancel = tokio_util::sync::CancellationToken::new();

    let cancel_handle = cancel.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("received ctrl+c, stopping shred subscriber");
        cancel_handle.cancel();
    });

    shred::run_shred_subscriber(
        UPSTREAM_WS_URL.to_string(),
        sender,
        cancel,
    )
    .await;

    drop(sender_drop);
    tracing::info!("waiting for writer to flush...");
    let _ = writer_handle.await;

    tracing::info!("querying stored data...");

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

    tracing::info!("shred subscriber example stopped");
}
