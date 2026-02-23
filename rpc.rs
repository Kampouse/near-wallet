//! NEAR RPC Client - Simplified for GPUI wallet

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

const YOCTO_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Network {
    Mainnet,
    Testnet,
}

impl Network {
    pub fn rpc_url(&self) -> &'static str {
        match self {
            Network::Mainnet => "https://rpc.mainnet.near.org",
            Network::Testnet => "https://rpc.testnet.near.org",
        }
    }
    
    pub fn indexer_url(&self) -> &'static str {
        match self {
            Network::Mainnet => "https://api.kitwallet.app",
            Network::Testnet => "https://testnet-api.kitwallet.app",
        }
    }
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Network::Mainnet => write!(f, "mainnet"),
            Network::Testnet => write!(f, "testnet"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NearBalance {
    pub available: f64,
}

impl NearBalance {
    pub fn format(&self) -> String {
        format!("{:.4} NEAR", self.available)
    }
}

#[derive(Debug, Clone)]
pub struct Transaction {
    pub hash: String,
    pub signer_id: String,
    pub receiver_id: String,
    pub amount: Option<f64>,
    pub timestamp: String,
    pub status: String,
}

impl Transaction {
    pub fn format_amount(&self) -> String {
        match self.amount {
            Some(amt) => format!("{:.4} NEAR", amt),
            None => "-".to_string(),
        }
    }
    
    pub fn format_time(&self) -> String {
        // Simplified - just return the timestamp
        self.timestamp.clone()
    }
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest<T> {
    jsonrpc: &'static str,
    id: &'static str,
    method: &'static str,
    params: T,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    message: String,
}

#[derive(Debug, Serialize)]
struct ViewAccountParams {
    request_type: &'static str,
    finality: &'static str,
    account_id: String,
}

#[derive(Debug, Deserialize)]
struct ViewStateResult {
    amount: String,
    locked: String,
}

#[derive(Debug, Deserialize)]
struct IndexerTxResponse {
    txns: Vec<IndexerTransaction>,
}

#[derive(Debug, Deserialize)]
struct IndexerTransaction {
    transaction: IndexerTxInner,
    transaction_outcome: IndexerOutcome,
}

#[derive(Debug, Deserialize)]
struct IndexerTxInner {
    hash: String,
    signer_id: String,
    receiver_id: String,
    actions: Vec<IndexerAction>,
}

#[derive(Debug, Deserialize)]
struct IndexerOutcome {
    block_timestamp: Option<String>,
    status: Option<IndexerStatus>,
}

#[derive(Debug, Deserialize)]
struct IndexerStatus {
    success_value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IndexerAction {
    transfer: Option<IndexerTransfer>,
}

#[derive(Debug, Deserialize)]
struct IndexerTransfer {
    deposit: String,
}

pub struct NearRpc {
    client: reqwest::Client,
    network: Network,
}

impl NearRpc {
    pub fn new(network: Network) -> Self {
        Self {
            client: reqwest::Client::new(),
            network,
        }
    }

    /// Fetch account balance from the NEAR RPC
    pub async fn get_account_balance(&self, account_id: &str) -> Result<NearBalance> {
        let url = self.network.rpc_url();
        
        let response = self.client
            .post(url)
            .json(&JsonRpcRequest {
                jsonrpc: "2.0",
                id: "dontcare",
                method: "query",
                params: ViewAccountParams {
                    request_type: "view_account",
                    finality: "final",
                    account_id: account_id.to_string(),
                },
            })
            .send()
            .await?;

        let result: JsonRpcResponse<ViewStateResult> = response.json().await?;
        
        let state = result.result.ok_or_else(|| {
            if let Some(error) = result.error {
                anyhow!("RPC error: {}", error.message)
            } else {
                anyhow!("Account not found")
            }
        })?;

        let amount: u128 = state.amount.parse().unwrap_or(0);
        let locked: u128 = state.locked.parse().unwrap_or(0);
        
        let available = (amount.saturating_sub(locked)) as f64 / YOCTO_NEAR as f64;

        Ok(NearBalance { available })
    }

    /// Fetch transaction history from indexer
    pub async fn get_transaction_history(&self, account_id: &str, limit: usize) -> Result<Vec<Transaction>> {
        let url = format!("{}/account/{}/activity", self.network.indexer_url(), account_id);
        
        let response = self.client
            .get(&url)
            .query(&[("limit", limit.to_string().as_str())])
            .send()
            .await?;

        let tx_response: IndexerTxResponse = response.json().await?;
        
        let transactions: Vec<Transaction> = tx_response.txns.into_iter().map(|tx| {
            let amount = tx.transaction.actions.iter()
                .filter_map(|a| a.transfer.as_ref())
                .filter_map(|t| t.deposit.parse::<u128>().ok())
                .map(|d| d as f64 / YOCTO_NEAR as f64)
                .next();
            
            Transaction {
                hash: tx.transaction.hash,
                signer_id: tx.transaction.signer_id,
                receiver_id: tx.transaction.receiver_id,
                amount,
                timestamp: tx.transaction_outcome.block_timestamp.unwrap_or_else(|| "Unknown".to_string()),
                status: if tx.transaction_outcome.status.and_then(|s| s.success_value).is_some() { "Success" } else { "Unknown" }.to_string(),
            }
        }).collect();

        Ok(transactions)
    }
}
