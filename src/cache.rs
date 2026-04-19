use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::B256;
use alloy::rpc::types::eth::{Transaction, TransactionReceipt};
use moka::sync::Cache as MokaCache;

const CAPACITY: usize = 50_000;
const TTL: Duration = Duration::from_secs(300);

pub struct AppCache {
    pub transactions: MokaCache<B256, Transaction>,
    pub receipts: MokaCache<B256, TransactionReceipt>,
}

impl AppCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            transactions: MokaCache::builder()
                .max_capacity(CAPACITY as u64)
                .time_to_idle(TTL)
                .build(),
            receipts: MokaCache::builder()
                .max_capacity(CAPACITY as u64)
                .time_to_idle(TTL)
                .build(),
        })
    }

    pub fn get_transaction(&self, hash: &B256) -> Option<Transaction> {
        self.transactions.get(hash)
    }

    pub fn insert_transaction(&self, hash: B256, tx: Transaction) {
        self.transactions.insert(hash, tx);
    }

    pub fn get_receipt(&self, hash: &B256) -> Option<TransactionReceipt> {
        self.receipts.get(hash)
    }

    pub fn insert_receipt(&self, hash: B256, receipt: TransactionReceipt) {
        self.receipts.insert(hash, receipt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tx() -> Transaction {
        serde_json::from_str(
            r#"{
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
            }"#,
        )
        .expect("parse tx")
    }

    fn make_receipt() -> TransactionReceipt {
        serde_json::from_str(
            r#"{
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
            }"#
        ).expect("parse receipt")
    }

    #[test]
    fn insert_and_get_transaction() {
        let cache = AppCache::new();
        let hash: B256 = "0x1111111111111111111111111111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        assert!(cache.get_transaction(&hash).is_none());
        let tx = make_tx();
        cache.insert_transaction(hash, tx);
        assert!(cache.get_transaction(&hash).is_some());
    }

    #[test]
    fn insert_and_get_receipt() {
        let cache = AppCache::new();
        let hash: B256 = "0x2222222222222222222222222222222222222222222222222222222222222222"
            .parse()
            .unwrap();
        assert!(cache.get_receipt(&hash).is_none());
        let receipt = make_receipt();
        cache.insert_receipt(hash, receipt);
        assert!(cache.get_receipt(&hash).is_some());
    }

    #[test]
    fn overwrite() {
        let cache = AppCache::new();
        let hash: B256 = "0x1111111111111111111111111111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let tx1 = make_tx();
        cache.insert_transaction(hash, tx1);
        let got = cache.get_transaction(&hash).unwrap();
        assert_eq!(*got.inner.hash(), hash);
    }
}
