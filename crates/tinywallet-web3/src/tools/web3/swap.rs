//! `web3_swap` agent tools: quote a single-chain swap, execute a prepared
//! quote, and list supported routes.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinytools::{Tool, ToolExposure, ToolResult};

use crate::crypto::service::{ExecuteQuoteParams, SwapQuoteParams, Web3Service};
use crate::quote::to_tool_result;

/// Prepares a single-chain swap.
#[derive(Debug)]
pub struct Web3SwapQuoteTool {
    service: Arc<Web3Service>,
}

/// Confirms and executes a prepared swap.
#[derive(Debug)]
pub struct Web3SwapExecuteTool {
    service: Arc<Web3Service>,
}

/// Lists the chains deBridge can swap and bridge between.
#[derive(Debug)]
pub struct Web3SwapRoutesTool {
    service: Arc<Web3Service>,
}

impl Web3SwapQuoteTool {
    /// A tool over `service`.
    #[must_use]
    pub const fn new(service: Arc<Web3Service>) -> Self {
        Self { service }
    }
}

impl Web3SwapExecuteTool {
    /// A tool over `service`.
    #[must_use]
    pub const fn new(service: Arc<Web3Service>) -> Self {
        Self { service }
    }
}

impl Web3SwapRoutesTool {
    /// A tool over `service`.
    #[must_use]
    pub const fn new(service: Arc<Web3Service>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Tool for Web3SwapQuoteTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[0].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[0].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::web3_tool_declarations()[0]
            .parameters
            .clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let params: SwapQuoteParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("invalid arguments: {e}"))),
        };
        Ok(to_tool_result(self.service.quote_swap(params).await))
    }
}

#[async_trait]
impl Tool for Web3SwapExecuteTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[1].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[1].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::web3_tool_declarations()[1]
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

#[async_trait]
impl Tool for Web3SwapRoutesTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[2].name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::web3_tool_declarations()[2].description
    }

    fn parameters_schema(&self) -> Value {
        tinywallet_bus::tools::web3_tool_declarations()[2]
            .parameters
            .clone()
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        Ok(to_tool_result(self.service.routes().await))
    }
}
