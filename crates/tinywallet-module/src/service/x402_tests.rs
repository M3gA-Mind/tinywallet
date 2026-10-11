#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

static LEDGER_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn pending_record(
    id: &str,
    authorization_valid_until: Option<chrono::DateTime<chrono::Utc>>,
) -> PaymentRecord {
    PaymentRecord {
        id: id.to_string(),
        url: "https://example.invalid".to_string(),
        asset: "usdc".to_string(),
        amount_atomic: 1,
        amount_display: "1 atomic".to_string(),
        recipient: "recipient".to_string(),
        network: "eip155:1".to_string(),
        tx_signature: None,
        status: PaymentStatus::Pending,
        timestamp: chrono::Utc::now(),
        authorization_valid_until,
        session_id: "coverage-test".to_string(),
        thread_id: None,
    }
}

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
    reason = "One lifecycle fixture verifies handle validation, authorization retention, and record behavior"
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
    assert!(missing_payment.contains("missing"));
    ledger::with_ledger_mut(|ledger| {
        ledger.try_record_payment(pending_record("failed-payment", None))
    })
    .unwrap()
    .unwrap();
    runtime
        .record(X402RecordPaymentRequest {
            service: handle.clone(),
            payment_id: "failed-payment".to_string(),
            response_status: 500,
            settlement_response: None,
        })
        .unwrap();
    let unresolved = runtime.summary(&handle).unwrap();
    assert_eq!(unresolved.reserved_atomic, 1);
    assert_eq!(
        ledger::with_ledger(ledger::PaymentLedger::pending_count).unwrap(),
        1
    );

    let expired_record = pending_record(
        "expired-payment",
        Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
    );
    ledger::with_ledger_mut(|ledger| ledger.try_record_payment(expired_record))
        .unwrap()
        .unwrap();
    let after_expiry = runtime.summary(&handle).unwrap();
    assert_eq!(after_expiry.reserved_atomic, 2);
    assert_eq!(
        ledger::with_ledger(|ledger| ledger.payment("expired-payment").unwrap().status).unwrap(),
        PaymentStatus::Pending,
        "expiry alone cannot prove an authorization did not settle before the deadline"
    );
    runtime
        .record(X402RecordPaymentRequest {
            service: handle.clone(),
            payment_id: "expired-payment".to_string(),
            response_status: 500,
            settlement_response: None,
        })
        .unwrap();
    let summary = runtime.summary(&handle).unwrap();
    assert_eq!(summary.reserved_atomic, 2);
    assert_eq!(
        ledger::with_ledger(ledger::PaymentLedger::pending_count).unwrap(),
        2
    );

    let receipt = BASE64.encode(
        serde_json::to_vec(&serde_json::json!({
            "success": true,
            "transaction": "0xsettled",
            "network": "eip155:1"
        }))
        .unwrap(),
    );
    runtime
        .record(X402RecordPaymentRequest {
            service: handle.clone(),
            payment_id: "expired-payment".to_string(),
            response_status: 503,
            settlement_response: Some(receipt),
        })
        .expect("trusted late settlement evidence still finalizes an expired handle");
    let summary = runtime.summary(&handle).unwrap();
    assert_eq!(summary.session_total_atomic, 1);
    assert_eq!(
        summary.reserved_atomic, 1,
        "the unresolved authorization stays held"
    );

    let mut legacy_expired = pending_record(
        "legacy-expired-payment",
        Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
    );
    legacy_expired.status = PaymentStatus::Failed;
    ledger::with_ledger_mut(|ledger| ledger.try_record_payment(legacy_expired))
        .unwrap()
        .unwrap();
    assert_eq!(runtime.summary(&handle).unwrap().reserved_atomic, 2);
    let legacy_receipt = BASE64.encode(
        serde_json::to_vec(&serde_json::json!({
            "success": true,
            "transaction": "0xlegacy-settled",
            "network": "eip155:1"
        }))
        .unwrap(),
    );
    runtime
        .record(X402RecordPaymentRequest {
            service: handle.clone(),
            payment_id: "legacy-expired-payment".to_string(),
            response_status: 503,
            settlement_response: Some(legacy_receipt.clone()),
        })
        .unwrap();
    runtime
        .record(X402RecordPaymentRequest {
            service: handle.clone(),
            payment_id: "legacy-expired-payment".to_string(),
            response_status: 503,
            settlement_response: Some(legacy_receipt),
        })
        .expect("a lost reply after durable settlement is safe to replay");
    let summary = runtime.summary(&handle).unwrap();
    assert_eq!(summary.session_total_atomic, 2);
    assert_eq!(summary.reserved_atomic, 1);
    runtime.close(&handle).unwrap();
    broker_task.abort();
}

#[derive(Clone)]
struct PaymentApprovalHost {
    approved: Arc<AtomicBool>,
    authorizations: Arc<AtomicUsize>,
    pause: Arc<AtomicBool>,
    approval_started: Arc<tokio::sync::Notify>,
    release_approval: Arc<tokio::sync::Notify>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinywallet.Host")]
impl PaymentApprovalHost {
    async fn authorize_x402_payment(
        &self,
        _request: tinywallet_bus::wire::AuthorizeX402PaymentRequest,
    ) -> tinybus::Result<tinywallet_bus::wire::HostResponse> {
        self.authorizations.fetch_add(1, Ordering::SeqCst);
        if self.pause.swap(false, Ordering::SeqCst) {
            self.approval_started.notify_one();
            self.release_approval.notified().await;
        }
        Ok(tinywallet_bus::wire::HostResponse::Authorization(
            tinywallet_bus::wire::AuthorizationResult {
                approved: self.approved.load(Ordering::SeqCst),
                reason: Some("fixture approval decision".to_string()),
            },
        ))
    }
}

fn valid_challenge_header() -> String {
    BASE64.encode(
        serde_json::to_vec(&serde_json::json!({
            "x402Version": 2,
            "resource": {"url": "https://wallet.fixture.test/resource"},
            "accepts": [{
                "scheme": "exact",
                "network": "eip155:1",
                "amount": "100",
                "asset": "0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                "payTo": "0x3535353535353535353535353535353535353535",
                "maxTimeoutSeconds": 60
            }]
        }))
        .unwrap(),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One MemoryBus scenario exercises the x402 service lifecycle and cancellation cleanup"
)]
async fn x402_service_lifecycle_records_approval_denial_and_cancellation() {
    use tinywallet_bus::wire::{
        X402CloseRequest, X402ConfigureRequest, X402PreparePaymentRequest,
        X402RecordPaymentRequest, X402SummaryRequest,
    };

    let _ledger_guard = LEDGER_TEST_LOCK.lock().await;
    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker = tinybus::broker::Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let approved = Arc::new(AtomicBool::new(true));
    let authorizations = Arc::new(AtomicUsize::new(0));
    let pause = Arc::new(AtomicBool::new(false));
    let approval_started = Arc::new(tokio::sync::Notify::new());
    let release_approval = Arc::new(tokio::sync::Notify::new());
    host.serve_at(
        crate::service::HOST_OBJECT_PATH.try_into().unwrap(),
        PaymentApprovalHost {
            approved: approved.clone(),
            authorizations: authorizations.clone(),
            pause: pause.clone(),
            approval_started: approval_started.clone(),
            release_approval: release_approval.clone(),
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
    let callbacks = HostCallbacks::new(&client).unwrap();
    let wallet = Arc::new(crate::service::Wallet {
        callbacks: Some(callbacks),
        ..crate::service::Wallet::default()
    });
    let workspace = tempfile::tempdir().unwrap();
    let handle = wallet
        .x402_configure(X402ConfigureRequest {
            config: X402ConfigResponse {
                workspace_dir: workspace.path().display().to_string(),
                session_id: "wallet-service-lifecycle".to_string(),
                per_request_max_atomic: 1_000,
                daily_max_atomic: 2_000,
                monthly_max_atomic: 3_000,
            },
        })
        .await
        .unwrap();
    let replaced = wallet
        .x402_configure(X402ConfigureRequest {
            config: X402ConfigResponse {
                workspace_dir: workspace.path().display().to_string(),
                session_id: "wallet-service-lifecycle-replaced".to_string(),
                per_request_max_atomic: 1_000,
                daily_max_atomic: 2_000,
                monthly_max_atomic: 3_000,
            },
        })
        .await
        .unwrap();
    assert_ne!(handle.handle, replaced.handle);
    assert!(
        wallet
            .x402_spending_summary(X402SummaryRequest { service: handle })
            .await
            .is_err()
    );
    let handle = replaced;

    let prepared = wallet
        .x402_prepare_payment(X402PreparePaymentRequest {
            service: handle.clone(),
            url: "https://wallet.fixture.test/resource".to_string(),
            payment_required: valid_challenge_header(),
            owner: Some(tinywallet_bus::wire::QuoteOwner {
                thread_id: "fixture-thread".to_string(),
                client_id: "fixture-client".to_string(),
            }),
            secret: secret(tinywallet_bus::Chain::Evm),
        })
        .await
        .unwrap();
    assert_eq!(prepared.amount_atomic, 100);
    assert!(prepared.payment_header.starts_with("ey"));
    assert!(
        uuid::Uuid::parse_str(prepared.payment_id.strip_prefix("x402_").unwrap()).is_ok(),
        "payment handles must stay collision-safe across process restarts"
    );
    wallet
        .x402_close(X402CloseRequest {
            service: handle.clone(),
        })
        .await
        .unwrap();
    let handle = wallet
        .x402_configure(X402ConfigureRequest {
            config: X402ConfigResponse {
                workspace_dir: workspace.path().display().to_string(),
                session_id: "wallet-service-lifecycle-reopened".to_string(),
                per_request_max_atomic: 1_000,
                daily_max_atomic: 2_000,
                monthly_max_atomic: 3_000,
            },
        })
        .await
        .unwrap();
    let held = wallet
        .x402_spending_summary(X402SummaryRequest {
            service: handle.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        held.reserved_atomic, 100,
        "closing and reopening must preserve a signed authorization's budget hold"
    );

    wallet
        .x402_record_payment(X402RecordPaymentRequest {
            service: handle.clone(),
            payment_id: prepared.payment_id.clone(),
            response_status: 503,
            settlement_response: None,
        })
        .await
        .unwrap();
    let unresolved = wallet
        .x402_spending_summary(X402SummaryRequest {
            service: handle.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        unresolved.reserved_atomic, 100,
        "an HTTP failure without settlement evidence cannot release a still-valid authorization"
    );

    wallet
        .x402_record_payment(X402RecordPaymentRequest {
            service: handle.clone(),
            payment_id: prepared.payment_id.clone(),
            response_status: 200,
            settlement_response: Some(
                BASE64.encode(
                    serde_json::to_vec(&serde_json::json!({
                        "success": true,
                        "transaction": "0xsettled",
                        "network": "eip155:1"
                    }))
                    .unwrap(),
                ),
            ),
        })
        .await
        .unwrap();
    let settled = wallet
        .x402_spending_summary(X402SummaryRequest {
            service: handle.clone(),
        })
        .await
        .unwrap();
    assert_eq!(settled.daily_total_atomic, 100);
    assert_eq!(settled.reserved_atomic, 0);

    approved.store(false, Ordering::SeqCst);
    let denied = wallet
        .x402_prepare_payment(X402PreparePaymentRequest {
            service: handle.clone(),
            url: "https://wallet.fixture.test/resource".to_string(),
            payment_required: valid_challenge_header(),
            owner: None,
            secret: secret(tinywallet_bus::Chain::Evm),
        })
        .await
        .unwrap_err();
    assert!(denied.to_string().contains("X402OperationFailed"));
    assert_eq!(
        wallet
            .x402_spending_summary(X402SummaryRequest {
                service: handle.clone(),
            })
            .await
            .unwrap()
            .reserved_atomic,
        0
    );

    approved.store(true, Ordering::SeqCst);
    pause.store(true, Ordering::SeqCst);
    let pending_wallet = wallet.clone();
    let pending_handle = handle.clone();
    let task = tokio::spawn(async move {
        pending_wallet
            .x402_prepare_payment(X402PreparePaymentRequest {
                service: pending_handle,
                url: "https://wallet.fixture.test/resource".to_string(),
                payment_required: valid_challenge_header(),
                owner: None,
                secret: secret(tinywallet_bus::Chain::Evm),
            })
            .await
    });
    approval_started.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    release_approval.notify_one();
    pause.store(false, Ordering::SeqCst);
    assert_eq!(
        wallet
            .x402_spending_summary(X402SummaryRequest {
                service: handle.clone(),
            })
            .await
            .unwrap()
            .reserved_atomic,
        0
    );

    wallet
        .x402_close(X402CloseRequest {
            service: handle.clone(),
        })
        .await
        .unwrap();
    assert!(
        wallet
            .x402_spending_summary(X402SummaryRequest { service: handle })
            .await
            .is_err()
    );

    let blocked_workspace = tempfile::tempdir().unwrap();
    std::fs::write(blocked_workspace.path().join("x402"), "not a directory").unwrap();
    let blocked_handle = wallet
        .x402_configure(X402ConfigureRequest {
            config: X402ConfigResponse {
                workspace_dir: blocked_workspace.path().display().to_string(),
                session_id: "unwritable-ledger".to_string(),
                per_request_max_atomic: 1_000,
                daily_max_atomic: 2_000,
                monthly_max_atomic: 3_000,
            },
        })
        .await
        .unwrap();
    let authorizations_before_unpersisted = authorizations.load(Ordering::SeqCst);
    let unpersisted = wallet
        .x402_prepare_payment(X402PreparePaymentRequest {
            service: blocked_handle.clone(),
            url: "https://wallet.fixture.test/resource".to_string(),
            payment_required: valid_challenge_header(),
            owner: None,
            secret: secret(tinywallet_bus::Chain::Evm),
        })
        .await;
    assert!(
        unpersisted.is_err(),
        "the signed header must not leave the module if durable budget accounting fails"
    );
    assert_eq!(
        authorizations.load(Ordering::SeqCst),
        authorizations_before_unpersisted + 1,
        "the fixture reaches the real approval and signing path before the durable-write refusal"
    );
    wallet
        .x402_close(X402CloseRequest {
            service: blocked_handle,
        })
        .await
        .unwrap();
    broker_task.abort();
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "The MemoryBus fixture proves capacity ordering, secret wiping, and cancellation cleanup"
)]
async fn x402_pending_capacity_is_reserved_before_authorization_and_released_on_cancel() {
    use tinywallet_bus::wire::{X402CloseRequest, X402ConfigureRequest, X402PreparePaymentRequest};

    let _ledger_guard = LEDGER_TEST_LOCK.lock().await;
    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker = tinybus::broker::Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let approved = Arc::new(AtomicBool::new(true));
    let authorizations = Arc::new(AtomicUsize::new(0));
    let pause = Arc::new(AtomicBool::new(false));
    let approval_started = Arc::new(tokio::sync::Notify::new());
    let release_approval = Arc::new(tokio::sync::Notify::new());
    host.serve_at(
        crate::service::HOST_OBJECT_PATH.try_into().unwrap(),
        PaymentApprovalHost {
            approved,
            authorizations: authorizations.clone(),
            pause: pause.clone(),
            approval_started: approval_started.clone(),
            release_approval: release_approval.clone(),
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
    let wallet = Arc::new(crate::service::Wallet {
        callbacks: Some(HostCallbacks::new(&client).unwrap()),
        ..crate::service::Wallet::default()
    });
    let workspace = tempfile::tempdir().unwrap();
    let handle = wallet
        .x402_configure(X402ConfigureRequest {
            config: X402ConfigResponse {
                workspace_dir: workspace.path().display().to_string(),
                session_id: "pending-capacity-test".to_string(),
                per_request_max_atomic: 1_000,
                daily_max_atomic: 100_000,
                monthly_max_atomic: 100_000,
            },
        })
        .await
        .unwrap();

    let prepare = || X402PreparePaymentRequest {
        service: handle.clone(),
        url: "https://wallet.fixture.test/resource".to_string(),
        payment_required: valid_challenge_header(),
        owner: None,
        secret: secret(tinywallet_bus::Chain::Evm),
    };
    for _ in 0..(MAX_PENDING - 1) {
        wallet.x402_prepare_payment(prepare()).await.unwrap();
    }
    assert_eq!(authorizations.load(Ordering::SeqCst), MAX_PENDING - 1);

    pause.store(true, Ordering::SeqCst);
    let pending_wallet = wallet.clone();
    let pending_request = prepare();
    let admitted =
        tokio::spawn(async move { pending_wallet.x402_prepare_payment(pending_request).await });
    approval_started.notified().await;

    let mut rejected_request = prepare();
    let runtime_guard = wallet.x402.read().await;
    let capacity_error = runtime_guard
        .as_ref()
        .unwrap()
        .admit_request(&mut rejected_request)
        .err()
        .unwrap();
    drop(runtime_guard);
    assert!(capacity_error.contains("capacity reached"));
    assert_eq!(
        rejected_request.secret.mnemonic, "",
        "a rejected capacity admission wipes the host-provided secret"
    );

    let over_capacity = wallet.x402_prepare_payment(prepare()).await;
    assert!(
        over_capacity.is_err(),
        "a full store rejects before it creates another spend authorization"
    );
    assert_eq!(
        authorizations.load(Ordering::SeqCst),
        MAX_PENDING,
        "the rejected request must not reach host approval or signing"
    );

    admitted.abort();
    assert!(admitted.await.unwrap_err().is_cancelled());
    release_approval.notify_one();
    pause.store(false, Ordering::SeqCst);

    wallet
        .x402_prepare_payment(prepare())
        .await
        .expect("cancellation releases its in-flight capacity slot");
    assert_eq!(authorizations.load(Ordering::SeqCst), MAX_PENDING + 1);

    wallet
        .x402_close(X402CloseRequest { service: handle })
        .await
        .unwrap();
    broker_task.abort();
}
