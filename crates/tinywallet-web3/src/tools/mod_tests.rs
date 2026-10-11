#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use tinytools::Tool;

use super::wallet::{
    WalletChainStatusTool, WalletLookupTxTool, WalletPrepareTransferTool, WalletStatusTool,
    WalletTxReceiptTool, WalletTxStatusTool,
};
use super::web3::{
    Web3BridgeExecuteTool, Web3BridgeQuoteTool, Web3DappCallTool, Web3DappExecuteTool,
    Web3SwapExecuteTool, Web3SwapQuoteTool, Web3SwapRoutesTool,
};
use crate::test_support::{Rig, ServiceRig};

#[test]
fn implementation_tools_preserve_the_recorded_pre_move_declarations() {
    let rig = Rig::new();
    let service = ServiceRig::new();
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(WalletStatusTool::new(rig.engine.clone())),
        Box::new(WalletChainStatusTool::new(rig.engine.clone())),
        Box::new(WalletPrepareTransferTool::new(rig.engine.clone())),
        Box::new(WalletTxStatusTool::new(rig.engine.clone())),
        Box::new(WalletTxReceiptTool::new(rig.engine.clone())),
        Box::new(WalletLookupTxTool::new(rig.engine.clone())),
        Box::new(Web3SwapQuoteTool::new(service.service.clone())),
        Box::new(Web3SwapExecuteTool::new(service.service.clone())),
        Box::new(Web3SwapRoutesTool::new(service.service.clone())),
        Box::new(Web3BridgeQuoteTool::new(service.service.clone())),
        Box::new(Web3BridgeExecuteTool::new(service.service.clone())),
        Box::new(Web3DappCallTool::new(service.service.clone())),
        Box::new(Web3DappExecuteTool::new(service.service.clone())),
    ];
    let declarations: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name(),
                "description": tool.description(),
                "parameters": tool.parameters_schema(),
            })
        })
        .collect();
    let recorded: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tinywallet-bus/tests/fixtures/tool-declarations-pre-native-v0.9.0.json"
    ))
    .unwrap();
    assert_eq!(declarations, recorded[..13]);
}
