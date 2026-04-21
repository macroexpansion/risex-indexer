use std::sync::Arc;

use alloy::primitives::B256;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::cache::AppCache;
use crate::db::Db;

pub enum WriterMessage {
    StoreTransaction {
        hash: B256,
        tx: Value,
    },
    StoreReceipt {
        hash: B256,
        receipt: Value,
    },
    StoreBackfillCursor {
        block_number: u64,
    },
}

pub fn create_writer_channel() -> (mpsc::Sender<WriterMessage>, mpsc::Receiver<WriterMessage>) {
    mpsc::channel(10_000)
}

pub async fn run_writer_task(db: Arc<Db>, cache: Arc<AppCache>, mut rx: mpsc::Receiver<WriterMessage>) {
    tracing::info!("writer task started");
    while let Some(msg) = rx.recv().await {
        match msg {
            WriterMessage::StoreTransaction { hash, tx } => {
                if let Err(e) = db.put_transaction(hash, &tx) {
                    tracing::error!(%hash, error = %e, "failed to write transaction to db");
                } else {
                    cache.insert_transaction(hash, tx);
                }
            }
            WriterMessage::StoreReceipt { hash, receipt } => {
                if let Err(e) = db.put_receipt(hash, &receipt) {
                    tracing::error!(%hash, error = %e, "failed to write receipt to db");
                } else {
                    cache.insert_receipt(hash, receipt);
                }
            }
            WriterMessage::StoreBackfillCursor { block_number } => {
                if let Err(e) = db.put_backfill_cursor(block_number) {
                    tracing::error!(block_number, error = %e, "failed to write backfill cursor");
                }
            }
        }
    }
    tracing::info!("writer task stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (
        Arc<Db>,
        Arc<AppCache>,
        mpsc::Sender<WriterMessage>,
        mpsc::Receiver<WriterMessage>,
        TempDir,
    ) {
        let dir = TempDir::new().expect("temp dir");
        let db = Arc::new(Db::open(dir.path().to_str().expect("path")).expect("open db"));
        let cache = AppCache::new();
        let (tx, rx) = create_writer_channel();
        (db, cache, tx, rx, dir)
    }

    #[tokio::test]
    async fn store_transaction() {
        let (db, cache, sender, rx, _dir) = setup();
        let tx: Value = serde_json::json!({
            "type": "0x0",
            "nonce": "0x1",
            "gasPrice": "0x3e8",
            "gas": "0x5208",
            "to": "0x0000000000000000000000000000000000000001",
            "value": "0x64",
            "input": "0x",
            "hash": "0x1111111111111111111111111111111111111111111111111111111111111111",
            "v": "0x1c",
            "r": "0x1",
            "s": "0x2",
            "from": "0x0000000000000000000000000000000000000002"
        });
        let hash: B256 = "0x1111111111111111111111111111111111111111111111111111111111111111"
            .parse()
            .unwrap();

        sender
            .send(WriterMessage::StoreTransaction {
                hash,
                tx: tx.clone(),
            })
            .await
            .expect("send");
        drop(sender);

        run_writer_task(db, cache.clone(), rx).await;

        assert!(
            cache.get_transaction(&hash).is_some(),
            "cache should have tx"
        );
    }

    #[tokio::test]
    async fn store_receipt() {
        let (db, cache, sender, rx, _dir) = setup();
        let receipt: Value = serde_json::json!({
            "type": "0x0",
            "status": "0x1",
            "cumulativeGasUsed": "0x5208",
            "logs": [],
            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "transactionHash": "0x2222222222222222222222222222222222222222222222222222222222222222",
            "transactionIndex": "0x0",
            "blockHash": "0x3333333333333333333333333333333333333333333333333333333333333333",
            "blockNumber": "0x1",
            "gasUsed": "0x5208",
            "effectiveGasPrice": "0x3e8",
            "from": "0x0000000000000000000000000000000000000002",
            "to": "0x0000000000000000000000000000000000000001"
        });
        let hash: B256 = "0x2222222222222222222222222222222222222222222222222222222222222222"
            .parse()
            .unwrap();

        sender
            .send(WriterMessage::StoreReceipt { hash, receipt })
            .await
            .expect("send");
        drop(sender);

        run_writer_task(db, cache.clone(), rx).await;

        assert!(
            cache.get_receipt(&hash).is_some(),
            "cache should have receipt"
        );
    }

    #[tokio::test]
    async fn store_backfill_cursor() {
        let (db, cache, sender, rx, dir) = setup();
        sender
            .send(WriterMessage::StoreBackfillCursor { block_number: 42 })
            .await
            .expect("send");
        drop(sender);

        run_writer_task(db, cache, rx).await;

        let dir_path = dir.path().to_str().expect("path");
        let db2 = Db::open(dir_path).expect("reopen db");
        assert_eq!(db2.get_backfill_cursor().expect("get"), Some(42));
    }
}