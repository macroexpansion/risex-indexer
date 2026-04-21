# Concurrent Backfill Design

## Overview

Replace the sequential backfill in `backfill_block` with a concurrent pipeline using `tokio::task::JoinSet` for block-level parallelism and `tokio::sync::Semaphore` for RPC-level concurrency limiting. Best-effort semantics: failed hashes are skipped, and gaps are covered by the existing on-demand cache-miss fallback in the HTTP server.

## Concurrency Model

Two-level concurrency control:

- **Block level**: `JoinSet` holds up to `BACKFILL_BLOCK_CONCURRENCY` block-tasks in flight simultaneously.
- **RPC level**: A shared `Arc<Semaphore>` with `BACKFILL_CONCURRENCY` permits caps total in-flight RPC calls across all block-tasks.

```
run_backfill_worker loop:
  ┌─────────────────────────────────────────────────┐
  │  JoinSet (max BACKFILL_BLOCK_CONCURRENCY tasks)  │
  │                                                   │
  │  Block N    Block N-1    Block N-2   ...          │
  │    │           │            │                     │
  │    ├─hash A───┤ ├─hash D──┤ ...                   │
  │    ├─hash B───┤ ├─hash E──┤                       │
  │    └─hash C───┘ └─hash F──┘                       │
  │         ↕ semaphore permits                       │
  │  ┌─────────────────────────────────────┐         │
  │  │  Semaphore(BACKFILL_CONCURRENCY)     │         │
  │  │  limits total in-flight RPC calls    │         │
  │  └─────────────────────────────────────┘         │
  └─────────────────────────────────────────────────┘
```

## Per-Block Task Logic

Each block-task runs as `backfill_block_concurrent`:

1. Fetch block via `get_block_by_number(block_number)` — no semaphore needed (1 call per block-task). If block not found, log error and return.
2. Extract tx hashes. If empty, return.
3. For each hash, spawn a sub-task in a local `JoinSet`:
   - Acquire semaphore permit (await)
   - Fetch tx and receipt concurrently via `tokio::join!`, each RPC call wrapped in a permit-acquiring helper
   - Send results to writer channel; skip on error
4. Await the local `JoinSet` (all hash sub-tasks finish)
5. Return `Ok(())`

**Error handling**: Each per-hash sub-task handles its own errors. Failed tx or receipt fetch → log + skip. The block-task only fails if `get_block_by_number` fails (whole block skipped, no retry — cursor already advanced, best-effort).

**Permit-acquiring helper**:

```rust
async fn rpc_with_permit<F, T>(semaphore: &Semaphore, f: F) -> Option<T>
where
    F: Future<Output = Result<Option<T>, UpstreamError>>,
{
    let _permit = semaphore.acquire().await.ok()?;
    match f.await {
        Ok(Some(val)) => Some(val),
        Ok(None) => {
            tracing::warn!("RPC returned not found");
            None
        }
        Err(e) => {
            tracing::error!(error = %e, "RPC call failed");
            None
        }
    }
}
```

Each RPC call (`get_transaction_by_hash`, `get_transaction_receipt`) is invoked through this helper, so tx + receipt for the same hash run concurrently (each grabs a permit independently). Both return `Result<Option<Value>, UpstreamError>` — `Ok(None)` means not found upstream (logged as warn), `Err` means transport/RPC error (logged as error). Both cases result in a skip.

## Cursor Management

Best-effort: cursor decremented after dispatching a block to the JoinSet, not after completion. `StoreBackfillCursor` sent to writer channel at dispatch time. No watermark tracking — if a hash fetch fails, the HTTP server's cache-miss fallback covers it on demand.

## Configuration

| Env Var | Default | Purpose |
|---|---|---|
| `BACKFILL_BLOCK_CONCURRENCY` | 4 | Max block-tasks in JoinSet |
| `BACKFILL_CONCURRENCY` | 16 | Semaphore permits (max in-flight RPC calls) |

Add both fields to `Config` struct in `src/config.rs`.

## Worker Loop

```
run_backfill_worker(db, client, sender, delay, max_blocks, cancel):
  1. Init cursor (same as current: read from db, else fetch latest block)
  2. Create Arc<Semaphore> with BACKFILL_CONCURRENCY permits
  3. Create JoinSet for block-tasks
  4. loop:
     a. While JoinSet.len() < BACKFILL_BLOCK_CONCURRENCY
           && cursor > 0 && filled < max_blocks:
        - Spawn backfill_block_concurrent into JoinSet
        - Send StoreBackfillCursor(cursor) to writer
        - Decrement cursor, increment filled
     b. tokio::select!:
        - JoinSet.join_next() → task completed, log result
        - cancel.cancelled() → abort JoinSet, break
     c. Apply delay between dispatch rounds (respecting cancel)
  5. Wait for remaining JoinSet tasks, then return
```

**Shutdown**: On cancellation, abort all JoinSet tasks. Semaphore permits release on drop. Cursor already persisted per dispatch.

## Changes to Existing Code

- **`src/backfill.rs`**: Replace `backfill_block` with `backfill_block_concurrent`. Rewrite `run_backfill_worker` loop per above. Add `rpc_with_permit` helper.
- **`src/config.rs`**: Add `backfill_block_concurrency: u64` and `backfill_concurrency: u64` fields with env var parsing.
- **`src/main.rs`**: Pass new config fields to `run_backfill_worker` (signature change).

No changes to `rpc.rs`, `writer.rs`, `db.rs`, `cache.rs`, `server.rs`, or `shred.rs`.
