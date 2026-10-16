//! The ledger's storage: the `wallet.payments` stream and one `wallet.budgets` hold document, both
//! through the `tinystoragedrivers-core` ports.
//!
//! The ports are async and the ledger's API is synchronous (it runs under a
//! std mutex), so every call goes through [`Blocking`], which drives the future
//! on a dedicated runtime thread. Nothing here names a backend: the host hands
//! in the handles and picks sqlite, mongodb or memory.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use log::debug;
use serde::{Deserialize, Serialize};
use tinystoragedrivers_core::{
    Blocking, DocumentStore, ErrorKind, MemoryStorage, Precondition, Scope, ScopedStorage,
    StorageBackend, StreamStore, Version, WriteOptions,
};

use super::schema::{BUDGETS_COLLECTION, BUDGETS_VERSION, HOLDS_ID, OWNER, PAYMENTS_STREAM};
use super::types::{PaymentRecord, PaymentStatus};

const LOG_PREFIX: &str = "[x402::persist]";

/// How long a hold survives without being released or committed. A process that
/// dies between signing and recording must not lock budget forever.
pub(super) const HOLD_TTL_MS: i64 = 15 * 60 * 1000;
/// How often an owner of long-lived holds should call
/// [`PaymentLedger::renew_holds`](super::PaymentLedger::renew_holds): a third of
/// the TTL, so two missed beats still leave the lease alive.
pub const HOLD_RENEW_EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
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
    #[allow(
        clippy::expect_used,
        reason = "fixed valid scope on the memory backend"
    )]
    pub fn memory() -> Self {
        let scope = Scope::local().module(OWNER).expect("valid scope");
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
    pub expires_ms: i64,
}

/// The budget-hold document: every outstanding hold, across all processes
/// sharing the storage, keyed by owner-qualified id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HoldsDoc {
    #[serde(default)]
    pub holds: BTreeMap<String, Hold>,
}

/// One payment as stored in `wallet.payments`: `snake_case`, every instant an
/// epoch-millisecond `*_ms` field, no wrapper. This is a storage DTO; the
/// public [`PaymentRecord`] wire type is unchanged.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StoredPayment {
    pub id: String,
    pub url: String,
    pub asset: String,
    pub amount_atomic: u64,
    pub amount_display: String,
    pub recipient: String,
    pub network: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
    pub status: PaymentStatus,
    pub timestamp_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_valid_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solana_last_valid_block_height: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_expired_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_expiry_observed_height: Option<u64>,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_hold: Option<String>,
}

fn from_ms(field: &str, ms: i64) -> Result<DateTime<Utc>, String> {
    DateTime::from_timestamp_millis(ms).ok_or_else(|| format!("{field} {ms} is out of range"))
}

impl StoredPayment {
    pub(super) fn new(record: &PaymentRecord, committed_hold: Option<&str>) -> Self {
        Self {
            id: record.id.clone(),
            url: record.url.clone(),
            asset: record.asset.clone(),
            amount_atomic: record.amount_atomic,
            amount_display: record.amount_display.clone(),
            recipient: record.recipient.clone(),
            network: record.network.clone(),
            tx_signature: record.tx_signature.clone(),
            status: record.status,
            timestamp_ms: record.timestamp.timestamp_millis(),
            authorization_valid_until_ms: record
                .authorization_valid_until
                .map(|at| at.timestamp_millis()),
            solana_last_valid_block_height: record.solana_last_valid_block_height,
            authorization_expired_at_ms: record
                .authorization_expired_at
                .map(|at| at.timestamp_millis()),
            authorization_expiry_observed_height: record.authorization_expiry_observed_height,
            session_id: record.session_id.clone(),
            thread_id: record.thread_id.clone(),
            committed_hold: committed_hold.map(str::to_owned),
        }
    }

    /// The public record and the hold this entry committed, if any.
    pub(super) fn into_record(self) -> Result<(PaymentRecord, Option<String>), String> {
        let record = PaymentRecord {
            id: self.id,
            url: self.url,
            asset: self.asset,
            amount_atomic: self.amount_atomic,
            amount_display: self.amount_display,
            recipient: self.recipient,
            network: self.network,
            tx_signature: self.tx_signature,
            status: self.status,
            timestamp: from_ms("timestamp_ms", self.timestamp_ms)?,
            authorization_valid_until: self
                .authorization_valid_until_ms
                .map(|ms| from_ms("authorization_valid_until_ms", ms))
                .transpose()?,
            solana_last_valid_block_height: self.solana_last_valid_block_height,
            authorization_expired_at: self
                .authorization_expired_at_ms
                .map(|ms| from_ms("authorization_expired_at_ms", ms))
                .transpose()?,
            authorization_expiry_observed_height: self.authorization_expiry_observed_height,
            session_id: self.session_id,
            thread_id: self.thread_id,
        };
        Ok((record, self.committed_hold))
    }
}

/// A payment read back from the stream, with the hold its entry committed.
#[derive(Debug)]
pub(super) struct LoadedPayment {
    pub record: PaymentRecord,
    pub committed_hold: Option<String>,
}

/// Storage plus the sync bridge.
#[derive(Debug)]
pub(super) struct Persist {
    storage: LedgerStorage,
    bridge: Blocking,
}

impl Persist {
    /// Run one port call on the bridge, folding both layers of failure into a
    /// message that names the operation.
    fn run<T, F>(&self, what: &str, future: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: Future<Output = tinystoragedrivers_core::Result<T>> + Send + 'static,
    {
        match self.bridge.run(future) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) | Err(error) => Err(format!("{what}: {error}")),
        }
    }

    pub(super) fn open(storage: LedgerStorage) -> Result<Self, String> {
        let bridge = Blocking::new().map_err(|e| format!("start storage bridge: {e}"))?;
        Ok(Self { storage, bridge })
    }

    /// Records from stream offset `from` to the end, with the next offset.
    /// Entries that no longer parse are skipped with a warning.
    pub(super) fn read_from(&self, from: u64) -> Result<(Vec<LoadedPayment>, u64), String> {
        let mut records = Vec::new();
        let mut next = from;
        loop {
            let streams = Arc::clone(&self.storage.streams);
            let window = self.run("read payments stream", async move {
                streams.read_window(PAYMENTS_STREAM, next, WINDOW).await
            })?;
            let Some(last) = window.last() else { break };
            next = last.offset.saturating_add(1);
            for entry in window {
                let parsed = serde_json::from_value::<StoredPayment>(entry.value)
                    .map_err(|error| error.to_string())
                    .and_then(StoredPayment::into_record);
                match parsed {
                    Ok((record, committed_hold)) => records.push(LoadedPayment {
                        record,
                        committed_hold,
                    }),
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
    pub(super) fn append(
        &self,
        record: &PaymentRecord,
        committed_hold: Option<&str>,
    ) -> Result<u64, String> {
        let value = serde_json::to_value(StoredPayment::new(record, committed_hold))
            .map_err(|error| error.to_string())?;
        let streams = Arc::clone(&self.storage.streams);
        self.run("append ledger record", async move {
            streams.append(PAYMENTS_STREAM, value).await
        })
    }

    /// The hold document and the version it was read at (`None` when absent).
    pub(super) fn load_holds(&self) -> Result<(Option<Version>, HoldsDoc), String> {
        let documents = Arc::clone(&self.storage.documents);
        let found = self.run("read budget holds", async move {
            documents.get(BUDGETS_COLLECTION, HOLDS_ID).await
        })?;
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
        let body = serde_json::to_value(doc).map_err(|error| error.to_string())?;
        let precondition = version.map_or(Precondition::Absent, Precondition::Version);
        let documents = Arc::clone(&self.storage.documents);
        let result = self.bridge.run(async move {
            documents
                .put_with(
                    BUDGETS_COLLECTION,
                    HOLDS_ID,
                    body,
                    precondition,
                    WriteOptions::new().schema(BUDGETS_VERSION),
                )
                .await
        });
        match result.map_err(|error| format!("write budget holds: {error}"))? {
            Ok(_) => Ok(true),
            Err(error)
                if matches!(error.kind(), ErrorKind::Conflict | ErrorKind::AlreadyExists) =>
            {
                debug!("{LOG_PREFIX} budget hold CAS lost the race");
                Ok(false)
            }
            Err(error) => Err(format!("write budget holds: {error}")),
        }
    }
}
