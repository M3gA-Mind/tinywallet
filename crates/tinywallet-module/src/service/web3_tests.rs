#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use tinywallet_crypto::Chain;
use tinywallet_crypto::rpc::NetworkId;

fn network(chain: Chain, evm_chain_id: Option<u64>) -> NetworkId {
    NetworkId {
        chain,
        evm_chain_id,
    }
}

#[test]
fn network_names_round_trip_for_every_supported_chain() {
    for (crypto, bus) in [
        (Chain::Evm, tinywallet_bus::Chain::Evm),
        (Chain::Btc, tinywallet_bus::Chain::Btc),
        (Chain::Solana, tinywallet_bus::Chain::Solana),
        (Chain::Tron, tinywallet_bus::Chain::Tron),
    ] {
        let id = network(crypto, Some(137));
        let request = network_request(id).unwrap();
        assert_eq!(request.chain, bus);
        assert_eq!(request.evm_chain_id, Some(137));
        assert_eq!(network_id(request).unwrap(), id);
    }
}

#[test]
fn rest_results_preserve_success_and_distinguish_transport_errors() {
    let network = network(Chain::Solana, None);
    assert_eq!(
        rest_result(
            network,
            ChainTransportResult::Ok {
                body: Value::String("response".to_string()),
            },
        )
        .unwrap(),
        "response"
    );
    let non_string = rest_result(
        network,
        ChainTransportResult::Ok {
            body: serde_json::json!({"unexpected": true}),
        },
    )
    .unwrap_err();
    assert!(matches!(non_string, TransportError::Rpc { .. }));
    let unreachable = rest_result(
        network,
        ChainTransportResult::Unreachable {
            message: "offline".to_string(),
        },
    )
    .unwrap_err();
    assert!(matches!(unreachable, TransportError::Unreachable { .. }));
    let rejected = rest_result(
        network,
        ChainTransportResult::RpcError {
            message: "invalid request".to_string(),
        },
    )
    .unwrap_err();
    assert!(matches!(rejected, TransportError::Rpc { .. }));
}

#[test]
fn default_endpoints_use_mainnet_and_report_default_source() {
    let endpoints = DefaultEndpoints;
    assert_ne!(endpoints.url(WalletChain::Evm, None), "");
    assert_ne!(
        endpoints.url(WalletChain::Evm, Some(EvmNetwork::PolygonMainnet)),
        ""
    );
    assert_ne!(endpoints.url(WalletChain::Solana, None), "");
    assert_eq!(endpoints.source(WalletChain::Evm, None), RpcSource::Default);
    assert_eq!(endpoints.solana_cluster(), SolanaCluster::Mainnet);
}

#[tokio::test]
async fn task_scopes_bind_quote_owner_and_consume_secret_once() {
    let owner = tinywallet_bus::wire::QuoteOwner {
        thread_id: "thread".to_string(),
        client_id: "client".to_string(),
    };
    assert_eq!(TaskQuoteScope.current_owner(), None);
    assert_eq!(
        with_quote_owner(Some(owner.clone()), async {
            TaskQuoteScope.current_owner()
        })
        .await,
        Some(LibraryQuoteOwner {
            thread_id: owner.thread_id,
            client_id: owner.client_id,
        })
    );

    let secret = SecretMaterial {
        mnemonic: "temporary phrase".to_string(),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        chain: tinywallet_bus::Chain::Evm,
    };
    assert!(take_secret().is_err());
    let (first, second) =
        with_execution_context(None, secret, None, async { (take_secret(), take_secret()) }).await;
    assert_eq!(first.unwrap().mnemonic, "temporary phrase");
    assert!(second.is_err());
}

#[derive(Clone)]
struct TestHost {
    authorize: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinywallet.Host")]
impl TestHost {
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "TinyBus interface methods must be async even when this fixture returns immediately"
    )]
    async fn wallet_backend(
        &self,
        request: WalletBackendRequest,
    ) -> tinybus::Result<tinywallet_bus::wire::HostResponse> {
        use tinywallet_bus::wire::{HostResponse, WalletBackendResponse};
        let data = match request {
            WalletBackendRequest::Routes => serde_json::json!({"routes":["evm", "solana"]}),
            WalletBackendRequest::BridgeTransaction { body }
                if body.get("srcChainId") == Some(&serde_json::json!(7_565_164)) =>
            {
                serde_json::json!({
                    "request": body,
                    "tx": {"data":"deadbeef"}
                })
            }
            WalletBackendRequest::SwapTransaction { body }
            | WalletBackendRequest::BridgeTransaction { body } => serde_json::json!({
                "request": body,
                "tx": {"to":"0x0000000000000000000000000000000000000002", "data":"0x1234", "value":"0"}
            }),
        };
        Ok(HostResponse::WalletBackend(WalletBackendResponse { data }))
    }

    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "TinyBus interface methods must be async even when this fixture returns immediately"
    )]
    async fn wallet_accounts(&self) -> tinybus::Result<tinywallet_bus::wire::HostResponse> {
        use tinywallet_bus::wire::{HostResponse, WalletAccountInfo, WalletAccountsResponse};
        Ok(HostResponse::WalletAccounts(WalletAccountsResponse {
            configured: true,
            accounts: [
                (
                    tinywallet_bus::Chain::Evm,
                    "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
                    "m/44'/60'/0'/0/0",
                ),
                (tinywallet_bus::Chain::Btc, "bc1qfixture", "m/84'/0'/0'/0/0"),
                (
                    tinywallet_bus::Chain::Solana,
                    "solana-fixture",
                    "m/44'/501'/0'/0'",
                ),
                (
                    tinywallet_bus::Chain::Tron,
                    "tron-fixture",
                    "m/44'/195'/0'/0/0",
                ),
            ]
            .into_iter()
            .map(|(chain, address, derivation_path)| WalletAccountInfo {
                chain,
                address: address.to_string(),
                derivation_path: derivation_path.to_string(),
            })
            .collect(),
        }))
    }

    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "TinyBus interface methods must be async even when this fixture returns immediately"
    )]
    async fn chain_transport(
        &self,
        request: ChainTransportRequest,
    ) -> tinybus::Result<tinywallet_bus::wire::HostResponse> {
        use tinywallet_bus::wire::{ChainTransportResult, HostResponse};
        let result = match request {
            ChainTransportRequest::JsonRpc { method, params, .. } => {
                let body = match method.as_str() {
                    "eth_chainId" | "eth_gasPrice" => serde_json::json!("0x1"),
                    "eth_getTransactionCount" => serde_json::json!("0x0"),
                    "eth_estimateGas" => serde_json::json!("0x5208"),
                    "eth_sendRawTransaction" => serde_json::json!("0xfeedface"),
                    _ => serde_json::json!({"params": params}),
                };
                match method.as_str() {
                    "fixture_unreachable" => ChainTransportResult::Unreachable {
                        message: "offline fixture".to_string(),
                    },
                    "fixture_rpc_error" => ChainTransportResult::RpcError {
                        message: "rejected fixture".to_string(),
                    },
                    _ => ChainTransportResult::Ok { body },
                }
            }
            ChainTransportRequest::RestGet { .. } | ChainTransportRequest::RestPost { .. } => {
                ChainTransportResult::Ok {
                    body: serde_json::json!("rest response"),
                }
            }
        };
        Ok(HostResponse::ChainTransport(result))
    }

    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "TinyBus interface methods must be async even when this fixture returns immediately"
    )]
    async fn authorize_web3_execution(
        &self,
        _request: AuthorizeWeb3ExecutionRequest,
    ) -> tinybus::Result<tinywallet_bus::wire::HostResponse> {
        use tinywallet_bus::wire::{AuthorizationResult, HostResponse};
        Ok(HostResponse::Authorization(AuthorizationResult {
            approved: self.authorize.load(std::sync::atomic::Ordering::SeqCst),
            reason: Some("fixture decision".to_string()),
        }))
    }

    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "TinyBus interface methods must be async even when this fixture returns immediately"
    )]
    async fn authorize_x402_payment(
        &self,
        _request: tinywallet_bus::wire::AuthorizeX402PaymentRequest,
    ) -> tinybus::Result<tinywallet_bus::wire::HostResponse> {
        use tinywallet_bus::wire::{AuthorizationResult, HostResponse};
        Ok(HostResponse::Authorization(AuthorizationResult {
            approved: true,
            reason: None,
        }))
    }
}

async fn callbacks() -> (
    HostCallbacks,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    tinybus::Connection,
    tokio::task::JoinHandle<tinybus::Result<()>>,
) {
    use std::sync::atomic::AtomicBool;

    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker = tinybus::broker::Broker::new();
    let task = broker.spawn(bus.clone());
    let host = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let authorize = std::sync::Arc::new(AtomicBool::new(true));
    host.serve_at(
        crate::service::HOST_OBJECT_PATH.try_into().unwrap(),
        TestHost {
            authorize: authorize.clone(),
        },
    )
    .await
    .unwrap();
    host.request_name(crate::service::HOST_INTERFACE)
        .await
        .unwrap();
    let client = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    assert!(
        client
            .list_names()
            .await
            .unwrap()
            .iter()
            .any(|name| name.as_str() == crate::service::HOST_INTERFACE)
    );
    (HostCallbacks::new(&client).unwrap(), authorize, host, task)
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One in-memory fixture proves every adapter and typed approval mapping together"
)]
async fn host_adapters_and_quote_approval_cover_supported_operations() {
    use tinywallet_web3::crypto::defaults::EvmNetwork;
    use tinywallet_web3::crypto::service::{BridgeQuoteParams, DappCallParams, SwapQuoteParams};

    let (callbacks, _, _host, broker_task) = callbacks().await;
    let accounts = HostAccounts(callbacks.clone()).status().await.unwrap();
    assert!(accounts.configured);
    assert_eq!(accounts.accounts.len(), 4);
    let backend = HostWeb3Backend(callbacks.clone());
    assert_eq!(backend.routes().await.unwrap()["routes"][0], "evm");
    assert!(
        backend
            .swap_tx(&serde_json::json!({"amount":"1"}))
            .await
            .is_ok()
    );
    assert!(
        backend
            .bridge_tx(&serde_json::json!({"amount":"1"}))
            .await
            .is_ok()
    );

    let transport = HostTransport(callbacks.clone());
    let id = network(Chain::Evm, Some(1));
    assert_eq!(
        transport
            .json_rpc(id, "eth_chainId", Value::Null)
            .await
            .unwrap(),
        serde_json::json!("0x1")
    );
    assert!(matches!(
        transport
            .json_rpc(id, "fixture_unreachable", Value::Null)
            .await
            .unwrap_err(),
        TransportError::Unreachable { .. }
    ));
    assert!(matches!(
        transport
            .json_rpc(id, "fixture_rpc_error", Value::Null)
            .await
            .unwrap_err(),
        TransportError::Rpc { .. }
    ));
    assert_eq!(
        transport.rest_get(id, "balance").await.unwrap(),
        "rest response"
    );
    assert_eq!(
        transport
            .rest_post(id, "send", "body".into(), "application/json")
            .await
            .unwrap(),
        "rest response"
    );

    let runtime = Web3Runtime::new(callbacks);
    let swap = runtime
        .service
        .quote_swap(SwapQuoteParams {
            chain_id: 1,
            token_in: "0x0000000000000000000000000000000000000000".to_string(),
            token_in_amount: "1".to_string(),
            token_out: "0x0000000000000000000000000000000000000001".to_string(),
            token_out_recipient: None,
            sender_address: None,
            slippage: None,
        })
        .await
        .unwrap();
    assert_eq!(
        runtime
            .execution_approval(&swap.quote_id, None)
            .unwrap()
            .operation,
        "swap"
    );

    let bridge = runtime
        .service
        .quote_bridge(BridgeQuoteParams {
            src_chain_id: 1,
            src_chain_token_in: "0x0".to_string(),
            src_chain_token_in_amount: "1".to_string(),
            dst_chain_id: 10,
            dst_chain_token_out: "0x1".to_string(),
            dst_chain_token_out_amount: None,
            dst_chain_token_out_recipient: None,
            src_chain_order_authority_address: None,
            dst_chain_order_authority_address: None,
        })
        .await
        .unwrap();
    assert_eq!(
        runtime
            .execution_approval(&bridge.quote_id, None)
            .unwrap()
            .operation,
        "bridge"
    );

    let solana_bridge = runtime
        .service
        .quote_bridge(BridgeQuoteParams {
            src_chain_id: tinywallet_web3::crypto::service::DEBRIDGE_SOLANA_CHAIN_ID,
            src_chain_token_in: "So11111111111111111111111111111111111111112".to_string(),
            src_chain_token_in_amount: "1".to_string(),
            dst_chain_id: 1,
            dst_chain_token_out: "0x1".to_string(),
            dst_chain_token_out_amount: None,
            dst_chain_token_out_recipient: None,
            src_chain_order_authority_address: None,
            dst_chain_order_authority_address: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        runtime
            .execution_approval(&solana_bridge.quote_id, None)
            .unwrap()
            .solana_intent,
        Some(Web3ExecutionIntent::Solana { tx_blob_hex }) if tx_blob_hex == "deadbeef"
    ));

    let dapp = runtime
        .service
        .prepare_dapp_call(DappCallParams {
            contract_address: "0x1111111111111111111111111111111111111111".to_string(),
            calldata: "0xabcd".to_string(),
            value_raw: Some("0".to_string()),
            evm_network: Some(EvmNetwork::EthereumMainnet),
        })
        .await
        .unwrap();
    assert_eq!(
        runtime
            .execution_approval(&dapp.quote_id, None)
            .unwrap()
            .operation,
        "dapp_call"
    );
    broker_task.abort();
}

#[tokio::test]
async fn module_signer_checks_approval_and_secret_chain_before_signing() {
    use tinywallet_bus::wire::{Scheme, SecretMaterial, TransactionSpec};
    use tinywallet_web3::crypto::seams::WalletSigner;
    use tinywallet_web3::crypto::wallet::WalletChain;

    let (callbacks, authorize, _host, broker_task) = callbacks().await;
    let signer = ModuleSigner(callbacks);
    let secret = SecretMaterial {
        mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string(),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        chain: tinywallet_bus::Chain::Evm,
    };
    let approval = Web3ApprovalContext {
        quote_id: "quote".to_string(),
        operation: "swap".to_string(),
        solana_intent: None,
    };
    let transaction = TransactionSpec::Evm {
        to: "0x3535353535353535353535353535353535353535".to_string(),
        value_wei: "1".to_string(),
        data_hex: "0x".to_string(),
        nonce: 0,
        gas_limit: 21_000,
        gas_price_wei: "1".to_string(),
        chain_id: 1,
    };
    let (signed_tx, denied) =
        with_execution_context(None, secret.clone(), Some(approval.clone()), async {
            let message_signature = signer
                .sign_transaction(WalletChain::Evm, &transaction)
                .await
                .unwrap();
            authorize.store(false, std::sync::atomic::Ordering::SeqCst);
            let denied = signer
                .sign_transaction(WalletChain::Evm, &transaction)
                .await
                .unwrap_err();
            (message_signature, denied)
        })
        .await;
    assert!(signed_tx.raw.starts_with("0x"));
    assert!(denied.contains("fixture decision"));
    let missing_approval = with_execution_context(None, secret.clone(), None, async {
        signer
            .sign_transaction(WalletChain::Evm, &transaction)
            .await
            .unwrap_err()
    })
    .await;
    assert!(missing_approval.contains("approval context is unavailable"));
    let mismatch = with_execution_context(None, secret.clone(), None, async {
        signer
            .sign_transaction(WalletChain::Solana, &transaction)
            .await
            .unwrap_err()
    })
    .await;
    assert!(mismatch.contains("does not match"));

    let account = with_execution_context(None, secret.clone(), None, async {
        signer.derive_account(WalletChain::Evm).await.unwrap()
    })
    .await;
    assert!(account.address.starts_with("0x"));
    let message = with_execution_context(None, secret, None, async {
        signer
            .sign_message(WalletChain::Evm, &[0x42; 32], Scheme::Secp256k1Prehash)
            .await
            .unwrap()
    })
    .await;
    assert!(matches!(
        message,
        tinywallet_bus::wire::Signature::Secp256k1 { .. }
    ));
    broker_task.abort();
}
