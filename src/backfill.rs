use std::sync::Arc;
use std::time::Duration;

use rand::RngExt;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::db::Db;
use crate::rpc::{extract_tx_hashes_from_block, UpstreamClient, UpstreamError};
use crate::writer::WriterMessage;

pub async fn run_backfill_worker(
    db: Arc<Db>,
    client: UpstreamClient,
    sender: mpsc::Sender<WriterMessage>,
    (delay_min, delay_max): (Duration, Duration),
    max_blocks: Option<u64>,
    (block_concurrency, rpc_concurrency): (usize, usize),
    cancel: CancellationToken,
) {
    let cursor = match db.get_backfill_cursor() {
        Ok(Some(n)) => {
            tracing::info!(cursor = n, "resuming backfill from saved cursor");
            n
        }
        Ok(None) => match client.get_block_number().await {
            Ok(latest) => {
                tracing::info!(
                    latest,
                    "no saved cursor, starting backfill from latest block"
                );
                latest
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to fetch latest block number, backfill cannot start");
                return;
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "failed to read backfill cursor from db");
            return;
        }
    };

    let mut cursor = cursor;
    let start = cursor;
    let stop_at = max_blocks.map(|n| cursor.saturating_sub(n));
    let semaphore = Arc::new(Semaphore::new(rpc_concurrency));
    let mut join_set: JoinSet<()> = JoinSet::new();

    tracing::info!(
        cursor,
        max_blocks,
        ?stop_at,
        block_concurrency,
        rpc_concurrency,
        ?delay_min,
        ?delay_max,
        "backfill worker started"
    );

    loop {
        while join_set.len() < block_concurrency
            && cursor > 0
            && stop_at.map_or(true, |stop| cursor > stop)
        {
            if cancel.is_cancelled() {
                break;
            }

            tracing::info!(cursor, "dispatching backfill block");

            let client = client.clone();
            let task_sender = sender.clone();
            let semaphore = Arc::clone(&semaphore);
            let block_number = cursor;

            join_set.spawn(async move {
                backfill_block_concurrent(
                    &client,
                    &task_sender,
                    &semaphore,
                    delay_min,
                    delay_max,
                    block_number,
                )
                .await;
            });

            if let Err(e) = sender
                .send(WriterMessage::StoreBackfillCursor {
                    block_number: cursor,
                })
                .await
            {
                tracing::error!(error = %e, "failed to send StoreBackfillCursor");
            }

            cursor -= 1;
        }

        if cancel.is_cancelled() || join_set.is_empty() {
            break;
        }

        tokio::select! {
            result = join_set.join_next() => {
                if let Some(Err(e)) = result {
                    tracing::error!(error = %e, "backfill block task failed");
                }
            }
            _ = cancel.cancelled() => {
                break;
            }
        }
    }

    while let Some(result) = join_set.join_next().await {
        if let Err(e) = result {
            tracing::error!(error = %e, "backfill block task failed");
        }
    }

    tracing::info!(cursor, start, "backfill worker stopped");
}

async fn rpc_with_permit<F, Fut, T>(
    semaphore: &Semaphore,
    min_delay: Duration,
    max_delay: Duration,
    f: F,
) -> Option<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, UpstreamError>>,
{
    let _permit = semaphore.acquire().await.ok()?;
    let delay = rand::rng().random_range(min_delay..=max_delay);
    tokio::time::sleep(delay).await;
    match f().await {
        Ok(Some(val)) => Some(val),
        Ok(None) => None,
        Err(e) => {
            tracing::error!(error = %e, "RPC call failed");
            None
        }
    }
}

async fn backfill_block_concurrent(
    client: &UpstreamClient,
    sender: &mpsc::Sender<WriterMessage>,
    semaphore: &Arc<Semaphore>,
    delay_min: Duration,
    delay_max: Duration,
    block_number: u64,
) {
    let block = match client.get_block_by_number(block_number).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            tracing::error!(block_number, "block not found");
            return;
        }
        Err(e) => {
            tracing::error!(block_number, error = %e, "failed to fetch block");
            return;
        }
    };

    let hashes = extract_tx_hashes_from_block(&block);
    if hashes.is_empty() {
        tracing::debug!(block_number, "block has no transactions");
        return;
    }

    tracing::debug!(
        block_number,
        count = hashes.len(),
        "backfilling block transactions concurrently"
    );

    let mut hash_tasks: JoinSet<()> = JoinSet::new();

    for hash in hashes {
        let client = client.clone();
        let sender = sender.clone();
        let semaphore = Arc::clone(semaphore);

        hash_tasks.spawn(async move {
            let (tx_opt, receipt_opt) = tokio::join!(
                rpc_with_permit(&semaphore, delay_min, delay_max, || client
                    .get_transaction_by_hash(hash)),
                rpc_with_permit(&semaphore, delay_min, delay_max, || client
                    .get_transaction_receipt(hash)),
            );

            if let Some(tx) = tx_opt {
                if let Err(e) = sender
                    .send(WriterMessage::StoreTransaction { hash, tx })
                    .await
                {
                    tracing::error!(%hash, error = %e, "failed to send StoreTransaction");
                }
            } else {
                tracing::warn!(%hash, block_number, "transaction not found or fetch failed");
            }

            if let Some(receipt) = receipt_opt {
                if let Err(e) = sender
                    .send(WriterMessage::StoreReceipt { hash, receipt })
                    .await
                {
                    tracing::error!(%hash, error = %e, "failed to send StoreReceipt");
                }
            } else {
                tracing::warn!(%hash, block_number, "receipt not found or fetch failed");
            }
        });
    }

    while let Some(result) = hash_tasks.join_next().await {
        if let Err(e) = result {
            tracing::error!(error = %e, "hash sub-task failed");
        }
    }
}
