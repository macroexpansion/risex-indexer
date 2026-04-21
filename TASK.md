# RISEx Indexer

# Context

- Generally, Ethereum JSON-RPC is slow and expensive to scale.
- We want to serve as many static & historical requests from cache & indexed database as possible, and avoid hitting nodes at all costs.

---

# Requirements

- Write a Rust indexer indexing RISE Testnet’s transactions and receipts:
    - Indexing real-time transactions with [shreds](https://docs.risechain.com/docs/builders/shreds).
    - Best-effort backfilling for historical transactions.
- Write a Rust HTTP server that can serve Ethereum JSON-RPC:
    - Try to serve `eth_getTransactionByHash` and `eth_getTransactionReceipt` from a cache & indexed database.
    - Otherwise, fallback to https://testnet.riselabs.xyz and index the result on demand.
    - For other RPC methods, fallback to https://testnet.riselabs.xyz.
- We only need a Proof-of-Concept (4~8 hours), not a production-grade output!
- Reorg handling, testing, benchmarking, and monitoring are out of scope.
