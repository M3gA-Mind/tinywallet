#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

static LEDGER_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const VECTOR: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

fn secret(chain: tinywallet_bus::Chain) -> SecretMaterial {
    let derivation_path = match chain {
        tinywallet_bus::Chain::Solana => "m/44'/501'/0'/0'",
        _ => "m/44'/60'/0'/0/0",
    };
    SecretMaterial {
        mnemonic: VECTOR.to_string(),
        derivation_path: derivation_path.to_string(),
        chain,
    }
}

#[tokio::test]
async fn payment_signer_derives_and_signs_for_both_supported_rails() {
    let signer = ModulePaymentSigner;
    assert!(signer.account(PaymentChain::Evm).await.is_err());
    assert!(crate::service::web3::clone_secret().is_err());

    let (evm, evm_signature, solana, solana_signature) =
        crate::service::web3::with_execution_context(
            None,
            secret(tinywallet_bus::Chain::Evm),
            None,
            async {
                let evm = signer.account(PaymentChain::Evm).await.unwrap();
                let evm_signature = signer
                    .sign(PaymentChain::Evm, &[0x42; 32], SignScheme::Secp256k1Digest)
                    .await
                    .unwrap();
                let solana = crate::service::web3::with_execution_context(
                    None,
                    secret(tinywallet_bus::Chain::Solana),
                    None,
                    async {
                        let solana = signer.account(PaymentChain::Solana).await.unwrap();
                        let solana_signature = signer
                            .sign(PaymentChain::Solana, b"fixture", SignScheme::Ed25519)
                            .await
                            .unwrap();
                        (solana, solana_signature)
                    },
                )
                .await;
                (evm, evm_signature, solana.0, solana.1)
            },
        )
        .await;

    assert!(evm.address.starts_with("0x"));
    assert_eq!(evm.pubkey, None);
    assert_eq!(evm_signature.len(), 65);
    assert_eq!(solana.address.len(), 44);
    assert_eq!(solana.pubkey.unwrap().len(), 32);
    assert_eq!(solana_signature.len(), 64);
}

#[tokio::test]
async fn payment_signer_rejects_chain_and_scheme_mismatches() {
    let signer = ModulePaymentSigner;
    let (wrong_chain, unsupported_scheme) = crate::service::web3::with_execution_context(
        None,
        secret(tinywallet_bus::Chain::Solana),
        None,
        async {
            let wrong_chain = signer.account(PaymentChain::Evm).await.unwrap_err();
            let unsupported_scheme = signer
                .sign(PaymentChain::Evm, &[0; 32], SignScheme::Ed25519)
                .await
                .unwrap_err();
            (wrong_chain, unsupported_scheme)
        },
    )
    .await;
    assert!(wrong_chain.contains("does not match"));
    assert!(unsupported_scheme.contains("unsupported signing scheme"));
}

#[test]
fn wiping_a_secret_clears_its_phrase() {
    let mut secret = secret(tinywallet_bus::Chain::Evm);
    wipe(&mut secret);
    assert_eq!(secret.mnemonic, "");
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One lifecycle fixture verifies handle validation, reservation cleanup, and record behavior"
)]
async fn ledger_lifecycle_rejects_stale_handles_and_malformed_challenges() {
    let _ledger_guard = LEDGER_TEST_LOCK.lock().await;
    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker = tinybus::broker::Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let connection = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let callbacks = HostCallbacks::new(&connection).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mut runtime = X402Runtime::new(
        callbacks,
        X402ConfigResponse {
            workspace_dir: workspace.path().display().to_string(),
            session_id: "coverage-test".to_string(),
            per_request_max_atomic: 100,
            daily_max_atomic: 100,
            monthly_max_atomic: 100,
        },
    );
    assert!(format!("{runtime:?}").contains("X402Runtime"));
    let handle = runtime.handle();
    let stale = X402ServiceHandle {
        handle: "stale".to_string(),
    };
    let invalid_service = runtime
        .prepare(X402PreparePaymentRequest {
            service: stale,
            url: "https://example.invalid".to_string(),
            payment_required: "bad\nheader".to_string(),
            owner: None,
            secret: secret(tinywallet_bus::Chain::Evm),
        })
        .await
        .unwrap_err();
    assert!(invalid_service.contains("handle is unavailable"));

    let invalid_header = runtime
        .prepare(X402PreparePaymentRequest {
            service: handle.clone(),
            url: "https://example.invalid".to_string(),
            payment_required: "bad\nheader".to_string(),
            owner: None,
            secret: secret(tinywallet_bus::Chain::Evm),
        })
        .await
        .unwrap_err();
    assert!(invalid_header.contains("invalid payment challenge header"));

    let missing_payment = runtime
        .record(X402RecordPaymentRequest {
            service: handle.clone(),
            payment_id: "missing".to_string(),
            response_status: 500,
            settlement_response: None,
        })
        .unwrap_err();
    assert!(missing_payment.contains("missing or expired"));
    runtime.prune_pending();
    let reservation = ledger::reserve(1).unwrap().unwrap();
    runtime.pending.lock().unwrap().insert(
        "failed-payment".to_string(),
        PendingPayment {
            payment: tinywallet_x402::protocol::X402PaymentResult {
                header_value: "proof".to_string(),
                amount_atomic: 1,
                asset: "usdc".to_string(),
                recipient: "recipient".to_string(),
                network: "eip155:1".to_string(),
                url: "https://example.invalid".to_string(),
                reservation,
            },
            created_at: Instant::now(),
            thread_id: None,
        },
    );
    runtime
        .record(X402RecordPaymentRequest {
            service: handle.clone(),
            payment_id: "failed-payment".to_string(),
            response_status: 500,
            settlement_response: None,
        })
        .unwrap();
    let reservation = ledger::reserve(1).unwrap().unwrap();
    runtime.pending.lock().unwrap().insert(
        "expired-payment".to_string(),
        PendingPayment {
            payment: tinywallet_x402::protocol::X402PaymentResult {
                header_value: "proof".to_string(),
                amount_atomic: 1,
                asset: "usdc".to_string(),
                recipient: "recipient".to_string(),
                network: "eip155:1".to_string(),
                url: "https://example.invalid".to_string(),
                reservation,
            },
            created_at: Instant::now()
                .checked_sub(PENDING_TTL + Duration::from_secs(1))
                .unwrap(),
            thread_id: None,
        },
    );
    let expired = runtime
        .record(X402RecordPaymentRequest {
            service: handle.clone(),
            payment_id: "expired-payment".to_string(),
            response_status: 200,
            settlement_response: None,
        })
        .unwrap_err();
    assert!(expired.contains("missing or expired"));
    let summary = runtime.summary(&handle).unwrap();
    assert_eq!(summary.reserved_atomic, 0);
    runtime.close(&handle).unwrap();
    broker_task.abort();
}

#[tokio::test]
async fn pending_payments_are_capped_and_eviction_releases_the_old_reservation() {
    let _ledger_guard = LEDGER_TEST_LOCK.lock().await;
    // Build the runtime with a real in-memory bus connection, but do not need
    // a host callback for this store-capacity behavior.
    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker = tinybus::broker::Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let connection = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let callbacks = HostCallbacks::new(&connection).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mut runtime = X402Runtime::new(
        callbacks,
        X402ConfigResponse {
            workspace_dir: workspace.path().display().to_string(),
            session_id: "capacity-test".to_string(),
            per_request_max_atomic: 100,
            daily_max_atomic: 100,
            monthly_max_atomic: 100,
        },
    );

    for index in 0..=MAX_PENDING {
        let reservation = ledger::reserve(1).unwrap().unwrap();
        runtime
            .store_pending(
                format!("payment-{index}"),
                PendingPayment {
                    payment: tinywallet_x402::protocol::X402PaymentResult {
                        header_value: "proof".to_string(),
                        amount_atomic: 1,
                        asset: "usdc".to_string(),
                        recipient: "recipient".to_string(),
                        network: "eip155:1".to_string(),
                        url: "https://example.invalid".to_string(),
                        reservation,
                    },
                    created_at: Instant::now(),
                    thread_id: None,
                },
            )
            .unwrap();
    }

    let pending = runtime.pending.lock().unwrap();
    assert_eq!(pending.len(), MAX_PENDING);
    assert!(!pending.contains_key("payment-0"));
    assert!(pending.contains_key(&format!("payment-{MAX_PENDING}")));
    drop(pending);
    assert_eq!(
        runtime.summary(&runtime.handle()).unwrap().reserved_atomic,
        64
    );
    runtime.close(&runtime.handle()).unwrap();
    broker_task.abort();
}

#[tokio::test]
async fn pruning_expired_pending_payments_releases_their_reservations() {
    let _ledger_guard = LEDGER_TEST_LOCK.lock().await;
    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker = tinybus::broker::Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let connection = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mut runtime = X402Runtime::new(
        HostCallbacks::new(&connection).unwrap(),
        X402ConfigResponse {
            workspace_dir: workspace.path().display().to_string(),
            session_id: "prune-test".to_string(),
            per_request_max_atomic: 100,
            daily_max_atomic: 100,
            monthly_max_atomic: 100,
        },
    );
    let reservation = ledger::reserve(1).unwrap().unwrap();
    runtime
        .store_pending(
            "expired".to_string(),
            PendingPayment {
                payment: tinywallet_x402::protocol::X402PaymentResult {
                    header_value: "proof".to_string(),
                    amount_atomic: 1,
                    asset: "usdc".to_string(),
                    recipient: "recipient".to_string(),
                    network: "eip155:1".to_string(),
                    url: "https://example.invalid".to_string(),
                    reservation,
                },
                created_at: Instant::now()
                    .checked_sub(PENDING_TTL + Duration::from_secs(1))
                    .unwrap(),
                thread_id: None,
            },
        )
        .unwrap();

    runtime.prune_pending();

    assert_eq!(runtime.pending.lock().unwrap().len(), 0);
    assert_eq!(
        runtime.summary(&runtime.handle()).unwrap().reserved_atomic,
        0
    );
    runtime.close(&runtime.handle()).unwrap();
    broker_task.abort();
}
