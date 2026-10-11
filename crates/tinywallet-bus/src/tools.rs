//! Static declarations for the Wallet, Web3 and x402 agent tools.
//!
//! These descriptions and argument schemas are contract vocabulary. The
//! implementation crates use them when adapting their operations to
//! `tinytools::Tool`; this crate has no dependency on the tool runtime.

use std::sync::OnceLock;

use serde_json::{Value, json};

/// A tool's stable prompt-facing declaration, without an execution adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDeclaration {
    /// Stable tool name.
    pub name: &'static str,
    /// User- and model-facing description.
    pub description: &'static str,
    /// JSON Schema for the tool arguments.
    pub parameters: Value,
}

fn declaration(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> ToolDeclaration {
    ToolDeclaration {
        name,
        description,
        parameters,
    }
}

fn empty_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

fn chain_values() -> Value {
    json!(["evm", "btc", "solana", "tron"])
}

fn evm_network_values() -> Value {
    json!([
        "ethereum_mainnet",
        "base_mainnet",
        "arbitrum_one",
        "optimism_mainnet",
        "polygon_mainnet",
        "bsc_mainnet"
    ])
}

fn tx_query_parameters(verb: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "chain": {
                "type": "string",
                "enum": chain_values(),
                "description": format!("Blockchain network whose transaction to {verb}")
            },
            "hash": {
                "type": "string",
                "description": "Transaction hash / signature / txid to query"
            },
            "evmNetwork": {
                "type": "string",
                "enum": evm_network_values(),
                "description": "Optional EVM network when chain='evm'. Defaults to ethereum_mainnet."
            }
        },
        "required": ["chain", "hash"],
        "additionalProperties": false
    })
}

fn execute_quote_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "quoteId": {
                "type": "string",
                "description": "quoteId from a prior web3 quote/call."
            },
            "confirmed": {
                "type": "boolean",
                "description": "Must be true to execute."
            }
        },
        "required": ["quoteId", "confirmed"],
        "additionalProperties": false
    })
}

/// The six shipped wallet tools in their established declaration order.
#[must_use]
fn build_wallet_tool_declarations() -> [ToolDeclaration; 6] {
    [
        declaration(
            "wallet_status",
            "Check wallet configuration status — whether the wallet is set up, which chains are configured, and available accounts.",
            empty_parameters(),
        ),
        declaration(
            "wallet_chain_status",
            "List blockchain chain readiness — which chains have a configured account and RPC provider.",
            empty_parameters(),
        ),
        declaration(
            "wallet_prepare_transfer",
            "Prepare a cryptocurrency transfer. Returns a quote that must be confirmed before execution.",
            json!({
                "type": "object",
                "properties": {
                    "chain": {
                        "type": "string",
                        "enum": chain_values(),
                        "description": "Blockchain network to use"
                    },
                    "toAddress": {
                        "type": "string",
                        "description": "Destination wallet address"
                    },
                    "amountRaw": {
                        "type": "string",
                        "description": "Transfer amount in the chain's smallest unit (e.g. wei for EVM)"
                    },
                    "assetSymbol": {
                        "type": "string",
                        "description": "Asset symbol (e.g. ETH, USDC). Defaults to the native asset."
                    },
                    "evmNetwork": {
                        "type": "string",
                        "enum": evm_network_values(),
                        "description": "Optional EVM network when chain='evm'. Defaults to ethereum_mainnet."
                    }
                },
                "required": ["chain", "toAddress", "amountRaw"],
                "additionalProperties": false
            }),
        ),
        declaration(
            "wallet_tx_status",
            "Check the on-chain lifecycle state (pending / confirmed / failed / not_found) of a transaction by hash.",
            tx_query_parameters("check"),
        ),
        declaration(
            "wallet_tx_receipt",
            "Fetch the receipt of a broadcast transaction (success flag, fee, block, gas used) by hash.",
            tx_query_parameters("fetch the receipt for"),
        ),
        declaration(
            "wallet_lookup_tx",
            "Look up the raw transaction payload by hash on the target chain.",
            tx_query_parameters("look up"),
        ),
    ]
}

fn wallet_declarations() -> &'static [ToolDeclaration; 6] {
    static DECLARATIONS: OnceLock<[ToolDeclaration; 6]> = OnceLock::new();
    DECLARATIONS.get_or_init(build_wallet_tool_declarations)
}

/// The six shipped wallet tools in their established declaration order.
#[must_use]
pub fn wallet_tool_declarations() -> [ToolDeclaration; 6] {
    wallet_declarations().clone()
}

/// Finds a wallet declaration by its stable tool name.
#[must_use]
pub fn find_wallet_tool_declaration(name: &str) -> Option<&'static ToolDeclaration> {
    wallet_declarations().iter().find(|tool| tool.name == name)
}

macro_rules! wallet_declaration_accessor {
    ($function:ident, $name:literal) => {
        /// Returns the declaration for one named wallet tool.
        #[must_use]
        pub fn $function() -> &'static ToolDeclaration {
            find_wallet_tool_declaration($name)
                .expect("the named wallet declaration is part of the shipped contract")
        }
    };
}

wallet_declaration_accessor!(wallet_status_tool_declaration, "wallet_status");
wallet_declaration_accessor!(wallet_chain_status_tool_declaration, "wallet_chain_status");
wallet_declaration_accessor!(
    wallet_prepare_transfer_tool_declaration,
    "wallet_prepare_transfer"
);
wallet_declaration_accessor!(wallet_tx_status_tool_declaration, "wallet_tx_status");
wallet_declaration_accessor!(wallet_tx_receipt_tool_declaration, "wallet_tx_receipt");
wallet_declaration_accessor!(wallet_lookup_tx_tool_declaration, "wallet_lookup_tx");

/// The seven shipped swap, bridge and dapp tools in their established order.
#[must_use]
fn build_web3_tool_declarations() -> [ToolDeclaration; 7] {
    [
        declaration(
            "web3_swap_quote",
            "Prepare a single-chain crypto swap via deBridge. Returns a quote + quoteId to confirm with web3_swap_execute. For cross-chain swaps use web3_bridge_quote.",
            json!({
                "type": "object",
                "properties": {
                    "chainId": {"type": "integer", "description": "deBridge chain id (1 ETH, 56 BNB, 137 Polygon, 8453 Base, 42161 Arbitrum, 10 Optimism, 7565164 Solana)."},
                    "tokenIn": {"type": "string", "description": "Input token address (zero address for native)."},
                    "tokenInAmount": {"type": "string", "description": "Input amount in the token's smallest unit."},
                    "tokenOut": {"type": "string", "description": "Output token address."},
                    "tokenOutRecipient": {"type": "string", "description": "Optional. Defaults to the wallet's own address."},
                    "senderAddress": {"type": "string", "description": "Optional. Defaults to the wallet's own address."},
                    "slippage": {"type": "string", "description": "Optional slippage percent or 'auto' (default 'auto')."}
                },
                "required": ["chainId", "tokenIn", "tokenInAmount", "tokenOut"],
                "additionalProperties": false
            }),
        ),
        declaration(
            "web3_swap_execute",
            "Confirm and execute a prepared web3_swap quote (signs + broadcasts).",
            execute_quote_parameters(),
        ),
        declaration(
            "web3_swap_routes",
            "List the chains deBridge can swap/bridge between.",
            empty_parameters(),
        ),
        declaration(
            "web3_bridge_quote",
            "Prepare a cross-chain bridge via deBridge DLN. Returns a quote + quoteId to confirm with web3_bridge_execute. Source and destination chains must differ.",
            json!({
                "type": "object",
                "properties": {
                    "srcChainId": {"type": "integer", "description": "Source deBridge chain id."},
                    "srcChainTokenIn": {"type": "string", "description": "Source token address."},
                    "srcChainTokenInAmount": {"type": "string", "description": "Source amount in the token's smallest unit."},
                    "dstChainId": {"type": "integer", "description": "Destination deBridge chain id (must differ from source)."},
                    "dstChainTokenOut": {"type": "string", "description": "Destination token address."},
                    "dstChainTokenOutAmount": {"type": "string", "description": "Optional. 'auto' for market rate (default)."},
                    "dstChainTokenOutRecipient": {"type": "string", "description": "Optional. Defaults to the wallet's own destination address."},
                    "srcChainOrderAuthorityAddress": {"type": "string", "description": "Optional. Defaults to the wallet's source address."},
                    "dstChainOrderAuthorityAddress": {"type": "string", "description": "Optional. Defaults to the wallet's destination address."}
                },
                "required": ["srcChainId", "srcChainTokenIn", "srcChainTokenInAmount", "dstChainId", "dstChainTokenOut"],
                "additionalProperties": false
            }),
        ),
        declaration(
            "web3_bridge_execute",
            "Confirm and execute a prepared web3_bridge quote (signs + broadcasts the source-chain tx).",
            execute_quote_parameters(),
        ),
        declaration(
            "web3_dapp_call",
            "Prepare a generic EVM dapp contract call from pre-encoded calldata. Returns a quoteId to confirm with web3_dapp_execute.",
            json!({
                "type": "object",
                "properties": {
                    "contractAddress": {"type": "string", "description": "Target contract address."},
                    "calldata": {"type": "string", "description": "0x-prefixed hex calldata."},
                    "valueRaw": {"type": "string", "description": "Optional native value (smallest unit). Defaults to '0'."},
                    "evmNetwork": {
                        "type": "string",
                        "enum": evm_network_values(),
                        "description": "Optional EVM network. Defaults to ethereum_mainnet."
                    }
                },
                "required": ["contractAddress", "calldata"],
                "additionalProperties": false
            }),
        ),
        declaration(
            "web3_dapp_execute",
            "Confirm and execute a prepared web3_dapp call (signs + broadcasts).",
            execute_quote_parameters(),
        ),
    ]
}

fn web3_declarations() -> &'static [ToolDeclaration; 7] {
    static DECLARATIONS: OnceLock<[ToolDeclaration; 7]> = OnceLock::new();
    DECLARATIONS.get_or_init(build_web3_tool_declarations)
}

/// The seven shipped swap, bridge and dapp tools in their established order.
#[must_use]
pub fn web3_tool_declarations() -> [ToolDeclaration; 7] {
    web3_declarations().clone()
}

/// Finds a Web3 declaration by its stable tool name.
#[must_use]
pub fn find_web3_tool_declaration(name: &str) -> Option<&'static ToolDeclaration> {
    web3_declarations().iter().find(|tool| tool.name == name)
}

macro_rules! web3_declaration_accessor {
    ($function:ident, $name:literal) => {
        /// Returns the declaration for one named Web3 tool.
        #[must_use]
        pub fn $function() -> &'static ToolDeclaration {
            find_web3_tool_declaration($name)
                .expect("the named Web3 declaration is part of the shipped contract")
        }
    };
}

web3_declaration_accessor!(web3_swap_quote_tool_declaration, "web3_swap_quote");
web3_declaration_accessor!(web3_swap_execute_tool_declaration, "web3_swap_execute");
web3_declaration_accessor!(web3_swap_routes_tool_declaration, "web3_swap_routes");
web3_declaration_accessor!(web3_bridge_quote_tool_declaration, "web3_bridge_quote");
web3_declaration_accessor!(web3_bridge_execute_tool_declaration, "web3_bridge_execute");
web3_declaration_accessor!(web3_dapp_call_tool_declaration, "web3_dapp_call");
web3_declaration_accessor!(web3_dapp_execute_tool_declaration, "web3_dapp_execute");

/// The x402 HTTP payment tool declaration.
#[must_use]
fn build_x402_tool_declaration() -> ToolDeclaration {
    declaration(
        "x402_request",
        "Make an HTTP request to an x402-payable API endpoint. Automatically handles the HTTP 402 payment challenge by signing a payment (EVM EIP-3009 on Base/Ethereum, or Solana SPL transfer) with the wallet and retrying with the payment proof. Returns the API response after payment. Use this for x402-enabled APIs like twit.sh. The wallet must be set up with USDC on the target chain.",
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL of the x402-payable API endpoint (e.g. https://x402.twit.sh/tweets/by/id?id=1110302988)"
                },
                "method": {
                    "type": "string",
                    "description": "HTTP method (default: GET)",
                    "default": "GET",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"]
                },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers as key-value pairs",
                    "default": {}
                },
                "body": {
                    "type": "string",
                    "description": "Optional request body (for POST/PUT/PATCH)"
                }
            },
            "required": ["url"]
        }),
    )
}

/// The x402 HTTP payment tool declaration.
#[must_use]
pub fn x402_tool_declaration() -> ToolDeclaration {
    x402_tool_declaration_ref().clone()
}

/// Returns the cached x402 declaration by reference.
#[must_use]
pub fn x402_tool_declaration_ref() -> &'static ToolDeclaration {
    static DECLARATION: OnceLock<ToolDeclaration> = OnceLock::new();
    DECLARATION.get_or_init(build_x402_tool_declaration)
}

/// The x402 request declaration, under its stable tool name.
#[must_use]
pub fn x402_request_tool_declaration() -> &'static ToolDeclaration {
    x402_tool_declaration_ref()
}

/// All shipped Wallet, Web3 and x402 declarations in the established order.
#[must_use]
pub fn shipped_tool_declarations() -> [ToolDeclaration; 14] {
    let wallet = wallet_declarations();
    let web3 = web3_declarations();
    [
        wallet[0].clone(),
        wallet[1].clone(),
        wallet[2].clone(),
        wallet[3].clone(),
        wallet[4].clone(),
        wallet[5].clone(),
        web3[0].clone(),
        web3[1].clone(),
        web3[2].clone(),
        web3[3].clone(),
        web3[4].clone(),
        web3[5].clone(),
        web3[6].clone(),
        x402_tool_declaration_ref().clone(),
    ]
}

/// Finds any shipped declaration by its stable tool name.
#[must_use]
pub fn find_tool_declaration(name: &str) -> Option<&'static ToolDeclaration> {
    find_wallet_tool_declaration(name)
        .or_else(|| find_web3_tool_declaration(name))
        .or_else(|| (x402_tool_declaration_ref().name == name).then(x402_tool_declaration_ref))
}
