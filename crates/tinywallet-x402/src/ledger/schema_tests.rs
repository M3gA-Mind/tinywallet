//! Tests for the wallet storage schema: its declarations compose, and the v1
//! `x402` records migrate into the v2 collection and stream.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

use std::sync::Arc;

use serde_json::json;
use tinystoragedrivers_core::{
    Blocking, Item, MemoryStorage, MigrateOptions, Precondition, Registry, Scope, ScopedStorage,
    StorageBackend,
};

use super::super::{LedgerStorage, PaymentLedger, PaymentStatus, SpendingBudget};

const V1_PAYMENT_MS: i64 = 1_773_576_000_000; // 2026-03-15T12:00:00Z

fn v1_payment() -> Value {
    json!({
        "id": "pay-1",
        "url": "https://api.example.com/data",
        "asset": "USDC",
        "amountAtomic": 2500,
        "amountDisplay": "0.002500 USDC",
        "recipient": "Recipient",
        "network": "eip155:8453",
        "txSignature": "0xabc",
        "status": "settled",
        "timestamp": "2026-03-15T12:00:00Z",
        "authorizationValidUntil": "2026-03-15T12:05:00.250Z",
        "sessionId": "session-a",
        "threadId": "thread-1",
        "committed_hold": "owner:1"
    })
}

fn scoped() -> (MemoryStorage, ScopedStorage) {
    let backend = MemoryStorage::new();
    let scoped = backend
        .for_scope(&Scope::local().module("wallet").unwrap())
        .unwrap();
    (backend, scoped)
}

/// Declare the v2 items on `backend` and run the steps over `scoped`.
fn migrate(backend: &MemoryStorage, scoped: &ScopedStorage) -> Registry {
    let registry = Registry::compose([storage_schema()]).unwrap();
    let bridge = Blocking::new().unwrap();
    let backend = backend.clone();
    let scoped = scoped.clone();
    let registry = bridge
        .run(async move {
            registry.prepare(&backend).await.unwrap();
            registry
                .migrate(&scoped, &MigrateOptions::new())
                .await
                .unwrap();
            registry
        })
        .unwrap();
    registry
}

fn put_v1(scoped: &ScopedStorage, collection: &'static str, id: &'static str, body: Value) {
    let documents = Arc::clone(scoped.documents());
    Blocking::new()
        .unwrap()
        .run(async move {
            documents
                .ensure_collection(&CollectionSpec::new(collection))
                .await
                .unwrap();
            documents
                .put(collection, id, body, Precondition::None)
                .await
                .unwrap();
        })
        .unwrap();
}

fn append_v1(scoped: &ScopedStorage, entries: Vec<Value>) {
    let streams = Arc::clone(scoped.streams());
    Blocking::new()
        .unwrap()
        .run(async move {
            for entry in entries {
                streams.append("x402.payments", entry).await.unwrap();
            }
        })
        .unwrap();
}

#[test]
fn the_schema_composes_and_declares_module_scoped_wallet_items() {
    let schema = storage_schema();
    assert_eq!(schema.owner.as_str(), "wallet");
    Registry::compose([schema.clone()]).unwrap();

    let mut names: Vec<(String, ScopeClass)> = schema
        .items
        .iter()
        .map(|item| (item.name().to_owned(), item.scope()))
        .collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        names,
        vec![
            ("wallet.budgets".to_owned(), ScopeClass::Module),
            ("wallet.payments".to_owned(), ScopeClass::Module),
        ]
    );
    assert!(
        schema
            .items
            .iter()
            .any(|item| matches!(item, Item::Collection { version: 2, .. }))
    );
}

#[test]
fn v1_payments_migrate_to_snake_case_millisecond_entries() {
    let (backend, scoped) = scoped();
    append_v1(&scoped, vec![v1_payment()]);

    migrate(&backend, &scoped);

    let streams = Arc::clone(scoped.streams());
    let (migrated, legacy_left) = Blocking::new()
        .unwrap()
        .run(async move {
            (
                streams.read_window("wallet.payments", 0, 10).await.unwrap(),
                streams.len("x402.payments").await.unwrap(),
            )
        })
        .unwrap();
    assert_eq!(legacy_left, 0, "the v1 stream is removed");
    assert_eq!(migrated.len(), 1);
    assert_eq!(
        migrated[0].value,
        json!({
            "id": "pay-1",
            "url": "https://api.example.com/data",
            "asset": "USDC",
            "amount_atomic": 2500,
            "amount_display": "0.002500 USDC",
            "recipient": "Recipient",
            "network": "eip155:8453",
            "tx_signature": "0xabc",
            "status": "settled",
            "timestamp_ms": V1_PAYMENT_MS,
            "authorization_valid_until_ms": V1_PAYMENT_MS + 300_250,
            "session_id": "session-a",
            "thread_id": "thread-1",
            "committed_hold": "owner:1"
        })
    );
}

#[test]
fn migrated_payments_count_against_the_budget_of_a_reopened_ledger() {
    let (backend, scoped) = scoped();
    append_v1(&scoped, vec![v1_payment()]);
    migrate(&backend, &scoped);

    let ledger = PaymentLedger::open(
        LedgerStorage::from_scoped(&scoped),
        "session-a",
        SpendingBudget::default(),
    )
    .unwrap();
    let records = ledger.recent_payments(10);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "pay-1");
    assert_eq!(records[0].status, PaymentStatus::Settled);
    assert_eq!(records[0].amount_atomic, 2500);
    assert_eq!(records[0].timestamp.timestamp_millis(), V1_PAYMENT_MS);
    assert_eq!(records[0].thread_id.as_deref(), Some("thread-1"));
}

#[test]
fn unreadable_v1_payment_entries_are_dropped() {
    let (backend, scoped) = scoped();
    append_v1(
        &scoped,
        vec![json!("not a record"), v1_payment(), json!({"id": 7})],
    );

    migrate(&backend, &scoped);

    let streams = Arc::clone(scoped.streams());
    let migrated = Blocking::new()
        .unwrap()
        .run(async move { streams.read_window("wallet.payments", 0, 10).await.unwrap() })
        .unwrap();
    assert_eq!(migrated.len(), 1);
}

#[test]
fn v1_holds_migrate_with_expires_ms_and_keep_their_creation_time() {
    let (backend, scoped) = scoped();
    put_v1(
        &scoped,
        "x402_budget",
        "holds",
        json!({"holds": {
            "owner:1": {"amount": 40, "expires_at_ms": 1_800_000_000_000_i64},
            "owner:2": {"amount": 7, "expires_at_ms": 1_800_000_500_000_i64}
        }}),
    );

    migrate(&backend, &scoped);

    let documents = Arc::clone(scoped.documents());
    let (found, legacy) = Blocking::new()
        .unwrap()
        .run(async move {
            (
                documents.get("wallet.budgets", "holds").await.unwrap(),
                documents.get("x402_budget", "holds").await.unwrap(),
            )
        })
        .unwrap();
    assert!(legacy.is_none(), "the v1 collection is emptied");
    let found = found.expect("the hold document was migrated");
    assert_eq!(
        found.doc,
        json!({"holds": {
            "owner:1": {"amount": 40, "expires_ms": 1_800_000_000_000_i64},
            "owner:2": {"amount": 7, "expires_ms": 1_800_000_500_000_i64}
        }})
    );
    assert_eq!(found.meta.schema, 2);
}

#[test]
fn an_unreadable_v1_hold_document_migrates_to_no_holds() {
    let (backend, scoped) = scoped();
    put_v1(&scoped, "x402_budget", "holds", json!({"holds": 5}));

    migrate(&backend, &scoped);

    let documents = Arc::clone(scoped.documents());
    let found = Blocking::new()
        .unwrap()
        .run(async move { documents.get("wallet.budgets", "holds").await.unwrap() })
        .unwrap()
        .unwrap();
    assert_eq!(found.doc, json!({"holds": {}}));
}

#[test]
fn migrating_twice_changes_nothing() {
    let (backend, scoped) = scoped();
    append_v1(&scoped, vec![v1_payment()]);
    put_v1(
        &scoped,
        "x402_budget",
        "holds",
        json!({"holds": {"owner:1": {"amount": 1, "expires_at_ms": 5}}}),
    );
    migrate(&backend, &scoped);
    migrate(&backend, &scoped);

    let streams = Arc::clone(scoped.streams());
    let len = Blocking::new()
        .unwrap()
        .run(async move { streams.len("wallet.payments").await.unwrap() })
        .unwrap();
    assert_eq!(len, 1);
}
