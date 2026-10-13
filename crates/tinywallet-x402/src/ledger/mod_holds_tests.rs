//! Tests for the budget-hold protocol: the read order that stops two writers
//! overspending, and the lease that keeps a long-lived hold charged.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::Value;
use tinystoragedrivers_core::{
    Blocking, Capabilities, CollectionSpec, DocumentStore, Filter, Page, Precondition, Query,
    Result, Sort, Version, Versioned, async_trait,
};

use super::persist::{BUDGET_COLLECTION, HOLD_TTL_MS, HOLDS_ID, HoldsDoc, Persist};
use super::*;

const SESSION: &str = "session-holds";

fn budget() -> SpendingBudget {
    SpendingBudget {
        per_request_max_atomic: 100,
        daily_max_atomic: 100,
        monthly_max_atomic: 1_000,
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap()
}

fn settled(amount: u64) -> PaymentRecord {
    PaymentRecord {
        id: format!("other-writer-{amount}"),
        url: "https://api.example.com/data".into(),
        asset: "USDC".into(),
        amount_atomic: amount,
        amount_display: format!("{amount} atomic"),
        recipient: "Recipient".into(),
        network: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".into(),
        tx_signature: Some("sig".into()),
        status: PaymentStatus::Settled,
        timestamp: now(),
        authorization_valid_until: None,
        solana_last_valid_block_height: None,
        authorization_expired_at: None,
        authorization_expiry_observed_height: None,
        session_id: "other".into(),
        thread_id: None,
    }
}

/// One action another writer performs the moment a ledger first reads the hold
/// document, before the read is answered.
struct Interleave(Box<dyn FnOnce() + Send>);

impl std::fmt::Debug for Interleave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Interleave")
    }
}

/// A document store that runs [`Interleave`] once, just before the first read
/// of the hold document.
#[derive(Debug)]
struct InterleavingDocuments {
    inner: Arc<dyn DocumentStore>,
    before_first_hold_read: Mutex<Option<Interleave>>,
}

#[async_trait]
impl DocumentStore for InterleavingDocuments {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn ensure_collection(&self, spec: &CollectionSpec) -> Result<()> {
        self.inner.ensure_collection(spec).await
    }
    async fn get(&self, collection: &str, id: &str) -> Result<Option<Versioned<Value>>> {
        let action = self.before_first_hold_read.lock().unwrap().take();
        if let Some(action) = action {
            // The other writer runs on its own thread: the bridge thread this
            // future runs on must not be blocked by it.
            std::thread::spawn(action.0).join().unwrap();
        }
        self.inner.get(collection, id).await
    }
    async fn put(&self, c: &str, id: &str, doc: Value, p: Precondition) -> Result<Version> {
        self.inner.put(c, id, doc, p).await
    }
    async fn delete(&self, c: &str, id: &str, p: Precondition) -> Result<bool> {
        self.inner.delete(c, id, p).await
    }
    async fn query(&self, c: &str, q: &Query) -> Result<Page<Versioned<Value>>> {
        self.inner.query(c, q).await
    }
    async fn count(&self, c: &str, f: &Filter) -> Result<u64> {
        self.inner.count(c, f).await
    }
    async fn delete_where(&self, c: &str, f: &Filter) -> Result<u64> {
        self.inner.delete_where(c, f).await
    }
    async fn claim(
        &self,
        c: &str,
        f: &Filter,
        sort: &[Sort],
        patch: &Value,
    ) -> Result<Option<Versioned<Value>>> {
        self.inner.claim(c, f, sort, patch).await
    }
    async fn drop_collection(&self, c: &str) -> Result<()> {
        self.inner.drop_collection(c).await
    }
}

fn put_holds(storage: &LedgerStorage, doc: &HoldsDoc) {
    let persist = Persist::open(storage.clone()).unwrap();
    let (version, _) = persist.load_holds().unwrap();
    assert!(persist.store_holds(version, doc).unwrap());
}

fn hold(amount: u64, expires_at_ms: i64) -> super::persist::Hold {
    super::persist::Hold {
        amount,
        expires_at_ms,
    }
}

#[test]
fn a_commit_between_the_hold_read_and_the_record_read_cannot_be_missed() {
    // Writer B holds 80 of the 100-atomic day. A starts a reservation of 50;
    // just before A's read of the hold document, B commits: it appends its
    // 80-atomic record and then drops its hold. Reading the stream before the
    // hold document would see neither and admit 50 (130 > 100).
    let base = LedgerStorage::memory();
    let mut a_doc = HoldsDoc::default();
    a_doc.holds.insert(
        "other:1".into(),
        hold(80, now().timestamp_millis() + HOLD_TTL_MS),
    );
    put_holds(&base, &a_doc);

    let b_storage = base.clone();
    let commit = move || {
        let mut b = PaymentLedger::open(b_storage.clone(), "other", budget()).unwrap();
        b.record_payment(settled(80));
        put_holds(&b_storage, &HoldsDoc::default());
    };
    let storage = LedgerStorage::new(
        Arc::new(InterleavingDocuments {
            inner: Arc::clone(&base.documents),
            before_first_hold_read: Mutex::new(Some(Interleave(Box::new(commit)))),
        }),
        Arc::clone(&base.streams),
    );
    let mut a = PaymentLedger::open(storage, SESSION, budget()).unwrap();

    let verdict = a.reserve_at(now(), 50).unwrap();
    assert!(
        verdict.is_err(),
        "a record committed after the hold read must still be counted: {verdict:?}"
    );
}

#[test]
fn a_held_reservation_survives_past_the_ttl_when_renewed() {
    let storage = LedgerStorage::memory();
    let mut owner = PaymentLedger::open(storage.clone(), SESSION, budget()).unwrap();
    let mut other = PaymentLedger::open(storage, "other", budget()).unwrap();
    owner.reserve_at(now(), 100).unwrap().unwrap();

    // Renewed every 5 minutes, the hold is still there 40 minutes on.
    let mut t = now();
    for _ in 0..8 {
        t += Duration::minutes(5);
        assert_eq!(owner.renew_holds_at(t).unwrap(), 1);
    }
    assert!(
        other.reserve_at(t, 1).unwrap().is_err(),
        "a renewed hold keeps its budget charged past the original TTL"
    );
}

#[test]
fn an_unrenewed_reservation_lapses_after_the_ttl() {
    let storage = LedgerStorage::memory();
    let mut owner = PaymentLedger::open(storage.clone(), SESSION, budget()).unwrap();
    let mut other = PaymentLedger::open(storage, "other", budget()).unwrap();
    owner.reserve_at(now(), 100).unwrap().unwrap();
    let later = now() + Duration::milliseconds(HOLD_TTL_MS + 1);
    assert!(other.reserve_at(later, 100).unwrap().is_ok());
}

#[test]
fn renewing_re_places_a_hold_another_writer_pruned() {
    let storage = LedgerStorage::memory();
    let mut owner = PaymentLedger::open(storage.clone(), SESSION, budget()).unwrap();
    let mut other = PaymentLedger::open(storage, "other", budget()).unwrap();
    owner.reserve_at(now(), 60).unwrap().unwrap();

    // The lease lapsed and another writer's reservation pruned the hold.
    let lapsed = now() + Duration::milliseconds(HOLD_TTL_MS + 1);
    other.reserve_at(lapsed, 10).unwrap().unwrap();

    assert_eq!(owner.renew_holds_at(lapsed).unwrap(), 1);
    assert!(
        other.reserve_at(lapsed, 40).unwrap().is_err(),
        "60 re-held plus other's 10 leaves only 30 of the day"
    );
    assert!(other.reserve_at(lapsed, 30).unwrap().is_ok());
}

#[test]
fn a_released_hold_is_not_renewed() {
    let storage = LedgerStorage::memory();
    let mut owner = PaymentLedger::open(storage, SESSION, budget()).unwrap();
    let id = owner.reserve_at(now(), 60).unwrap().unwrap();
    owner.release(id);
    assert_eq!(owner.renew_holds_at(now()).unwrap(), 0);
    assert_eq!(owner.reserved_atomic(), 0);
}

#[test]
fn renewal_fails_loudly_when_the_hold_document_is_unreadable() {
    let storage = LedgerStorage::memory();
    let mut owner = PaymentLedger::open(storage.clone(), SESSION, budget()).unwrap();
    owner.reserve_at(now(), 10).unwrap().unwrap();
    let documents = Arc::clone(&storage.documents);
    Blocking::new()
        .unwrap()
        .run(async move {
            documents
                .put(
                    BUDGET_COLLECTION,
                    HOLDS_ID,
                    serde_json::json!({"holds": "garbage"}),
                    Precondition::None,
                )
                .await
                .unwrap();
        })
        .unwrap();
    assert!(owner.renew_holds_at(now()).is_err());
}
