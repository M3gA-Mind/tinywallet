//! Tests for owner-local x402 schema declarations and open-time migrations.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tinystoragedrivers_api::StreamStore;
use tinystoragedrivers_types::{Capabilities, Scope, SealOutcome, StreamEntry, StreamInfo};

use super::super::{LedgerStorage, PaymentLedger, PaymentStatus, SpendingBudget};
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

fn legacy_payments(count: usize) -> Vec<Value> {
    (0..count)
        .map(|index| {
            let mut payment = legacy_payment();
            payment["id"] = json!(format!("pay-{index}"));
            payment
        })
        .collect()
}

fn append_values(
    storage: &tinystoragedrivers_api::Storage,
    stream: &'static str,
    values: Vec<Value>,
) {
    let streams = Arc::clone(storage.streams());
    Blocking::new()
        .unwrap()
        .run(async move {
            streams.append_batch(stream, values).await.unwrap();
        })
        .unwrap();
}

fn assert_migration_not_marked(storage: &tinystoragedrivers_api::Storage) {
    let documents = Arc::clone(storage.documents());
    assert!(
        Blocking::new()
            .unwrap()
            .run(async move { documents.get("wallet.migrations", "x402").await })
            .unwrap()
            .unwrap()
            .is_none()
    );
}

#[derive(Debug, Clone, Copy)]
enum ReadFault {
    EmptyAfterFirstPage(&'static str),
    GrowthAfterSnapshot(&'static str),
    Truncate(&'static str, usize),
}

#[derive(Debug)]
struct FaultyReads {
    inner: Arc<dyn StreamStore>,
    fault: ReadFault,
    len_calls: AtomicUsize,
}

#[async_trait]
impl StreamStore for FaultyReads {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn append(&self, stream: &str, value: Value) -> tinystoragedrivers_types::Result<u64> {
        self.inner.append(stream, value).await
    }

    async fn append_batch(
        &self,
        stream: &str,
        values: Vec<Value>,
    ) -> tinystoragedrivers_types::Result<u64> {
        self.inner.append_batch(stream, values).await
    }

    async fn append_batch_if_offset(
        &self,
        stream: &str,
        expected_next_offset: u64,
        values: Vec<Value>,
    ) -> tinystoragedrivers_types::Result<u64> {
        self.inner
            .append_batch_if_offset(stream, expected_next_offset, values)
            .await
    }

    async fn read_window(
        &self,
        stream: &str,
        from: u64,
        limit: usize,
    ) -> tinystoragedrivers_types::Result<Vec<StreamEntry>> {
        match self.fault {
            ReadFault::EmptyAfterFirstPage(name) if stream == name && from > 0 => Ok(Vec::new()),
            ReadFault::Truncate(name, count) if stream == name => {
                let mut entries = self.inner.read_window(stream, from, limit).await?;
                entries.truncate(count);
                Ok(entries)
            }
            _ => self.inner.read_window(stream, from, limit).await,
        }
    }

    async fn len(&self, stream: &str) -> tinystoragedrivers_types::Result<u64> {
        let len = self.inner.len(stream).await?;
        if let ReadFault::GrowthAfterSnapshot(name) = self.fault
            && stream == name
            && self.len_calls.fetch_add(1, Ordering::Relaxed) > 0
        {
            return Ok(len.saturating_add(1));
        }
        Ok(len)
    }

    async fn truncate_before(
        &self,
        stream: &str,
        offset: u64,
    ) -> tinystoragedrivers_types::Result<u64> {
        self.inner.truncate_before(stream, offset).await
    }

    async fn delete_stream(&self, stream: &str) -> tinystoragedrivers_types::Result<bool> {
        self.inner.delete_stream(stream).await
    }

    async fn streams(&self, prefix: &str) -> tinystoragedrivers_types::Result<Vec<String>> {
        self.inner.streams(prefix).await
    }

    async fn seal(&self, stream: &str) -> tinystoragedrivers_types::Result<SealOutcome> {
        self.inner.seal(stream).await
    }

    async fn delete_sealed_stream(
        &self,
        stream: &str,
        sealed_ms: i64,
    ) -> tinystoragedrivers_types::Result<bool> {
        self.inner.delete_sealed_stream(stream, sealed_ms).await
    }

    async fn info(&self, stream: &str) -> tinystoragedrivers_types::Result<Option<StreamInfo>> {
        self.inner.info(stream).await
    }
}

#[test]
fn host_declarations_are_pure_wallet_collection_specs() {
    let declared = collections();
    assert_eq!(declared.len(), 2);
    assert!(
        declared
            .iter()
            .all(|collection| collection.name.starts_with("wallet."))
    );
    assert!(
        declared
            .iter()
            .any(|collection| collection.name == BUDGETS_COLLECTION)
    );
}

#[test]
fn opening_the_ledger_migrates_legacy_payments_before_reading_them() {
    let storage = test_storage();
    let streams = std::sync::Arc::clone(storage.streams());
    Blocking::new()
        .unwrap()
        .run(async move {
            streams
                .append("x402.payments", legacy_payment())
                .await
                .unwrap();
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
                                "expires_at_ms": 1_800_000_000_000_i64
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
        .run(async move {
            documents
                .get(BUDGETS_COLLECTION, HOLDS_ID)
                .await
                .unwrap()
                .unwrap()
        })
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
    assert_ne!(migrated.meta.created_ms, 1_700_000_000_000);
    assert_eq!(migrated.meta.expires_ms, Some(1_900_000_000_000));
}

#[test]
fn incomplete_legacy_payment_window_does_not_mark_migration_complete() {
    let storage = test_storage();
    append_values(&storage, "x402.payments", legacy_payments(513));
    let streams: Arc<dyn StreamStore> = Arc::new(FaultyReads {
        inner: Arc::clone(storage.streams()),
        fault: ReadFault::EmptyAfterFirstPage("x402.payments"),
        len_calls: AtomicUsize::new(0),
    });

    let documents = Arc::clone(storage.documents());
    let result = Blocking::new()
        .unwrap()
        .run(async move { super::migrate(documents.as_ref(), streams.as_ref()).await });

    assert!(matches!(result, Ok(Err(_))));
    assert_migration_not_marked(&storage);
}

#[test]
fn incomplete_existing_payment_prefix_does_not_mark_migration_complete() {
    let storage = test_storage();
    let source = legacy_payments(2);
    append_values(&storage, "x402.payments", source.clone());
    let migrated = super::migrate_payments_stream("x402.payments", source)
        .unwrap()
        .unwrap()
        .1;
    append_values(&storage, "wallet.payments", migrated);
    let streams: Arc<dyn StreamStore> = Arc::new(FaultyReads {
        inner: Arc::clone(storage.streams()),
        fault: ReadFault::Truncate("wallet.payments", 1),
        len_calls: AtomicUsize::new(0),
    });

    let documents = Arc::clone(storage.documents());
    let result = Blocking::new()
        .unwrap()
        .run(async move { super::migrate(documents.as_ref(), streams.as_ref()).await });

    assert!(matches!(result, Ok(Err(_))));
    assert_migration_not_marked(&storage);
}

#[test]
fn changed_legacy_payment_source_does_not_mark_migration_complete() {
    let storage = test_storage();
    append_values(&storage, "x402.payments", legacy_payments(1));
    let streams: Arc<dyn StreamStore> = Arc::new(FaultyReads {
        inner: Arc::clone(storage.streams()),
        fault: ReadFault::GrowthAfterSnapshot("x402.payments"),
        len_calls: AtomicUsize::new(0),
    });
    let documents = Arc::clone(storage.documents());

    let result = Blocking::new()
        .unwrap()
        .run(async move { super::migrate(documents.as_ref(), streams.as_ref()).await });

    assert!(matches!(result, Ok(Err(_))));
    assert_migration_not_marked(&storage);
}
