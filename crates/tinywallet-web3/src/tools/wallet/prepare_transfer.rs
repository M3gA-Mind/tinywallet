//! `wallet_prepare_transfer`: prepare a transfer quote to confirm later.

use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, warn};
use serde_json::Value;
use tinytools::{Tool, ToolExposure, ToolResult};

use crate::crypto::execution::PrepareTransferParams;
use crate::crypto::wallet::WalletEngine;
use crate::quote::to_tool_result;

/// Prepares a cryptocurrency transfer.
#[derive(Debug)]
pub struct WalletPrepareTransferTool {
    engine: Arc<WalletEngine>,
}

impl WalletPrepareTransferTool {
    /// A tool over `engine`.
    #[must_use]
    pub const fn new(engine: Arc<WalletEngine>) -> Self {
        Self { engine }
    }
}

/// The first six and last four characters of an address, for correlation
/// without logging it whole.
fn redact_address(address: &str) -> String {
    let chars: Vec<char> = address.chars().collect();
    if chars.len() <= 10 {
        return "…".to_string();
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

#[async_trait]
impl Tool for WalletPrepareTransferTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::wallet_prepare_transfer_tool_declaration().name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::wallet_prepare_transfer_tool_declaration().description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::wallet_prepare_transfer_tool_declaration()
            .parameters
            .clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let params: PrepareTransferParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => {
                debug!("[wallet_prepare_transfer] invalid arguments: {e}");
                return Ok(ToolResult::error(format!("invalid arguments: {e}")));
            }
        };
        debug!(
            "[wallet_prepare_transfer] chain={:?} to={} amount_len={}",
            params.chain,
            redact_address(&params.to_address),
            params.amount_raw.len()
        );
        let result = self.engine.prepare_transfer(params).await;
        match &result {
            Ok(_) => debug!("[wallet_prepare_transfer] success"),
            Err(e) => warn!("[wallet_prepare_transfer] failed: {e}"),
        }
        Ok(to_tool_result(result))
    }
}
