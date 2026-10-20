//! The x402 ledger's owner-local storage declarations and migrations.

use serde_json::{Map, Value, json};
use tinystoragedrivers_api::{DocumentStore, StreamStore};
use tinystoragedrivers_types::{
    CollectionSpec, ErrorKind, Precondition, StorageError, WriteOptions,
};

#[path = "schema_payment.rs"]
mod payment;
pub use payment::{LegacyPayment, StoredPayment};
pub(super) use payment::StoredPaymentStatus;

/// The owner prefix for the x402 ledger's storage names.
pub const OWNER: &str = "wallet";
/// Collection containing the single budget-hold document.
pub const BUDGETS_COLLECTION: &str = "wallet.budgets";
/// Schema version for [`BUDGETS_COLLECTION`].
pub const BUDGETS_VERSION: u32 = 2;
/// Identifier of the budget-hold document.
pub const HOLDS_ID: &str = "holds";
/// Append-only payment history stream.
pub const PAYMENTS_STREAM: &str = "wallet.payments";
/// Previous budget collection name migrated on first open.
pub const LEGACY_BUDGET_COLLECTION: &str = "x402_budget";
/// Previous payment stream name migrated on first open.
pub const LEGACY_PAYMENTS_STREAM: &str = "x402.payments";

const MIGRATIONS_COLLECTION: &str = "wallet.migrations";
const MIGRATIONS_ID: &str = "x402";
const BATCH_SIZE: usize = 512;

/// Collections the host should ensure for a module-bound x402 ledger.
///
/// The ledger applies its own migration on open; the host only ensures these
/// pure collection declarations in the scope it has already bound.
#[must_use]
pub fn collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(BUDGETS_COLLECTION), CollectionSpec::new(MIGRATIONS_COLLECTION)]
}

/// Apply x402-owned migrations before the ledger reads or writes its history.
pub(super) async fn migrate(
    documents: &dyn DocumentStore,
    streams: &dyn StreamStore,
) -> tinystoragedrivers_types::Result<()> {
    for spec in collections() {
        documents.ensure_collection(&spec).await?;
    }
    let state = documents.get(MIGRATIONS_COLLECTION, MIGRATIONS_ID).await?;
    let current = state
        .as_ref()
        .and_then(|record| record.doc.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if current >= BUDGETS_VERSION.into() {
        return Ok(());
    }

    migrate_budgets(documents).await?;
    migrate_payments(streams).await?;

    let next = json!({ "version": BUDGETS_VERSION });
    let precondition = state
        .map(|record| Precondition::Version(record.version))
        .unwrap_or(Precondition::Absent);
    match documents
        .put(MIGRATIONS_COLLECTION, MIGRATIONS_ID, next, precondition)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::Conflict => {
            let latest = documents.get(MIGRATIONS_COLLECTION, MIGRATIONS_ID).await?;
            let completed = latest
                .as_ref()
                .and_then(|record| record.doc.get("version"))
                .and_then(Value::as_u64)
                .is_some_and(|version| version >= BUDGETS_VERSION.into());
            if completed {
                Ok(())
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

async fn migrate_budgets(documents: &dyn DocumentStore) -> tinystoragedrivers_types::Result<()> {
    let Some(legacy) = documents
        .get(LEGACY_BUDGET_COLLECTION, HOLDS_ID)
        .await?
    else {
        return Ok(());
    };
    let holds = renamed_holds(&legacy.doc).unwrap_or_default();
    let migrated = json!({ "holds": holds.clone() });
    match documents
        .get(BUDGETS_COLLECTION, HOLDS_ID)
        .await?
    {
        Some(existing) if existing.doc == migrated => Ok(()),
        Some(_) => Err(StorageError::backend(
            "wallet budget migration found conflicting target data",
        )),
        None => {
            let mut options = WriteOptions::new()
                .schema(BUDGETS_VERSION)
                .created_ms(legacy.meta.created_ms);
            if let Some(expires_ms) = legacy.meta.expires_ms {
                options = options.expires_ms(expires_ms);
            }
            match documents
                .put_with(
                    BUDGETS_COLLECTION,
                    HOLDS_ID,
                    migrated,
                    Precondition::Absent,
                    options,
                )
                .await
            {
                Ok(_) => Ok(()),
                Err(error) if error.kind() == ErrorKind::Conflict => {
                    let latest = documents.get(BUDGETS_COLLECTION, HOLDS_ID).await?;
                    if latest.is_some_and(|record| record.doc == json!({ "holds": holds })) {
                        Ok(())
                    } else {
                        Err(error)
                    }
                }
                Err(error) => Err(error),
            }
        }
    }
}

impl From<super::types::PaymentStatus> for StoredPaymentStatus {
    fn from(status: super::types::PaymentStatus) -> Self {
        match status {
            super::types::PaymentStatus::Pending => Self::Pending,
            super::types::PaymentStatus::Settled => Self::Settled,
            super::types::PaymentStatus::Failed => Self::Failed,
            super::types::PaymentStatus::Denied => Self::Denied,
        }
    }
}

impl From<StoredPaymentStatus> for super::types::PaymentStatus {
    fn from(status: StoredPaymentStatus) -> Self {
        match status {
            StoredPaymentStatus::Pending => Self::Pending,
            StoredPaymentStatus::Settled => Self::Settled,
            StoredPaymentStatus::Failed => Self::Failed,
            StoredPaymentStatus::Denied => Self::Denied,
        }
    }
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;

async fn migrate_payments(streams: &dyn StreamStore) -> tinystoragedrivers_types::Result<()> {
    let source_len = streams.len(LEGACY_PAYMENTS_STREAM).await?;
    let mut source = Vec::new();
    let mut offset = 0;
    while offset < source_len {
        let window = streams
            .read_window(LEGACY_PAYMENTS_STREAM, offset, BATCH_SIZE)
            .await?;
        if window.is_empty() {
            break;
        }
        offset = window.last().map_or(offset, |entry| entry.offset.saturating_add(1));
        source.extend(window.into_iter().map(|entry| entry.value));
    }
    let migrated = migrate_payments_stream(LEGACY_PAYMENTS_STREAM, source)?
        .map_or_else(Vec::new, |(_, entries)| entries);
    if migrated.is_empty() {
        return Ok(());
    }

    let target_len = streams.len(PAYMENTS_STREAM).await?;
    if target_len > migrated.len() as u64 {
        return Err(StorageError::backend(
            "wallet payment migration found unexpected target entries",
        ));
    }
    let existing = streams
        .read_window(PAYMENTS_STREAM, 0, target_len as usize)
        .await?;
    if existing
        .iter()
        .zip(migrated.iter())
        .any(|(entry, value)| entry.value != *value)
    {
        return Err(StorageError::backend(
            "wallet payment migration found conflicting target data",
        ));
    }
    let start = target_len as usize;
    if start < migrated.len() {
        streams
            .append_batch_if_offset(PAYMENTS_STREAM, target_len, migrated[start..].to_vec())
            .await?;
    }
    Ok(())
}

fn migrate_payments_stream(
    source: &str,
    entries: Vec<Value>,
) -> tinystoragedrivers_types::Result<Option<(String, Vec<Value>)>> {
    if source != LEGACY_PAYMENTS_STREAM {
        return Ok(None);
    }
    let mut migrated = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Ok(legacy) = serde_json::from_value::<LegacyPayment>(entry) {
            migrated.push(
                serde_json::to_value(StoredPayment::from(legacy))
                    .map_err(|error| StorageError::invalid_input(error.to_string()))?,
            );
        }
    }
    Ok(Some((PAYMENTS_STREAM.to_owned(), migrated)))
}

fn renamed_holds(body: &Value) -> Option<Map<String, Value>> {
    let holds = match body.get("holds") {
        None => return Some(Map::new()),
        Some(holds) => holds.as_object()?,
    };
    holds
        .iter()
        .map(|(key, hold)| {
            let amount = hold.get("amount")?.as_u64()?;
            let expires_ms = hold.get("expires_at_ms")?.as_i64()?;
            Some((key.clone(), json!({ "amount": amount, "expires_ms": expires_ms })))
        })
        .collect()
}
