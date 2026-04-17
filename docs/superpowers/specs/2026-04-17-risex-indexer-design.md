# RISEx Indexer Design

## Overview

A Rust PoC indexer for RISE Testnet that caches transactions and receipts in RocksDB and serves Ethereum JSON-RPC requests from cache, falling back to the upstream node on cache misses. Real-time indexing via Shred WebSocket subscriptions; best-effort backfilling of historical blocks.

**Scope: PoC only (4-8 hours). No reorg handling, testing, benchmarking, or monitoring.**

## Architecture

Single binary with 4 async tasks communicating via channels:

```
┌─────────────────────────────────────────────────────────────┐
│                     risex-indexer (binary)                  │
│                                                             │
│  ┌──────────┐    ┌──────────┐    ┌──────────────────────┐   │
│  │ Shred    │    │ Backfill │    │  HTTP Server (Axum)   │   │
│  │ Subscriber│    │ Worker   │    │                       │   │
│  │          │    │          │    │  eth_getTransaction*  │   │
│  │  WS →    │    │  RPC →   │    │  → RocksDB lookup    │   │
│  │  parse   │    │  parse   │    │  → fallback upstream  │   │
│  │  tx+rcpt │    │  tx+rcpt │    │  → index on demand   │   │
│  └────┬─────┘    └────┬─────┘    └──────────┬───────────┘   │
│       │               │                      │               │
│       │  mpsc::channel│                      │ read + write  │
│       ▼               ▼                      │ (on miss)     │
│  ┌────────────────────────────┐              │               │
│  │    Writer Task             │◄─────────────┘               │
│  │    (single-threaded DB)    │                               │
│  │    RocksDB                 │                               │
│  └────────────────────────────┘                               │
│                                                               │
└───────────────────────────────────────────────────────────────┘
```

- **Shred Subscriber**: WebSocket connection to RISE, subscribes to shreds, parses transactions/receipts, sends to writer channel
- **Backfill Worker**: Iterates backwards from head block, fetches transactions/receipts via JSON-RPC, sends to writer channel
- **HTTP Server**: Axum server handling JSON-RPC requests; serves from RocksDB or falls back to upstream
- **Writer Task**: Single consumer of the mpsc channel; all RocksDB writes go through here for concurrency safety

## Data Model & Storage

**RocksDB column families:**

| Column Family | Key | Value | Purpose |
|---|---|---|---|
| `transactions` | tx hash (0x-prefixed hex bytes) | Postcard-serialized `Transaction` | Full transaction objects |
| `receipts` | tx hash (0x-prefixed hex bytes) | Postcard-serialized `TransactionReceipt` | Full receipt objects |
| `metadata` | `backfill_cursor` | u64 big-endian bytes | Last backfill block number processed |

**Serialization**: Postcard (compact binary serde) for stored values.

**Startup behavior**: Open RocksDB, create column families if absent, read `backfill_cursor` from metadata. If present, resume from that block number. If absent, start from current head block.

## Shred Subscriber

1. Connect to `wss://testnet.riselabs.xyz/ws`
2. Send `eth_subscribe` with `["shreds"]`
3. On each shred notification, parse JSON and extract transactions
4. **Receipts**: Build `TransactionReceipt` directly from shred data (contains status, logs, cumulativeGasUsed, etc.)
5. **Transactions**: Send receipt to writer channel immediately, then asynchronously fetch the full `Transaction` via `eth_getTransactionByHash` from upstream RPC and send that to the writer channel separately
6. On disconnect: log and reconnect with exponential backoff (1s → 2s → 4s → ... → 30s max)

The shred data is partial (no input data, no v/r/s), so the full transaction must be fetched from the upstream RPC. The receipt can be constructed from shred fields alone.

## Backfill Worker

1. Read `backfill_cursor` from metadata; if absent, fetch current block number as starting point
2. Fetch block by number, extract transaction hashes
3. For each tx hash: fetch `eth_getTransactionByHash` and `eth_getTransactionReceipt` from upstream, send to writer channel
4. Update `backfill_cursor` in metadata after each block
5. Decrement cursor and continue; configurable delay between blocks (default 100ms) to avoid hammering upstream
6. Stop at block 0 or a configurable minimum block number

**Error handling**: On RPC error, log and retry the same block after a short delay.

**Graceful shutdown**: Check `CancellationToken` between blocks.

## HTTP Server

Listen on configurable port (default 8545). Handle `POST /` for all JSON-RPC requests.

**Routing logic:**

```
incoming request → parse method
  → eth_getTransactionByHash: lookup RocksDB → found? return : proxy upstream, index result, return
  → eth_getTransactionReceipt: lookup RocksDB → found? return : proxy upstream, index result, return
  → any other method: proxy to upstream, return response as-is
```

**Cache miss indexing**: On a miss for `eth_getTransactionByHash` or `eth_getTransactionReceipt`, proxy to upstream, send the result to the writer channel for storage, then return the response.

**Batch requests**: For PoC, process single requests with full lookup/fallback logic. Pass batch requests (arrays) through to upstream unchanged.

**Configuration via environment variables:**
- `LISTEN_ADDR` (default `0.0.0.0:8545`)
- `UPSTREAM_RPC_URL` (default `https://testnet.riselabs.xyz`)
- `UPSTREAM_WS_URL` (default `wss://testnet.riselabs.xyz/ws`)
- `ROCKSDB_PATH` (default `./data`)

## Error Handling & Edge Cases

- **Shred disconnect**: Reconnect with exponential backoff (1s → 2s → 4s → ... → 30s max). Log each attempt.
- **Upstream RPC failure**: Return JSON-RPC error to client. For backfill, retry the same block after a delay.
- **RocksDB write failure**: Log and continue. Writes are idempotent (keyed by tx hash) so a missed write just means a future cache miss.
- **Channel full**: Drop messages with a log warning. Channel capacity 10,000. Dropped messages result in a future cache miss.
- **Graceful shutdown**: Shared `CancellationToken` across all tasks. On SIGTERM/SIGINT, each task finishes its current unit of work. Flush RocksDB before exit.

## Dependencies

- **axum**: HTTP framework
- **alloy**: Ethereum types (transactions, receipts, RPC serde)
- **rocksdb**: Storage backend
- **tokio**: Async runtime, channels, cancellation
- **serde + postcard**: Serialization
- **tokio-tungstenite**: WebSocket client for shred subscription
- **reqwest**: HTTP client for upstream RPC calls
- **tracing**: Structured logging

## Out of Scope

- Reorg handling
- Testing / benchmarks
- Monitoring / metrics
- Batch JSON-RPC optimization
- Transaction trace/debug methods