//! The ledger's storage: a payments stream and one budget-hold document, both
//! through the `tinystoragedrivers-core` ports.
//!
//! The ports are async and the ledger's API is synchronous (it runs under a
//! std mutex), so every call goes through [`Blocking`], which drives the future
//! on a dedicated runtime thread. Nothing here names a backend: the host hands
//! in the handles and picks sqlite, mongodb or memory.

use std::collections::BTreeMap;
use std::sync::Arc;

use log::debug;
use serde::{Deserialize, Serialize};
use tinystoragedrivers_core::{
    Blocking, CollectionSpec, DocumentStore, ErrorKind, MemoryStorage, Precondition, Scope,
    ScopedStorage, StorageBackend, StreamStore, Version,
};

use super::types::PaymentRecord;

const LOG_PREFIX: &str = "[x402::persist]";

/// Append-only stream of payment records. A payment that changes status is
/// appended again with the same id; readers fold to the latest per id.
pub(super) const PAYMENTS_STREAM: &str = "x402.payments";
/// Collection holding the single budget-hold document.
pub(super) const BUDGET_COLLECTION: &str = "x402_budget";
/// Id of the budget-hold document.
pub(super) const HOLDS_ID: &str = "holds";
/// How long a hold survives without being released or committed. A process that
/// dies between signing and recording must not lock budget forever.
pub(super) const HOLD_TTL_MS: i64 = 15 * 60 * 1000;
/// Records read per stream window while loading.
const WINDOW: usize = 512;

/// The storage handles a ledger persists through.
#[derive(Debug, Clone)]
pub struct LedgerStorage {
    pub(super) documents: Arc<dyn DocumentStore>,
    pub(super) streams: Arc<dyn StreamStore>,
}

impl LedgerStorage {
    /// Persist through the given document and stream ports.
    #[must_use]
    pub fn new(documents: Arc<dyn DocumentStore>, streams: Arc<dyn StreamStore>) -> Self {
        Self { documents, streams }
    }

    /// Persist through a host-provisioned scoped handle.
    #[must_use]
    pub fn from_scoped(storage: &ScopedStorage) -> Self {
        Self::new(storage.documents().clone(), storage.streams().clone())
    }

    /// A private in-memory store. Nothing survives the process: for tests and
    /// hosts that deliberately want an ephemeral ledger.
    ///
    /// # Panics
    ///
    /// Never in practice: the fixed scope name is valid and the memory backend
    /// opens any valid scope.
    #[must_use]
    #[allow(clippy::expect_used, reason = "fixed valid scope on the memory backend")]
    pub fn memory() -> Self {
        let scope = Scope::new("x402").expect("valid scope");
        let scoped = MemoryStorage::new()
            .for_scope(&scope)
            .expect("memory backend opens any scope");
        Self::from_scoped(&scoped)
    }
}

/// One budget hold as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Hold {
    pub amount: u64,
    pub expires_at_ms: i64,
}

/// The budget-hold document: every outstanding hold, across all processes
/// sharing the storage, keyed by owner-qualified id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HoldsDoc {
    #[serde(default)]
    pub holds: BTreeMap<String, Hold>,
}

/// Storage plus the sync bridge.
#[derive(Debug)]
pub(super) struct Persist {
    storage: LedgerStorage,
    bridge: Blocking,
}

impl Persist {
    pub(super) fn open(storage: LedgerStorage) -> Result<Self, String> {
        let bridge = Blocking::new().map_err(|e| format!("start storage bridge: {e}"))?;
        let documents = Arc::clone(&storage.documents);
        bridge
            .run(async move {
                documents
                    .ensure_collection(&CollectionSpec::new(BUDGET_COLLECTION))
                    .await
            })
            .map_err(|e| format!("bridge: {e}"))?
            .map_err(|e| format!("declare x402 budget collection: {e}"))?;
        Ok(Self { storage, bridge })
    }

    /// Records from stream offset `from` to the end, with the next offset.
    /// Entries that no longer parse are skipped with a warning.
    pub(super) fn read_from(&self, from: u64) -> Result<(Vec<PaymentRecord>, u64), String> {
        let mut records = Vec::new();
        let mut next = from;
        loop {
            let streams = Arc::clone(&self.storage.streams);
            let window = self
                .bridge
                .run(async move { streams.read_window(PAYMENTS_STREAM, next, WINDOW).await })
                .map_err(|e| format!("bridge: {e}"))?
                .map_err(|e| format!("read payments stream: {e}"))?;
            let Some(last) = window.last() else { break };
            next = last.offset.saturating_add(1);
            for entry in window {
                match serde_json::from_value::<PaymentRecord>(entry.value) {
                    Ok(record) => records.push(record),
                    Err(error) => log::warn!(
                        "{LOG_PREFIX} corrupt record at offset {}: {error}",
                        entry.offset
                    ),
                }
            }
        }
        Ok((records, next))
    }

    /// Append one record and return its stream offset.
    pub(super) fn append(&self, record: &PaymentRecord) -> Result<u64, String> {
        let value = serde_json::to_value(record)
            .map_err(|error| format!("serialize ledger record: {error}"))?;
        let streams = Arc::clone(&self.storage.streams);
        self.bridge
            .run(async move { streams.append(PAYMENTS_STREAM, value).await })
            .map_err(|e| format!("bridge: {e}"))?
            .map_err(|e| format!("append ledger record: {e}"))
    }

    /// The hold document and the version it was read at (`None` when absent).
    pub(super) fn load_holds(&self) -> Result<(Option<Version>, HoldsDoc), String> {
        let documents = Arc::clone(&self.storage.documents);
        let found = self
            .bridge
            .run(async move { documents.get(BUDGET_COLLECTION, HOLDS_ID).await })
            .map_err(|e| format!("bridge: {e}"))?
            .map_err(|e| format!("read budget holds: {e}"))?;
        match found {
            None => Ok((None, HoldsDoc::default())),
            Some(versioned) => {
                let doc = serde_json::from_value::<HoldsDoc>(versioned.doc)
                    .map_err(|e| format!("budget holds document is unreadable: {e}"))?;
                Ok((Some(versioned.version), doc))
            }
        }
    }

    /// Compare-and-swap the hold document. `Ok(false)` means someone else wrote
    /// it since `version` was read; the caller re-reads and retries.
    pub(super) fn store_holds(
        &self,
        version: Option<Version>,
        doc: &HoldsDoc,
    ) -> Result<bool, String> {
        let body = serde_json::to_value(doc).map_err(|e| format!("serialize holds: {e}"))?;
        let precondition = version.map_or(Precondition::Absent, Precondition::Version);
        let documents = Arc::clone(&self.storage.documents);
        let result = self
            .bridge
            .run(async move {
                documents
                    .put(BUDGET_COLLECTION, HOLDS_ID, body, precondition)
                    .await
            })
            .map_err(|e| format!("bridge: {e}"))?;
        match result {
            Ok(_) => Ok(true),
            Err(error) if matches!(error.kind(), ErrorKind::Conflict | ErrorKind::AlreadyExists) => {
                debug!("{LOG_PREFIX} budget hold CAS lost the race");
                Ok(false)
            }
            Err(error) => Err(format!("write budget holds: {error}")),
        }
    }
}
