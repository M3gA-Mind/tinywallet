//! `web3_dapp` agent tools: prepare a generic EVM contract call and execute it.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinytools::{Tool, ToolExposure, ToolResult};

use crate::crypto::service::{DappCallParams, ExecuteQuoteParams, Web3Service};
use crate::quote::to_tool_result;

/// Prepares a generic EVM contract call.
#[derive(Debug)]
pub struct Web3DappCallTool {
    service: Arc<Web3Service>,
}

/// Confirms and executes a prepared contract call.
#[derive(Debug)]
pub struct Web3DappExecuteTool {
    service: Arc<Web3Service>,
}

impl Web3DappCallTool {
    /// A tool over `service`.
    #[must_use]
    pub const fn new(service: Arc<Web3Service>) -> Self {
        Self { service }
    }
}

impl Web3DappExecuteTool {
    /// A tool over `service`.
    #[must_use]
    pub const fn new(service: Arc<Web3Service>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Tool for Web3DappCallTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[5].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[5].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::web3_tool_declarations()[5]
            .parameters
            .clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let params: DappCallParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("invalid arguments: {e}"))),
        };
        Ok(to_tool_result(self.service.prepare_dapp_call(params).await))
    }
}

#[async_trait]
impl Tool for Web3DappExecuteTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[6].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[6].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::web3_tool_declarations()[6]
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
