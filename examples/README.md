# Examples

## Backfill

Runs the backfill worker only — fetches historical blocks from the RISE testnet, stores transactions and receipts in RocksDB.

```sh
RUST_LOG=info cargo run --example backfill
```

### Hardcoded defaults

| Setting | Value |
|---|---|
| `UPSTREAM_RPC_URL` | `https://testnet.riselabs.xyz` |
| `ROCKSDB_PATH` | `./data` |
| `BACKFILL_DELAY_MS` | `100` |

Edit `examples/backfill.rs` to change these values.

Press `Ctrl+C` to stop.
