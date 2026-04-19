use alloy::primitives::B256;
use alloy::providers::{Provider, RootProvider};
use alloy::rpc::types::eth::Block;
use alloy::transports::TransportError;
use serde_json::{Value, json};

#[derive(Clone)]
pub struct UpstreamClient {
    provider: RootProvider,
}

impl UpstreamClient {
    pub fn new(rpc_url: String) -> Self {
        Self {
            provider: RootProvider::new_http(rpc_url.parse().expect("invalid RPC URL")),
        }
    }

    pub async fn get_block_number(&self) -> Result<u64, UpstreamError> {
        Ok(self.provider.get_block_number().await?)
    }

    pub async fn get_block_by_number(&self, number: u64) -> Result<Option<Block>, UpstreamError> {
        Ok(self
            .provider
            .get_block_by_number(number.into())
            .hashes()
            .await?)
    }

    pub async fn get_block_by_number_full(&self, number: u64) -> Result<Value, UpstreamError> {
        self.provider
            .raw_request(
                "eth_getBlockByNumber".into(),
                vec![json!(format!("0x{number:x}")), json!(true)],
            )
            .await
            .map_err(UpstreamError::Transport)
    }

    pub async fn get_transaction_by_hash(
        &self,
        hash: B256,
    ) -> Result<Option<Value>, UpstreamError> {
        let result: Value = self
            .provider
            .raw_request(
                "eth_getTransactionByHash".into(),
                vec![json!(format!("{hash:#x}"))],
            )
            .await
            .map_err(UpstreamError::Transport)?;
        if result.is_null() {
            return Ok(None);
        }
        Ok(Some(result))
    }

    pub async fn get_transaction_receipt(
        &self,
        hash: B256,
    ) -> Result<Option<Value>, UpstreamError> {
        let result: Value = self
            .provider
            .raw_request(
                "eth_getTransactionReceipt".into(),
                vec![json!(format!("{hash:#x}"))],
            )
            .await
            .map_err(UpstreamError::Transport)?;
        if result.is_null() {
            return Ok(None);
        }
        Ok(Some(result))
    }

    pub async fn proxy_request(
        &self,
        method: impl Into<String>,
        params: Vec<Value>,
    ) -> Result<Value, UpstreamError> {
        self.provider
            .raw_request(method.into().into(), params)
            .await
            .map_err(UpstreamError::Transport)
    }
}

pub fn extract_tx_hashes_from_block(block: &Block) -> Vec<B256> {
    block.transactions.hashes().collect()
}

#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    const TESTNET_RPC: &str = "https://testnet.riselabs.xyz";

    fn client() -> UpstreamClient {
        UpstreamClient::new(TESTNET_RPC.to_string())
    }

    #[tokio::test]
    async fn get_block_number() {
        let client = client();
        let block_number = client.get_block_number().await.expect("get_block_number");
        println!("latest block number: {block_number}");
        assert!(block_number > 0, "block number should be positive");
    }

    #[tokio::test]
    async fn get_block_and_tx_hashes() {
        let client = client();
        let latest = client.get_block_number().await.expect("get_block_number");
        let block = client
            .get_block_by_number(latest)
            .await
            .expect("get_block")
            .expect("block should exist");
        let hashes = extract_tx_hashes_from_block(&block);
        println!("block {latest} has {} transactions", hashes.len());
        assert!(!hashes.is_empty(), "block should have transactions");

        let tx_val = client
            .get_transaction_by_hash(hashes[0])
            .await
            .expect("get_transaction")
            .expect("tx should exist");
        let expected = format!("{:#x}", hashes[0]);
        assert_eq!(
            tx_val.get("hash").and_then(|v| v.as_str()),
            Some(expected.as_str())
        );
    }

    #[tokio::test]
    async fn get_transaction_receipt() {
        let client = client();
        let latest = client.get_block_number().await.expect("get_block_number");
        let block = client
            .get_block_by_number(latest)
            .await
            .expect("get_block")
            .expect("block should exist");
        let hashes = extract_tx_hashes_from_block(&block);
        assert!(!hashes.is_empty(), "block should have transactions");

        let receipt = client
            .get_transaction_receipt(hashes[0])
            .await
            .expect("get_receipt")
            .expect("receipt should exist");
        let expected = format!("{:#x}", hashes[0]);
        assert_eq!(
            receipt.get("transactionHash").and_then(|v| v.as_str()),
            Some(expected.as_str())
        );
    }

    #[tokio::test]
    async fn get_transaction_not_found() {
        let client = client();
        let zero_hash = B256::ZERO;
        let result = client
            .get_transaction_by_hash(zero_hash)
            .await
            .expect("call");
        assert!(result.is_none(), "nonexistent tx should return None");
    }

    #[tokio::test]
    async fn proxy_request() {
        let client = client();
        let result = client
            .proxy_request("eth_blockNumber", vec![])
            .await
            .expect("proxy");
        assert!(result.is_string(), "result should be a hex string");
    }

    #[tokio::test]
    async fn get_block_by_number_full() {
        let client = client();
        let latest = client.get_block_number().await.expect("get_block_number");
        let block = client
            .get_block_by_number_full(latest)
            .await
            .expect("get_block_full");
        assert!(
            block.get("transactions").is_some(),
            "full block should have transactions"
        );
    }
}
