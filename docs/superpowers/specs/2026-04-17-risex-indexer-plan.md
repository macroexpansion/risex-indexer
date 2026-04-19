# RISEx Indexer — Implementation Plan

Reference design: `docs/superpowers/specs/2026-04-17-risex-indexer-design.md`

## Step 1: Project scaffold & dependencies

- `cargo init` in repo root
- Add all dependencies to `Cargo.toml`: axum, alloy (full features for rpc/types/serde), rocksdb, tokio (full), serde, postcard, tokio-tungstenite, reqwest, moka, tracing, tracing-subscriber
- Create `src/main.rs` with a minimal `#[tokio::main]` entrypoint that prints "hello" and exits
- Create the module file structure:
  - `src/db.rs` — RocksDB wrapper
  - `src/cache.rs` — LRU cache wrapper
  - `src/shred.rs` — Shred subscriber
  - `src/backfill.rs` — Backfill worker
  - `src/server.rs` — Axum HTTP server
  - `src/writer.rs` — Writer task + channel types
  - `src/rpc.rs` — Upstream RPC client
  - `src/config.rs` — Env config

**Verify**: `cargo build` compiles cleanly.

## Step 2: Config & DB layer

**`src/config.rs`**: 
- Struct `Config` with env vars: `LISTEN_ADDR`, `UPSTREAM_RPC_URL`, `UPSTREAM_WS_URL`, `ROCKSDB_PATH`, `BACKFILL_DELAY_MS`, `PRELOAD_BLOCKS`
- Implement `Config::from_env()` with defaults

**`src/db.rs`**:
- Struct `Db` wrapping `rocksdb::DB`
- Open DB with column families: `transactions`, `receipts`, `metadata`
- Methods: `get_transaction(hash)`, `put_transaction(hash, tx)`, `get_receipt(hash)`, `put_receipt(hash, receipt)`, `get_backfill_cursor()`, `put_backfill_cursor(block_number)`
- Use `FixedSuffixBytes` or plain `&[u8]` keys — tx hashes as hex bytes

**Verify**: Unit test that opens a temp DB, writes/reads a transaction and receipt, reads/writes backfill cursor.

## Step 3: Cache layer

**`src/cache.rs`**:
- Struct `Cache` with two `moka::sync::Cache` instances: `transactions` and `receipts`
- Keyed by `B256` (alloy 32-byte hash type)
- Capacity: 50_000 entries each, TTL: 5 minutes
- Methods: `get_transaction(hash)`, `insert_transaction(hash, tx)`, `get_receipt(hash)`, `insert_receipt(hash, receipt)`
- Wrap in `Arc<Cache>` for sharing

**Verify**: Unit test that inserts/gets/evicts entries.

## Step 4: Writer task & channel types

**`src/writer.rs`**:
- Enum `WriterMessage`: `StoreTransaction { hash: B256, tx: Transaction }`, `StoreReceipt { hash: B256, receipt: TransactionReceipt }`, `StoreBackfillCursor { block_number: u64 }`
- `mpsc::channel<WriterMessage>` with capacity 10,000
- `run_writer_task` function: loops on `rx.recv()`, matches each message:
  - `StoreTransaction` → write to RocksDB + insert into LRU cache
  - `StoreReceipt` → write to RocksDB + insert into LRU cache
  - `StoreBackfillCursor` → write to metadata CF
- Takes `Db`, `Arc<Cache>`, `Receiver<WriterMessage>`

**Verify**: Integration test that sends messages through the channel and verifies DB + cache state.

## Step 5: Upstream RPC client

**`src/rpc.rs`**:
- Struct `UpstreamClient` wrapping `reqwest::Client`
- Methods:
  - `get_block_by_number(number)` → returns block with transactions
  - `get_transaction_by_hash(hash)` → returns `Transaction`
  - `get_transaction_receipt(hash)` → returns `TransactionReceipt`
  - `get_block_number()` → returns latest block number
  - `proxy_request(body: bytes)` → forwards raw JSON-RPC request, returns raw response bytes
- Use alloy's RPC type definitions for deserialization
- All methods hit `UPSTREAM_RPC_URL`

**Verify**: Manual test against `https://testnet.riselabs.xyz` fetching a known block/tx.

## Step 6: Shred subscriber

**`src/shred.rs`**:
- `run_shred_subscriber` async function
- Connect via `tokio-tungstenite` to `UPSTREAM_WS_URL`
- Send `{"jsonrpc":"2.0","id":1,"method":"eth_subscribe","params":["shreds"]}`
- Read messages, parse as JSON
- For each shred notification, extract transactions array
- For each transaction in the shred:
  1. Build a `TransactionReceipt` from the shred receipt fields, send `StoreReceipt` to writer channel
  2. Fire an async `eth_getTransactionByHash` RPC call to get the full transaction, send `StoreTransaction` to writer channel
- On disconnect: log error, reconnect with exponential backoff (1s → 30s max)
- Accept `CancellationToken` for graceful shutdown

**Verify**: Run manually, verify shreds are received and data flows to DB.

## Step 7: Backfill worker

**`src/backfill.rs`**:
- `run_backfill_worker` async function
- On startup: read `backfill_cursor` from DB. If absent, fetch current block number from upstream as starting point
- Loop: fetch block by number → extract tx hashes → for each hash, fetch tx + receipt → send `StoreTransaction`/`StoreReceipt` to writer → send `StoreBackfillCursor` → sleep configurable delay (default 100ms) → decrement cursor
- Stop at block 0 or on `CancellationToken` cancellation
- On RPC error: log, sleep, retry same block

**Verify**: Run manually, check DB fills with historical data.

## Step 8: Preloading

Add preloading to `src/main.rs` (or a `src/preload.rs` module):
- After DB + cache init, before starting tasks:
  1. Fetch latest block number from upstream
  2. For blocks (latest - PRELOAD_BLOCKS + 1)..=latest:
     - Fetch block with full transactions + each receipt
     - Send all tx/receipt pairs to writer channel
  3. Set backfill cursor to (latest - PRELOAD_BLOCKS)

**Verify**: Start indexer, verify LRU cache is populated with recent blocks.

## Step 9: HTTP server

**`src/server.rs`**:
- Axum handler for `POST /` that accepts raw JSON-RPC body
- Parse request: extract `method` and `params`
- `eth_getTransactionByHash`:
  1. Check LRU cache → if hit and not expired, return
  2. Check RocksDB → if hit, insert into LRU cache, return
  3. Proxy to upstream → send result to writer channel, return result
- `eth_getTransactionReceipt`: same flow
- All other methods: proxy raw request to upstream, return raw response
- Batch requests (arrays): pass through to upstream unchanged for PoC
- Return JSON-RPC responses with proper `jsonrpc`/`id`/`result`/`error` fields

**Verify**: `curl` test against local server for each method.

## Step 10: Wire everything in main

**`src/main.rs`**:
- Load config from env
- Open RocksDB
- Create shared `Cache` (Arc)
- Create writer channel + spawn writer task
- Create `UpstreamClient`
- Run preloading
- Spawn shred subscriber task (with cancellation token)
- Spawn backfill worker task (with cancellation token)
- Start Axum server (with cancellation token for graceful shutdown)
- On SIGTERM/SIGINT: cancel all tasks, wait for completion, flush DB

**Verify**: Full end-to-end test — start indexer, send JSON-RPC requests, verify responses match upstream or come from cache.

## Dependency order

```
Step 1 (scaffold)
  → Step 2 (config + DB)
  → Step 3 (cache)
  → Step 4 (writer)
  → Step 5 (RPC client)
  → Step 6 (shred subscriber) — depends on Steps 4, 5
  → Step 7 (backfill worker) — depends on Steps 4, 5
  → Step 8 (preloading) — depends on Steps 2, 3, 4, 5
  → Step 9 (HTTP server) — depends on Steps 2, 3, 4, 5
  → Step 10 (wire main) — depends on all above
```

Steps 6 and 7 can be developed in parallel after Step 5.
Steps 8 and 9 can be developed in parallel after Step 5.