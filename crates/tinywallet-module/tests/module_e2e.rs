//! End-to-end test for loading the built `TinyWallet` module into `TinyBus`.
//!
//! The only test that exercises the real thing: the built `cdylib`, the ABI
//! descriptor, manifest admission, the dynamic loader, and a broker routing
//! actual frames. Everything else in this crate calls Rust functions directly
//! and would keep passing if the artifact stopped loading altogether.
//!
//! What it proves that a unit test cannot: a transaction signed *through the
//! loaded module* is byte-for-byte the transaction the library produces signing
//! in one step. That is the whole claim of moving signing out of a host — if it
//! were false, the host would broadcast something other than what it built.
//!
//! It also demonstrates the intended custody boundary: key material crosses
//! only on `TinyBus` confidential members into an attested module. Ordinary
//! account, quote, approval, and transport callbacks never carry secrets.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use std::{fs, path::PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::{ModuleHost, ModuleState};
use tinybus::transport::memory::MemoryBus;
use tinywallet::wire::{
    AttachRequest, PublicKey, Scheme, Signature, SignedTransaction, SigningRequest,
    TransactionSpec, UnsignedTransaction,
};
use tinywallet::{Chain, tx};
use tinywallet_bus::wire::{
    AuthorizationResult, AuthorizeX402PaymentRequest, ChainTransportRequest, ChainTransportResult,
    HostResponse, SecretMaterial, WalletAccountsResponse, WalletBackendRequest,
    WalletBackendResponse, X402ConfigResponse, X402PreparePaymentRequest, X402RecordPaymentRequest,
};
use tinywallet_module::{BUS_NAME, HOST_INTERFACE, HOST_OBJECT_PATH, OBJECT_PATH};

/// Every method the manifest must declare, in order.
///
/// The manifest's own list is hand-written in `module_export!`, so this is a
/// second hand-written list checked against it. That catches a manifest edited
/// without the interface, which is the failure that actually happens — a method
/// absent from the manifest is not advertised to a host at all.
const EXPECTED_METHODS: &[&str] = &[
    "BuildUnsigned",
    "AttachSignature",
    "DeriveAccount",
    "SignTransaction",
    "ExportKey",
    "SignMessage",
    "ValidateAddress",
    "ConstructEvmTransaction",
    "Web3Routes",
    "Web3SwapQuote",
    "Web3BridgeQuote",
    "Web3DappQuote",
    "Web3Execute",
    "X402PreparePayment",
    "X402RecordPayment",
    "X402SpendingSummary",
];

/// The BIP-39 test vector mnemonic. Never use it for real funds.
const VECTOR: &str = "abandon abandon abandon abandon abandon abandon \
                      abandon abandon abandon abandon abandon about";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires TINYWALLET_TEST_MODULE to point at the built cdylib"]
#[expect(
    clippy::too_many_lines,
    reason = "one artifact load is required because TinyBus does not unload modules"
)]
async fn the_built_module_signs_every_chain_over_a_real_broker() {
    // One test rather than four: TinyBus never unloads a module, and a second
    // load of the same artifact would collide on the well-known name, so every
    // chain is exercised against the one admitted instance.
    let (
        client,
        _host,
        host_bad_response,
        approved_web3,
        submitted_tx,
        approved_x402,
        x402_approved,
        modules,
        broker_task,
        _staged,
    ) = admit_module().await;
    wait_until_serving(&client).await;

    let proxy = client.proxy(BUS_NAME, OBJECT_PATH, BUS_NAME).unwrap();

    let routes: serde_json::Value = proxy.call("Web3Routes", ((),)).await.unwrap();
    assert_eq!(routes, serde_json::json!({"routes": ["evm", "solana"]}));
    let quote: serde_json::Value = proxy
        .call(
            "Web3SwapQuote",
            (tinywallet_bus::wire::Web3QuoteRequest {
                owner: Some(tinywallet_bus::wire::QuoteOwner {
                    thread_id: "thread-1".to_string(),
                    client_id: "client-1".to_string(),
                }),
                params: serde_json::json!({
                    "chainId": 1,
                    "tokenIn": "0x0000000000000000000000000000000000000000",
                    "tokenInAmount": "100",
                    "tokenOut": "0x0000000000000000000000000000000000000001"
                }),
            },),
        )
        .await
        .unwrap();
    assert_eq!(quote["kind"], "swap");
    assert!(quote["quoteId"].as_str().unwrap().starts_with("w3_"));
    assert_eq!(
        quote["quote"]["request"]["senderAddress"],
        "0x9858EfFD232B4033E47d90003D41EC34EcaEda94"
    );
    let execution: serde_json::Value = proxy
        .call_confidential(
            "Web3Execute",
            (tinywallet_bus::wire::Web3ExecuteRequest {
                owner: Some(tinywallet_bus::wire::QuoteOwner {
                    thread_id: "thread-1".to_string(),
                    client_id: "client-1".to_string(),
                }),
                quote_id: quote["quoteId"].as_str().unwrap().to_string(),
                confirmed: true,
                secret: SecretMaterial {
                    mnemonic: VECTOR.to_string(),
                    derivation_path: "m/44'/60'/0'/0/0".to_string(),
                    chain: Chain::Evm,
                },
            },),
        )
        .await
        .unwrap();
    assert_eq!(execution["transactionHash"], "0xfeedface");
    let approved = approved_web3.lock().unwrap().clone().unwrap();
    assert_eq!(approved.quote_id, quote["quoteId"].as_str().unwrap());
    assert_eq!(approved.operation, "swap");
    match approved.transaction {
        tinywallet_bus::wire::Web3ExecutionIntent::Evm {
            network,
            to,
            data,
            value,
            nonce,
            gas_limit,
            gas_price_wei,
            chain_id,
        } => {
            assert_eq!(network, "eip155:1");
            assert_eq!(to, "0x0000000000000000000000000000000000000002");
            assert_eq!(data.as_deref(), Some("0x1234"));
            assert_eq!(value, "0");
            assert_eq!(nonce, 0);
            assert_eq!(gas_limit, 21_000);
            assert_eq!(gas_price_wei, "1");
            assert_eq!(chain_id, 1);
        }
        intent @ tinywallet_bus::wire::Web3ExecutionIntent::Solana { .. } => {
            panic!("expected exact EVM signing facts, got {intent:?}")
        }
    }
    let submitted = submitted_tx.lock().unwrap().clone().unwrap();
    assert!(submitted.starts_with("0x"));
    assert!(
        submitted.len() > 100,
        "module should submit a signed EVM transaction"
    );
    let challenge = BASE64.encode(
        serde_json::to_vec(&serde_json::json!({
            "x402Version": 2,
            "resource": {"url": "https://fixture.test/resource"},
            "accepts": [{
                "scheme": "exact",
                "network": "eip155:1",
                "amount": "100",
                "asset": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                "payTo": "0x0000000000000000000000000000000000000002",
                "maxTimeoutSeconds": 60,
                "extra": {"name": "USD Coin", "version": "2"}
            }],
            "extensions": {}
        }))
        .unwrap(),
    );
    x402_approved.store(false, Ordering::SeqCst);
    let denied = proxy
        .call_confidential::<serde_json::Value>(
            "X402PreparePayment",
            (X402PreparePaymentRequest {
                url: "https://fixture.test/resource".to_string(),
                payment_required: challenge.clone(),
                owner: None,
                secret: SecretMaterial {
                    mnemonic: VECTOR.to_string(),
                    derivation_path: "m/44'/60'/0'/0/0".to_string(),
                    chain: Chain::Evm,
                },
            },),
        )
        .await
        .unwrap_err();
    assert!(denied.to_string().contains("X402OperationFailed"));
    let denied_summary: serde_json::Value = proxy.call("X402SpendingSummary", ((),)).await.unwrap();
    assert_eq!(denied_summary["reserved_atomic"], 0);
    x402_approved.store(true, Ordering::SeqCst);
    let mut expensive_challenge: serde_json::Value =
        serde_json::from_slice(&BASE64.decode(&challenge).unwrap()).unwrap();
    expensive_challenge["accepts"][0]["amount"] = serde_json::json!("1000001");
    let expensive_challenge = BASE64.encode(serde_json::to_vec(&expensive_challenge).unwrap());
    let too_expensive = proxy
        .call_confidential::<serde_json::Value>(
            "X402PreparePayment",
            (X402PreparePaymentRequest {
                url: "https://fixture.test/resource".to_string(),
                payment_required: expensive_challenge,
                owner: None,
                secret: SecretMaterial {
                    mnemonic: VECTOR.to_string(),
                    derivation_path: "m/44'/60'/0'/0/0".to_string(),
                    chain: Chain::Evm,
                },
            },),
        )
        .await
        .unwrap_err();
    assert!(too_expensive.to_string().contains("X402OperationFailed"));
    let budget_summary: serde_json::Value = proxy.call("X402SpendingSummary", ((),)).await.unwrap();
    assert_eq!(budget_summary["reserved_atomic"], 0);
    assert_eq!(
        approved_x402
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .amount_atomic,
        100,
        "an over-budget payment must be rejected before host authorization"
    );
    let payment: serde_json::Value = proxy
        .call_confidential(
            "X402PreparePayment",
            (X402PreparePaymentRequest {
                url: "https://fixture.test/resource".to_string(),
                payment_required: challenge,
                owner: None,
                secret: SecretMaterial {
                    mnemonic: VECTOR.to_string(),
                    derivation_path: "m/44'/60'/0'/0/0".to_string(),
                    chain: Chain::Evm,
                },
            },),
        )
        .await
        .unwrap();
    assert_ne!(payment["payment_header"].as_str().unwrap(), "");
    let approved_payment = approved_x402.lock().unwrap().clone().unwrap();
    assert_eq!(approved_payment.url, "https://fixture.test/resource");
    assert_eq!(approved_payment.amount_atomic, 100);
    assert_eq!(approved_payment.network, "eip155:1");
    assert_eq!(
        approved_payment.asset,
        "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
    );
    assert_eq!(
        approved_payment.recipient,
        "0x0000000000000000000000000000000000000002"
    );
    proxy
        .call::<()>(
            "X402RecordPayment",
            (X402RecordPaymentRequest {
                payment_id: payment["payment_id"].as_str().unwrap().to_string(),
                response_status: 200,
                settlement_response: None,
            },),
        )
        .await
        .unwrap();
    let spending: serde_json::Value = proxy.call("X402SpendingSummary", ((),)).await.unwrap();
    assert_eq!(spending["session_total_atomic"], 100);
    assert_eq!(spending["reserved_atomic"], 0);
    host_bad_response.store(true, Ordering::SeqCst);
    let error = proxy
        .call::<serde_json::Value>("Web3Routes", ((),))
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            tinybus::Error::MethodFailed { name, .. }
                if name == "ai.tinyhumans.tinywallet.Error.HostCallFailed"
        ),
        "incompatible callback replies must fail closed without a local fallback"
    );
    host_bad_response.store(false, Ordering::SeqCst);

    validates_addresses_inside_the_artifact(&proxy).await;
    constructs_evm_transactions_inside_the_artifact(&proxy).await;
    signs_an_evm_transfer_identically_to_the_library(&proxy).await;
    signs_a_multi_input_bitcoin_spend(&proxy).await;
    signs_a_solana_transfer(&proxy).await;
    refuses_a_request_the_module_cannot_build(&proxy).await;
    refuses_confidential_secret_delivery_to_an_unattested_artifact().await;

    assert!(matches!(modules.list()[0].state, ModuleState::Ready));
    broker_task.abort();
}

/// Load the built artifact and check its manifest against the interface.
async fn admit_module() -> (
    Connection,
    Connection,
    Arc<AtomicBool>,
    Arc<Mutex<Option<tinywallet_bus::wire::AuthorizeWeb3ExecutionRequest>>>,
    Arc<Mutex<Option<String>>>,
    Arc<Mutex<Option<AuthorizeX402PaymentRequest>>>,
    Arc<AtomicBool>,
    ModuleHost,
    tokio::task::JoinHandle<tinybus::Result<()>>,
    tempfile::TempDir,
) {
    let artifact =
        std::env::var_os("TINYWALLET_TEST_MODULE").expect("TINYWALLET_TEST_MODULE must be set");
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let modules = ModuleHost::new(broker);

    let artifact = PathBuf::from(artifact);
    let staged = tempfile::Builder::new()
        .prefix("tinywallet-attested-")
        .tempdir_in(artifact.parent().unwrap())
        .unwrap();
    let staged_artifact = staged.path().join(artifact.file_name().unwrap());
    fs::copy(&artifact, &staged_artifact).unwrap();
    let digest = hex(&Sha256::digest(fs::read(&staged_artifact).unwrap()));
    fs::write(
        staged.path().join("modules.toml"),
        format!("{:?} = {:?}\n", artifact.file_name().unwrap(), digest),
    )
    .unwrap();

    let host = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let host_bad_response = Arc::new(AtomicBool::new(false));
    let approved_web3 = Arc::new(Mutex::new(None));
    let submitted_tx = Arc::new(Mutex::new(None));
    let approved_x402 = Arc::new(Mutex::new(None));
    let x402_approved = Arc::new(AtomicBool::new(true));
    host.serve_at(
        HOST_OBJECT_PATH.try_into().unwrap(),
        TestHost {
            bad_response: host_bad_response.clone(),
            approved_web3: approved_web3.clone(),
            submitted_tx: submitted_tx.clone(),
            approved_x402: approved_x402.clone(),
            x402_approved: x402_approved.clone(),
            x402_config_path: staged.path().join("workspace").display().to_string(),
        },
    )
    .await
    .unwrap();
    host.request_name(HOST_INTERFACE).await.unwrap();

    let loaded = modules
        .load_file(&staged_artifact)
        .expect("module should load");
    assert_eq!(loaded.name, "tinywallet-module");
    assert_eq!(loaded.manifest.bus_name.as_str(), BUS_NAME);
    assert_eq!(loaded.manifest.object_path.as_str(), OBJECT_PATH);
    assert_eq!(loaded.manifest.requires.len(), 1);
    assert_eq!(
        loaded.manifest.requires[0].interface.interface.as_str(),
        HOST_INTERFACE,
        "the compiled manifest must require the typed host callback contract"
    );
    assert!(loaded.manifest.requires[0].optional);
    assert_eq!(tinywallet_bus::HOST_METHODS.len(), 6);

    let declared: Vec<&str> = loaded
        .manifest
        .provides
        .iter()
        .flat_map(|interface| interface.methods.iter())
        .map(tinybus::MemberName::as_str)
        .collect();
    assert_eq!(
        declared, EXPECTED_METHODS,
        "manifest methods drifted from the interface"
    );

    // And that the manifest is the contract a host compiles against. The three
    // lists — the `#[tinybus::interface]` block, the `module_export!` manifest,
    // and `tinywallet_bus::METHODS` — are independent, and only a host reading
    // the third would notice the first two agreeing on a name it does not know.
    let mut sorted = declared.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        tinywallet_bus::METHODS,
        "the manifest and the published contract name different members"
    );

    let client = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    (
        client,
        host,
        host_bad_response,
        approved_web3,
        submitted_tx,
        approved_x402,
        x402_approved,
        modules,
        broker_task,
        staged,
    )
}

struct TestHost {
    bad_response: Arc<AtomicBool>,
    approved_web3: Arc<Mutex<Option<tinywallet_bus::wire::AuthorizeWeb3ExecutionRequest>>>,
    submitted_tx: Arc<Mutex<Option<String>>>,
    approved_x402: Arc<Mutex<Option<AuthorizeX402PaymentRequest>>>,
    x402_approved: Arc<AtomicBool>,
    x402_config_path: String,
}

#[expect(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "the generated TinyBus host interface requires async methods"
)]
#[tinybus::interface(name = "ai.tinyhumans.tinywallet.Host")]
impl TestHost {
    async fn authorize_x402_payment(
        &self,
        request: AuthorizeX402PaymentRequest,
    ) -> tinybus::Result<HostResponse> {
        *self.approved_x402.lock().unwrap() = Some(request);
        Ok(HostResponse::Authorization(AuthorizationResult {
            approved: self.x402_approved.load(Ordering::SeqCst),
            reason: Some("fixture approval policy".to_string()),
        }))
    }

    async fn x402_config(&self) -> tinybus::Result<HostResponse> {
        Ok(HostResponse::X402Config(X402ConfigResponse {
            workspace_dir: self.x402_config_path.clone(),
            session_id: "module-e2e".to_string(),
            per_request_max_atomic: 1_000_000,
            daily_max_atomic: 10_000_000,
            monthly_max_atomic: 100_000_000,
        }))
    }

    async fn wallet_accounts(&self) -> tinybus::Result<HostResponse> {
        Ok(HostResponse::WalletAccounts(WalletAccountsResponse {
            configured: true,
            accounts: vec![tinywallet_bus::wire::WalletAccountInfo {
                chain: Chain::Evm,
                address: "0x9858EfFD232B4033E47d90003D41EC34EcaEda94".to_string(),
                derivation_path: "m/44'/60'/0'/0/0".to_string(),
            }],
        }))
    }

    async fn authorize_web3_execution(
        &self,
        request: tinywallet_bus::wire::AuthorizeWeb3ExecutionRequest,
    ) -> tinybus::Result<HostResponse> {
        *self.approved_web3.lock().unwrap() = Some(request);
        Ok(HostResponse::Authorization(AuthorizationResult {
            approved: true,
            reason: None,
        }))
    }

    async fn chain_transport(
        &self,
        request: ChainTransportRequest,
    ) -> tinybus::Result<HostResponse> {
        let result = match request {
            ChainTransportRequest::JsonRpc { method, params, .. } => {
                let value = match method.as_str() {
                    "eth_chainId" | "eth_gasPrice" => serde_json::json!("0x1"),
                    "eth_getTransactionCount" => serde_json::json!("0x0"),
                    "eth_estimateGas" => serde_json::json!("0x5208"),
                    "eth_sendRawTransaction" => {
                        *self.submitted_tx.lock().unwrap() = params[0].as_str().map(str::to_string);
                        serde_json::json!("0xfeedface")
                    }
                    _ => serde_json::Value::Null,
                };
                ChainTransportResult::Ok { body: value }
            }
            _ => ChainTransportResult::Unreachable {
                message: "fixture transport does not implement REST".to_string(),
            },
        };
        Ok(HostResponse::ChainTransport(result))
    }

    async fn wallet_backend(&self, request: WalletBackendRequest) -> tinybus::Result<HostResponse> {
        if self.bad_response.load(Ordering::SeqCst) {
            return Ok(HostResponse::Authorization(AuthorizationResult {
                approved: true,
                reason: None,
            }));
        }
        let data = match request {
            WalletBackendRequest::Routes => serde_json::json!({"routes": ["evm", "solana"]}),
            WalletBackendRequest::SwapTransaction { body }
            | WalletBackendRequest::BridgeTransaction { body } => serde_json::json!({
                "request": body,
                "tx": {
                    "to": "0x0000000000000000000000000000000000000002",
                    "data": "0x1234",
                    "value": "0"
                }
            }),
        };
        Ok(HostResponse::WalletBackend(WalletBackendResponse { data }))
    }
}

/// Wait for the module to claim its well-known name.
async fn wait_until_serving(client: &Connection) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client
                .list_names()
                .await
                .unwrap()
                .iter()
                .any(|name| name.as_str() == BUS_NAME)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("module should become ready");
}

/// The load-bearing case: through the bus must equal in-process.
async fn signs_an_evm_transfer_identically_to_the_library(proxy: &tinybus::Proxy) {
    let secret = derive(Chain::Evm, "m/44'/60'/0'/0/0");
    let spec = TransactionSpec::Evm {
        to: "0x3535353535353535353535353535353535353535".to_string(),
        value_wei: "1000000000000000000".to_string(),
        data_hex: "0x".to_string(),
        nonce: 9,
        gas_limit: 21_000,
        gas_price_wei: "20000000000".to_string(),
        chain_id: 1,
    };

    let signed = round_trip(proxy, &spec, &secret).await;

    let expected = tx::evm::LegacyTransaction {
        nonce: 9,
        gas_price: 20_000_000_000,
        gas_limit: 21_000,
        to: Some("0x3535353535353535353535353535353535353535".to_string()),
        value: 1_000_000_000_000_000_000,
        data: Vec::new(),
        chain_id: 1,
    }
    .sign(&secret)
    .unwrap();

    assert_eq!(
        signed.raw,
        format!("0x{}", hex(&expected)),
        "a transaction signed through the module diverged from the library"
    );
    assert_eq!(
        signed.txid,
        Some(tx::evm::LegacyTransaction::hash_of(&expected))
    );
}

/// Several inputs, so the multi-signature ordering crosses the bus for real.
async fn signs_a_multi_input_bitcoin_spend(proxy: &tinybus::Proxy) {
    let derived = tinywallet::key::derive(Chain::Btc, VECTOR, "m/84'/0'/0'/0/0").unwrap();
    let secret = derived.secret_bytes().to_vec();
    let txid = "7f3b662ea8b6ff2e0e1a1f9bd0f1c39a6b8ba51e1b0f0e0d0c0b0a0908070605";
    let spec = TransactionSpec::Btc {
        from: derived.address().to_string(),
        to: "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_string(),
        amount_sat: 150_000,
        fee_sat: 2_000,
        utxos: (0..3)
            .map(|vout| tinywallet::wire::Utxo {
                txid: txid.to_string(),
                vout,
                value: 60_000 + u64::from(vout) * 10_000,
            })
            .collect(),
    };

    let signed = round_trip(proxy, &spec, &secret).await;

    let expected = tx::btc::Transfer {
        from: derived.address().to_string(),
        to: "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_string(),
        amount: 150_000,
        fee: 2_000,
    }
    .sign(
        &(0..3)
            .map(|vout| tx::btc::Utxo {
                txid: txid.to_string(),
                vout,
                value: 60_000 + u64::from(vout) * 10_000,
            })
            .collect::<Vec<_>>(),
        &secret,
    )
    .unwrap();

    assert_eq!(signed.raw, expected);
}

/// ed25519 rather than a prehashed digest, so the other signing scheme is covered.
async fn signs_a_solana_transfer(proxy: &tinybus::Proxy) {
    use ed25519_dalek::{Signer as _, SigningKey};

    let derived = tinywallet::key::derive(Chain::Solana, VECTOR, "m/44'/501'/0'/0'").unwrap();
    let spec = TransactionSpec::Solana {
        from: derived.address().to_string(),
        to: "11111111111111111111111111111111".to_string(),
        lamports: 1_000_000_000,
        recent_blockhash: "11111111111111111111111111111111".to_string(),
    };
    let public = PublicKey {
        key_hex: hex(derived.secret_bytes()),
    };

    let unsigned: UnsignedTransaction = proxy
        .call(
            "BuildUnsigned",
            (SigningRequest {
                transaction: spec.clone(),
                public_key: public.clone(),
            },),
        )
        .await
        .unwrap();
    assert_eq!(unsigned.payloads[0].scheme, Scheme::Ed25519);

    // Signed here, in this process. The module is never given the key.
    let key: [u8; 32] = derived.secret_bytes().try_into().unwrap();
    let signature = SigningKey::from_bytes(&key)
        .sign(&unhex(&unsigned.payloads[0].bytes_hex))
        .to_bytes();

    let signed: SignedTransaction = proxy
        .call(
            "AttachSignature",
            (AttachRequest {
                transaction: spec.clone(),
                public_key: public,
                signatures: vec![Signature::Ed25519 {
                    signature_hex: hex(&signature),
                }],
            },),
        )
        .await
        .unwrap();

    let expected = tx::solana::NativeTransfer {
        from: derived.address().to_string(),
        to: "11111111111111111111111111111111".to_string(),
        lamports: 1_000_000_000,
        recent_blockhash: "11111111111111111111111111111111".to_string(),
    }
    .sign(derived.secret_bytes())
    .unwrap();

    assert_eq!(signed.raw, base64(&expected));
}

/// A malformed request must come back as a named error, not a signature.
///
/// This used to send a request whose `chain` tag contradicted its transaction.
/// That is no longer expressible — the requests carry no `chain` and the spec
/// names its own — so the case is now a transaction the module can parse but
/// cannot build: a Tron payload whose recomputed `txID` disagrees with the one
/// claimed, which is the defence against a compromised node.
async fn refuses_a_request_the_module_cannot_build(proxy: &tinybus::Proxy) {
    let result: tinybus::Result<UnsignedTransaction> = proxy
        .call(
            "BuildUnsigned",
            (SigningRequest {
                transaction: TransactionSpec::Tron {
                    raw_data_hex: "0a02b1f12208".to_string() + &"ab".repeat(64),
                    expected_to: "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t".to_string(),
                    expected_txid: "00".repeat(32),
                    transfer: tinywallet::wire::TronTransfer::Native { amount_sun: 0 },
                },
                public_key: PublicKey {
                    key_hex: "02".repeat(33),
                },
            },),
        )
        .await;

    let error = result.expect_err("a tampered transaction must not produce a signature");
    assert_eq!(
        error.wire_name(),
        "ai.tinyhumans.tinywallet.Error.InvalidInput",
        "the wire error name is the contract a host matches on"
    );
}

/// Drive both calls for a secp256k1 chain, signing locally in between.
async fn round_trip(
    proxy: &tinybus::Proxy,
    spec: &TransactionSpec,
    secret: &[u8],
) -> SignedTransaction {
    let public_key = PublicKey {
        key_hex: hex(&compressed_public(secret)),
    };

    let unsigned: UnsignedTransaction = proxy
        .call(
            "BuildUnsigned",
            (SigningRequest {
                transaction: spec.clone(),
                public_key: public_key.clone(),
            },),
        )
        .await
        .unwrap();

    // Every payload signed here, with a key the module has never seen.
    let signatures = unsigned
        .payloads
        .iter()
        .map(|payload| {
            assert_eq!(payload.scheme, Scheme::Secp256k1Prehash);
            let digest: [u8; 32] = unhex(&payload.bytes_hex).try_into().unwrap();
            let secret = SecretKey::from_slice(secret).unwrap();
            let recoverable = Secp256k1::signing_only()
                .sign_ecdsa_recoverable(&Message::from_digest(digest), &secret);
            let (recovery_id, compact) = recoverable.serialize_compact();
            Signature::Secp256k1 {
                rs_hex: hex(&compact),
                recovery_id: u8::try_from(recovery_id.to_i32()).unwrap(),
            }
        })
        .collect();

    proxy
        .call(
            "AttachSignature",
            (AttachRequest {
                transaction: spec.clone(),
                public_key,
                signatures,
            },),
        )
        .await
        .unwrap()
}

fn derive(chain: Chain, path: &str) -> Vec<u8> {
    tinywallet::key::derive(chain, VECTOR, path)
        .unwrap()
        .secret_bytes()
        .to_vec()
}

fn compressed_public(secret: &[u8]) -> [u8; 33] {
    use bitcoin::secp256k1::PublicKey as SecpPublic;
    let secret = SecretKey::from_slice(secret).unwrap();
    SecpPublic::from_secret_key(&Secp256k1::new(), &secret).serialize()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn unhex(value: &str) -> Vec<u8> {
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
        .collect()
}

/// Standard base64, matching what the module emits for Solana.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[((triple >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((triple >> 12) & 0x3f) as usize]));
        out.push(if chunk.len() > 1 {
            char::from(TABLE[((triple >> 6) & 0x3f) as usize])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(TABLE[(triple & 0x3f) as usize])
        } else {
            '='
        });
    }
    out
}

/// The guard, proven against the real loader.
///
/// This module is loaded from a bare path with no `modules.toml` beside it, so
/// nobody vouched for its bytes and the broker must refuse to carry a secret to
/// it — even though the artifact is genuine, the name resolves, and the very
/// same method would succeed if it were attested.
///
/// It is the negative half that makes the confidential flow worth anything. A
/// positive result alone would be equally consistent with a broker that carried
/// every confidential message to anyone who asked.
async fn refuses_confidential_secret_delivery_to_an_unattested_artifact() {
    let artifact =
        std::env::var_os("TINYWALLET_TEST_MODULE").expect("TINYWALLET_TEST_MODULE must be set");
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let modules = ModuleHost::new(broker);
    modules
        .load_file(artifact)
        .expect("unattested module should load");
    let client = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    wait_until_serving(&client).await;
    let proxy = client.proxy(BUS_NAME, OBJECT_PATH, BUS_NAME).unwrap();
    let request = serde_json::json!({
        "secret": {
            "mnemonic": VECTOR,
            "derivation_path": "m/44'/60'/0'/0/0",
            "chain": "evm",
        },
        "transaction": {
            "kind": "evm",
            "to": "0x3535353535353535353535353535353535353535",
            "value_wei": "1000000000000000000",
            "data_hex": "0x",
            "nonce": 9,
            "gas_limit": 21_000,
            "gas_price_wei": "20000000000",
            "chain_id": 1,
        },
    });

    let error = proxy
        .call_confidential::<SignedTransaction>("SignTransaction", (request.clone(),))
        .await
        .expect_err("an unattested module must not receive a recovery phrase");

    // The specific error matters: "not attested" and "no such name" send an
    // operator to fix different things, and the module *is* installed here.
    let rendered = error.to_string();
    assert!(
        rendered.contains("NotAttested") || rendered.to_lowercase().contains("attest"),
        "expected an attestation refusal, got: {rendered}"
    );
    let error = proxy
        .call::<SignedTransaction>("SignTransaction", (request,))
        .await
        .expect_err("secret-bearing calls must reject ordinary delivery");
    assert!(
        error.to_string().to_lowercase().contains("confidential"),
        "expected confidential-call refusal, got: {error}"
    );
    broker_task.abort();
}

async fn validates_addresses_inside_the_artifact(proxy: &tinybus::Proxy) {
    use tinywallet_bus::wire::{ValidateAddressRequest, ValidateAddressResponse};
    for (chain, address) in [
        (Chain::Evm, "0x3535353535353535353535353535353535353535"),
        (Chain::Solana, "11111111111111111111111111111111"),
        (Chain::Tron, "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"),
    ] {
        let response: ValidateAddressResponse = proxy
            .call(
                "ValidateAddress",
                (ValidateAddressRequest {
                    chain,
                    address: address.into(),
                    sender: false,
                },),
            )
            .await
            .unwrap();
        assert_eq!(response.result.unwrap(), address);
    }
    let response: ValidateAddressResponse = proxy
        .call(
            "ValidateAddress",
            (ValidateAddressRequest {
                chain: Chain::Evm,
                address: "invalid".into(),
                sender: false,
            },),
        )
        .await
        .unwrap();
    assert!(matches!(
        response.result,
        Err(tinywallet_bus::Error::InvalidAddress { .. })
    ));
    let response: ValidateAddressResponse = proxy
        .call(
            "ValidateAddress",
            (ValidateAddressRequest {
                chain: Chain::Btc,
                address: "1BoatSLRHtKNngkdXEeobR76b53LETtpyT".into(),
                sender: true,
            },),
        )
        .await
        .unwrap();
    assert!(matches!(
        response.result,
        Err(tinywallet_bus::Error::UnsupportedAddressType {
            chain: Chain::Btc,
            ..
        })
    ));
}

/// Verify native/token/call construction and signature-ready bytes through the loaded ABI.
async fn constructs_evm_transactions_inside_the_artifact(proxy: &tinybus::Proxy) {
    use tinywallet_bus::wire::{ConstructedEvmTransaction, EvmConstructionRequest, EvmIntent};
    let secret = derive(Chain::Evm, "m/44'/60'/0'/0/0");
    let mut request = EvmConstructionRequest {
        public_key: PublicKey {
            key_hex: hex(&compressed_public(&secret)),
        },
        chain_id: 1,
        nonce: 7,
        gas_limit: 21000,
        gas_price_wei: "30000000000".into(),
        intent: EvmIntent::NativeTransfer {
            to: "0x3535353535353535353535353535353535353535".into(),
            amount_wei: "5".into(),
        },
    };
    for intent in [
        request.intent.clone(),
        EvmIntent::Erc20Transfer {
            token: "0x1111111111111111111111111111111111111111".into(),
            to: "0x3535353535353535353535353535353535353535".into(),
            amount_raw: "340282366920938463463374607431768211456".into(),
        },
        EvmIntent::ContractCall {
            to: "0x1111111111111111111111111111111111111111".into(),
            value_wei: "0".into(),
            data_hex: "0xAaBb".into(),
        },
    ] {
        request.intent = intent;
        let response: ConstructedEvmTransaction = proxy
            .call(
                tinywallet_bus::names::methods::CONSTRUCT_EVM_TRANSACTION,
                (&request,),
            )
            .await
            .unwrap();
        let unsigned: UnsignedTransaction = proxy
            .call(
                "BuildUnsigned",
                (SigningRequest {
                    transaction: response.transaction.clone(),
                    public_key: request.public_key.clone(),
                },),
            )
            .await
            .unwrap();
        assert_eq!(response.unsigned, unsigned);
        assert_eq!(
            response.approval.sender,
            "0x9858EfFD232B4033E47d90003D41EC34EcaEda94"
        );
        assert_eq!(response.approval.max_fee_wei, "630000000000000");
        let signed = round_trip(proxy, &response.transaction, &secret).await;
        assert!(signed.raw.starts_with("0x"));
    }
    request.public_key.key_hex = "00".repeat(33);
    let refusal: tinybus::Result<ConstructedEvmTransaction> =
        proxy.call("ConstructEvmTransaction", (request,)).await;
    assert!(
        matches!(refusal,Err(tinybus::Error::MethodFailed{name,..}) if name=="ai.tinyhumans.tinywallet.Error.InvalidInput")
    );
}
