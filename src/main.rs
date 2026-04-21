use std::sync::Arc;
use std::time::Duration;

use tokio::signal;
use tokio_util::sync::CancellationToken;

use risex_indexer::{backfill, cache, config, db, rpc, server, shred, writer};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("risex_indexer=info".parse().unwrap()),
        )
        .init();

    let config = config::Config::from_env();
    tracing::info!(?config.listen_addr, ?config.upstream_rpc_url, ?config.upstream_ws_url, ?config.rocksdb_path, config.backfill_delay_min_ms, config.backfill_delay_max_ms, ?config.backfill_max_blocks, config.backfill_block_concurrency, config.backfill_concurrency, "starting risex-indexer");

    let db = Arc::new(db::Db::open(&config.rocksdb_path).expect("failed to open rocksdb"));
    let cache = cache::AppCache::new();
    let (sender, rx) = writer::create_writer_channel();
    let client = rpc::UpstreamClient::new(config.upstream_rpc_url.clone());

    let writer_db = Arc::clone(&db);
    let writer_cache = Arc::clone(&cache);
    tokio::spawn(async move {
        writer::run_writer_task(writer_db, writer_cache, rx).await;
    });

    let cancel = CancellationToken::new();

    let shred_sender = sender.clone();
    let ws_url = config.upstream_ws_url.clone();
    let cancel_shred = cancel.clone();
    tokio::spawn(async move {
        shred::run_shred_subscriber(ws_url, shred_sender, cancel_shred).await;
    });

    let backfill_db = Arc::clone(&db);
    let backfill_client = client.clone();
    let backfill_sender = sender.clone();
    let backfill_delay_min = Duration::from_millis(config.backfill_delay_min_ms);
    let backfill_delay_max = Duration::from_millis(config.backfill_delay_max_ms);
    let backfill_max = config.backfill_max_blocks;
    let backfill_block_concurrency = config.backfill_block_concurrency;
    let backfill_concurrency = config.backfill_concurrency;
    let cancel_backfill = cancel.clone();
    tokio::spawn(async move {
        backfill::run_backfill_worker(
            backfill_db,
            backfill_client,
            backfill_sender,
            (backfill_delay_min, backfill_delay_max),
            backfill_max,
            (backfill_block_concurrency, backfill_concurrency),
            cancel_backfill,
        )
        .await;
    });

    let app_state = server::AppState {
        cache,
        db,
        client,
        sender,
    };
    let app = server::create_app(app_state);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listener");
    tracing::info!(addr = %config.listen_addr, "HTTP server listening");

    let cancel_server = cancel.clone();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        tokio::select! {
            _ = signal::ctrl_c() => {
                tracing::info!("received ctrl+c, shutting down");
            }
            _ = cancel_server.cancelled() => {
                tracing::info!("cancellation triggered, shutting down");
            }
        }
    });

    if let Err(e) = server.await {
        tracing::error!(error = %e, "server error");
    }

    tracing::info!("cancelling background tasks...");
    cancel.cancel();

    tracing::info!("risex-indexer stopped");
}

