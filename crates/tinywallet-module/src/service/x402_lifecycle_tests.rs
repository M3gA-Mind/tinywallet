//! Tests for x402 service configuration and ledger lifecycle ordering.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[tokio::test]
async fn missing_callbacks_do_not_close_an_active_x402_ledger() {
    let _ledger_guard = LEDGER_TEST_LOCK.lock().await;
    tinywallet_x402::ledger::clear_global();
    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker_task = tinybus::broker::Broker::new().spawn(bus.clone());
    let connection = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let callbacks = crate::service::host::HostCallbacks::new(&connection).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let config = || tinywallet_bus::wire::X402ConfigResponse {
        workspace_dir: workspace.path().display().to_string(),
        session_id: "callback-preflight-test".to_string(),
        per_request_max_atomic: 100,
        daily_max_atomic: 100,
        monthly_max_atomic: 100,
    };
    let mut wallet = crate::service::Wallet {
        callbacks: Some(callbacks),
        ..crate::service::Wallet::default()
    };
    let handle = wallet
        .x402_configure(tinywallet_bus::wire::X402ConfigureRequest { config: config() })
        .await
        .unwrap();

    // Model a host callback interface that becomes unavailable before a
    // replacement attempt. The active service and its ledger must survive.
    wallet.callbacks = None;
    let error = wallet
        .x402_configure(tinywallet_bus::wire::X402ConfigureRequest { config: config() })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        tinybus::Error::MethodFailed { name, .. }
            if name == "ai.tinyhumans.tinywallet.Error.HostUnavailable"
    ));
    assert!(
        wallet
            .x402_spending_summary(tinywallet_bus::wire::X402SummaryRequest { service: handle })
            .await
            .is_ok()
    );

    tinywallet_x402::ledger::clear_global();
    broker_task.abort();
}
