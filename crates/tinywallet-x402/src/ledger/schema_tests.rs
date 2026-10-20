//! Tests for owner-local x402 schema declarations and open-time migrations.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use tinystoragedrivers_types::{Precondition, Scope, WriteOptions};

use super::super::{LedgerStorage, PaymentLedger, PaymentStatus, SpendingBudget};
use super::{BUDGETS_COLLECTION, HOLDS_ID, collections};
use crate::ledger::blocking::Blocking;

const V1_PAYMENT_MS: i64 = 1_773_576_000_000;

fn test_storage() -> tinystoragedrivers_api::Storage {
    tinystoragedrivers_testkit::memory_storage(&Scope::new("wallet.migration-test").unwrap())
}

fn legacy_payment() -> Value {
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

#[test]
fn host_declarations_are_pure_wallet_collection_specs() {
    let declared = collections();
    assert_eq!(declared.len(), 2);
    assert!(declared.iter().all(|collection| collection.name.starts_with("wallet.")));
    assert!(declared.iter().any(|collection| collection.name == BUDGETS_COLLECTION));
}

#[test]
fn opening_the_ledger_migrates_legacy_payments_before_reading_them() {
    let storage = test_storage();
    let streams = std::sync::Arc::clone(storage.streams());
    Blocking::new()
        .unwrap()
        .run(async move {
            streams.append("x402.payments", legacy_payment()).await.unwrap();
        })
        .unwrap();

    let ledger = PaymentLedger::open(
        LedgerStorage::from_storage(&storage),
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

    let reopened = PaymentLedger::open(
        LedgerStorage::from_storage(&storage),
        "session-a",
        SpendingBudget::default(),
    )
    .unwrap();
    assert_eq!(reopened.recent_payments(10), records);
}

#[test]
fn opening_the_ledger_migrates_legacy_holds_and_keeps_record_metadata() {
    let storage = test_storage();
    let documents = std::sync::Arc::clone(storage.documents());
    Blocking::new()
        .unwrap()
        .run(async move {
            documents
                .put_with(
                    "x402_budget",
                    HOLDS_ID,
                    json!({
                        "holds": {
                            "old-writer:7": {
                                "amount": 321,
                                "expires_at_ms": 1_800_000_000_000
                            }
                        }
                    }),
                    Precondition::Absent,
                    WriteOptions::new()
                        .schema(1)
                        .created_ms(1_700_000_000_000)
                        .expires_ms(1_900_000_000_000),
                )
                .await
                .unwrap();
        })
        .unwrap();

    let _ledger = PaymentLedger::open(
        LedgerStorage::from_storage(&storage),
        "session-a",
        SpendingBudget::default(),
    )
    .unwrap();
    let documents = std::sync::Arc::clone(storage.documents());
    let migrated = Blocking::new()
        .unwrap()
        .run(async move { documents.get(BUDGETS_COLLECTION, HOLDS_ID).await.unwrap().unwrap() })
        .unwrap();
    assert_eq!(
        migrated.doc,
        json!({
            "holds": {
                "old-writer:7": { "amount": 321, "expires_ms": 1_800_000_000_000_i64 }
            }
        })
    );
    assert_eq!(migrated.meta.schema, 2);
    assert_eq!(migrated.meta.created_ms, 1_700_000_000_000);
    assert_eq!(migrated.meta.expires_ms, Some(1_900_000_000_000));
}
