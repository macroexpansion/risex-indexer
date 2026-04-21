use alloy::primitives::B256;
use rocksdb::{ColumnFamily, DB, Options};
use serde_json::Value;

const CF_TRANSACTIONS: &str = "transactions";
const CF_RECEIPTS: &str = "receipts";
const CF_METADATA: &str = "metadata";

const KEY_BACKFILL_CURSOR: &[u8] = b"backfill_cursor";

pub struct Db {
    inner: DB,
}

impl Db {
    pub fn open(path: &str) -> Result<Self, rocksdb::Error> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cf_names = [CF_TRANSACTIONS, CF_RECEIPTS, CF_METADATA];
        let db = DB::open_cf(&opts, path, cf_names)?;
        Ok(Self { inner: db })
    }

    fn cf(&self, name: &str) -> &ColumnFamily {
        self.inner.cf_handle(name).expect("column family exists")
    }

    pub fn get_transaction(&self, hash: B256) -> Result<Option<Value>, rocksdb::Error> {
        let cf = self.cf(CF_TRANSACTIONS);
        let key = hash.as_slice();
        match self.inner.get_cf(cf, key)? {
            Some(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).expect("deserialize tx"),
            )),
            None => Ok(None),
        }
    }

    pub fn put_transaction(&self, hash: B256, tx: &Value) -> Result<(), rocksdb::Error> {
        let cf = self.cf(CF_TRANSACTIONS);
        let key = hash.as_slice();
        let value = serde_json::to_vec(tx).expect("serialize tx");
        self.inner.put_cf(cf, key, value)
    }

    pub fn get_receipt(&self, hash: B256) -> Result<Option<Value>, rocksdb::Error> {
        let cf = self.cf(CF_RECEIPTS);
        let key = hash.as_slice();
        match self.inner.get_cf(cf, key)? {
            Some(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).expect("deserialize receipt"),
            )),
            None => Ok(None),
        }
    }

    pub fn put_receipt(&self, hash: B256, receipt: &Value) -> Result<(), rocksdb::Error> {
        let cf = self.cf(CF_RECEIPTS);
        let key = hash.as_slice();
        let value = serde_json::to_vec(receipt).expect("serialize receipt");
        self.inner.put_cf(cf, key, value)
    }

    pub fn get_backfill_cursor(&self) -> Result<Option<u64>, rocksdb::Error> {
        let cf = self.cf(CF_METADATA);
        match self.inner.get_cf(cf, KEY_BACKFILL_CURSOR)? {
            Some(bytes) => {
                let val = u64::from_be_bytes(bytes[..8].try_into().expect("cursor is 8 bytes"));
                Ok(Some(val))
            }
            None => Ok(None),
        }
    }

    pub fn put_backfill_cursor(&self, block_number: u64) -> Result<(), rocksdb::Error> {
        let cf = self.cf(CF_METADATA);
        let value = block_number.to_be_bytes();
        self.inner.put_cf(cf, KEY_BACKFILL_CURSOR, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_db() -> (Db, TempDir) {
        let dir = TempDir::new().expect("temp dir");
        let db = Db::open(dir.path().to_str().expect("path")).expect("open db");
        (db, dir)
    }

    #[test]
    fn transaction_roundtrip() {
        let (db, _dir) = temp_db();
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
        db.put_transaction(hash, &tx).expect("put");
        let got = db.get_transaction(hash).expect("get").expect("some");
        assert_eq!(got["hash"], tx["hash"]);
    }

    #[test]
    fn receipt_roundtrip() {
        let (db, _dir) = temp_db();
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
        db.put_receipt(hash, &receipt).expect("put");
        let got = db.get_receipt(hash).expect("get").expect("some");
        assert_eq!(got["transactionHash"], receipt["transactionHash"]);
    }

    #[test]
    fn backfill_cursor_roundtrip() {
        let (db, _dir) = temp_db();
        assert!(db.get_backfill_cursor().expect("get").is_none());
        db.put_backfill_cursor(12345).expect("put");
        assert_eq!(db.get_backfill_cursor().expect("get"), Some(12345));
        db.put_backfill_cursor(99999).expect("put");
        assert_eq!(db.get_backfill_cursor().expect("get"), Some(99999));
    }
}