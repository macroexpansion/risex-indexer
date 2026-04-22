# RISEx Indexer

A caching Ethereum JSON-RPC proxy for the RISE Testnet. Indexes transactions and receipts via a two-tier cache (in-memory LRU + RocksDB) and serves cached responses, falling back to the upstream node on cache misses.

## Architecture

- **Shred subscriber** — WebSocket subscription that receives real-time block notifications and stores inline transactions/receipts
- **Backfill worker** — Concurrent historical block indexer with configurable rate limiting
- **Two-tier cache** — Moka LRU (50K entries, 5min idle TTL) + RocksDB persistent storage
- **HTTP server** — Axum-based JSON-RPC proxy with cache-first lookup: LRU → RocksDB → upstream

Request flow for `eth_getTransactionByHash` / `eth_getTransactionReceipt`:
1. Check in-memory LRU cache → return if hit
2. Check RocksDB → promote to LRU cache, return if hit
3. Proxy to upstream → store result via writer channel, return

All other JSON-RPC methods are proxied directly to upstream.

## Usage

```sh
cargo run
```

## Configuration

All config via environment variables with defaults:

| Variable | Default | Description |
|---|---|---|
| `LISTEN_ADDR` | `0.0.0.0:8545` | HTTP server bind address |
| `UPSTREAM_RPC_URL` | `https://testnet.riselabs.xyz` | Upstream JSON-RPC endpoint |
| `UPSTREAM_WS_URL` | `wss://testnet.riselabs.xyz/ws` | Upstream WebSocket endpoint |
| `ROCKSDB_PATH` | `./data` | RocksDB data directory |
| `BACKFILL_DELAY_MIN_MS` | `50` | Min random delay per RPC call (ms) |
| `BACKFILL_DELAY_MAX_MS` | `200` | Max random delay per RPC call (ms) |
| `BACKFILL_MAX_BLOCKS` | `10` | Max blocks to backfill (empty = unlimited) |
| `BACKFILL_BLOCK_CONCURRENCY` | `4` | Max concurrent RPC calls per block |
| `BACKFILL_CONCURRENCY` | `16` | Max concurrent blocks being fetched |

## Examples

```sh
cargo run --example backfill
cargo run --example shred
```

### JSON-RPC requests

Get transaction by hash:

```sh
curl -X POST http://localhost:8545/ \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_getTransactionByHash","params":["0x380ffe34d097033db9ac37ff617de07068171b297a3091623141e4095fd44b02"]}'
```

Get transaction receipt:

```sh
curl -X POST http://localhost:8545/ \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"eth_getTransactionReceipt","params":["0x380ffe34d097033db9ac37ff617de07068171b297a3091623141e4095fd44b02"]}'
```

## Testing

```sh
cargo test
```
