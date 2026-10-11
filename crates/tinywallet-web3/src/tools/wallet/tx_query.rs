//! Read-only tools for inspecting on-chain transactions by hash:
//! `wallet_tx_status`, `wallet_tx_receipt` and `wallet_lookup_tx`. All three
//! share the `{chain, hash, evmNetwork?}` input.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tinytools::{Tool, ToolExposure, ToolResult};

use crate::crypto::defaults::EvmNetwork;
use crate::crypto::wallet::{WalletChain, WalletEngine};
use crate::quote::to_tool_result;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TxQueryArgs {
    chain: WalletChain,
    #[serde(default)]
    evm_network: Option<EvmNetwork>,
    hash: String,
}

fn parse_args(args: Value) -> Result<TxQueryArgs, String> {
    serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))
}

/// Checks the lifecycle state of a transaction.
#[derive(Debug)]
pub struct WalletTxStatusTool {
    engine: Arc<WalletEngine>,
}

impl WalletTxStatusTool {
    /// A tool over `engine`.
    #[must_use]
    pub const fn new(engine: Arc<WalletEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for WalletTxStatusTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::wallet_tool_declarations()[3].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::wallet_tool_declarations()[3].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::wallet_tool_declarations()[3]
            .parameters
            .clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let args = match parse_args(args) {
            Ok(a) => a,
            Err(err) => return Ok(ToolResult::error(err)),
        };
        Ok(to_tool_result(
            self.engine
                .tx_status(args.chain, args.evm_network, &args.hash)
                .await,
        ))
    }
}

/// Fetches the receipt of a broadcast transaction.
#[derive(Debug)]
pub struct WalletTxReceiptTool {
    engine: Arc<WalletEngine>,
}

impl WalletTxReceiptTool {
    /// A tool over `engine`.
    #[must_use]
    pub const fn new(engine: Arc<WalletEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for WalletTxReceiptTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::wallet_tool_declarations()[4].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::wallet_tool_declarations()[4].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::wallet_tool_declarations()[4]
            .parameters
            .clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let args = match parse_args(args) {
            Ok(a) => a,
            Err(err) => return Ok(ToolResult::error(err)),
        };
        Ok(to_tool_result(
            self.engine
                .tx_receipt(args.chain, args.evm_network, &args.hash)
                .await,
        ))
    }
}

/// Looks up the raw transaction payload by hash.
#[derive(Debug)]
pub struct WalletLookupTxTool {
    engine: Arc<WalletEngine>,
}

impl WalletLookupTxTool {
    /// A tool over `engine`.
    #[must_use]
    pub const fn new(engine: Arc<WalletEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for WalletLookupTxTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::wallet_tool_declarations()[5].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::wallet_tool_declarations()[5].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::wallet_tool_declarations()[5]
            .parameters
            .clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let args = match parse_args(args) {
            Ok(a) => a,
            Err(err) => return Ok(ToolResult::error(err)),
        };
        Ok(to_tool_result(
            self.engine
                .lookup_tx(args.chain, args.evm_network, &args.hash)
                .await,
        ))
    }
}
