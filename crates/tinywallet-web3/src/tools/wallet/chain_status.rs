//! `wallet_chain_status`: which chains have an account and a provider.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinytools::{Tool, ToolExposure, ToolResult};

use crate::crypto::wallet::WalletEngine;
use crate::quote::to_tool_result;

/// Lists chain readiness.
#[derive(Debug)]
pub struct WalletChainStatusTool {
    engine: Arc<WalletEngine>,
}

impl WalletChainStatusTool {
    /// A tool over `engine`.
    #[must_use]
    pub const fn new(engine: Arc<WalletEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for WalletChainStatusTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::wallet_tool_declarations()[1].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::wallet_tool_declarations()[1].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::wallet_tool_declarations()[1]
            .parameters
            .clone()
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        Ok(to_tool_result(self.engine.chain_status().await))
    }
}
