//! `web3_bridge` agent tools: quote a cross-chain bridge and execute it.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinytools::{Tool, ToolExposure, ToolResult};

use crate::crypto::service::{BridgeQuoteParams, ExecuteQuoteParams, Web3Service};
use crate::quote::to_tool_result;

/// Prepares a cross-chain bridge.
#[derive(Debug)]
pub struct Web3BridgeQuoteTool {
    service: Arc<Web3Service>,
}

/// Confirms and executes a prepared bridge.
#[derive(Debug)]
pub struct Web3BridgeExecuteTool {
    service: Arc<Web3Service>,
}

impl Web3BridgeQuoteTool {
    /// A tool over `service`.
    #[must_use]
    pub const fn new(service: Arc<Web3Service>) -> Self {
        Self { service }
    }
}

impl Web3BridgeExecuteTool {
    /// A tool over `service`.
    #[must_use]
    pub const fn new(service: Arc<Web3Service>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Tool for Web3BridgeQuoteTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[3].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[3].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::web3_tool_declarations()[3]
            .parameters
            .clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let params: BridgeQuoteParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("invalid arguments: {e}"))),
        };
        Ok(to_tool_result(self.service.quote_bridge(params).await))
    }
}

#[async_trait]
impl Tool for Web3BridgeExecuteTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[4].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[4].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::web3_tool_declarations()[4]
            .parameters
            .clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let params: ExecuteQuoteParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("invalid arguments: {e}"))),
        };
        Ok(to_tool_result(self.service.execute_quote(params).await))
    }
}
