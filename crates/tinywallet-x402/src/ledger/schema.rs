//! The ledger's storage schema (owner `wallet`) and the v1 to v2 steps that
//! bring the `x402` records forward.
//!
//! | v1 | v2 |
//! |---|---|
//! | collection `x402_budget` | collection `wallet.budgets` (hold `expires_at_ms` becomes `expires_ms`) |
//! | stream `x402.payments` | stream `wallet.payments` (`snake_case` entries, every instant `*_ms`) |
//!
//! The host composes [`storage_schema`] into its registry, and a loadable
//! module's storage grant is generated from the same declaration. The steps are
//! pure: a re-run over the same input produces the same output.

use serde::Deserialize;
use serde_json::{Map, Value, json};
use tinystoragedrivers_core::{
    CollectionSpec, Owner, Schema, ScopeClass, SourceRecord, Step, StorageError, TargetRecord,
};

use super::persist::StoredPayment;
use super::types::PaymentRecord;

/// The schema owner: every name below starts with `wallet.`.
pub(super) const OWNER: &str = "wallet";
/// Collection holding the single budget-hold document.
pub(super) const BUDGETS_COLLECTION: &str = "wallet.budgets";
/// Body version of [`BUDGETS_COLLECTION`].
pub(super) const BUDGETS_VERSION: u32 = 2;
/// Id of the budget-hold document.
pub(super) const HOLDS_ID: &str = "holds";
/// Append-only stream of payment records. A payment that changes status is
/// appended again with the same id; readers fold to the latest per id.
pub(super) const PAYMENTS_STREAM: &str = "wallet.payments";

/// v1 name of [`BUDGETS_COLLECTION`].
const LEGACY_BUDGET_COLLECTION: &str = "x402_budget";
/// v1 name of [`PAYMENTS_STREAM`].
const LEGACY_PAYMENTS_STREAM: &str = "x402.payments";

/// What the wallet module stores: the budget-hold collection and the payments
/// stream, both in the module's own scope, with the steps that migrate the v1
/// `x402_budget` collection and `x402.payments` stream into them.
///
/// The payments stream is a spending history and is never sealed or deleted.
///
/// # Panics
///
/// Never in practice: `wallet` is a valid owner name.
#[must_use]
#[allow(clippy::expect_used, reason = "`wallet` is a fixed valid owner name")]
pub fn storage_schema() -> Schema {
    Schema::new(Owner::new(OWNER).expect("`wallet` is a valid owner"))
        .collection_with(
            BUDGETS_VERSION,
            CollectionSpec::new(BUDGETS_COLLECTION),
            ScopeClass::Module,
        )
        .stream(PAYMENTS_STREAM, ScopeClass::Module)
        .step(
            Step::new(
                "budgets_v2",
                BUDGETS_COLLECTION,
                1,
                BUDGETS_VERSION,
                |batch| Ok(migrate_budgets(batch)),
            )
            .source(LEGACY_BUDGET_COLLECTION),
        )
        .step(
            Step::stream("payments_v2", PAYMENTS_STREAM, 1, 2, |source, entries| {
                migrate_payments_stream(&source, entries)
            })
            .source_prefix(LEGACY_PAYMENTS_STREAM),
        )
}

/// A v1 payment entry: the camelCase wire record, flattened, with the hold
/// metadata beside it.
#[derive(Debug, Deserialize)]
struct LegacyPayment {
    #[serde(flatten)]
    record: PaymentRecord,
    #[serde(default)]
    committed_hold: Option<String>,
}

/// `x402.payments` entries to `wallet.payments` entries. An entry that no
/// longer parses is dropped: the v1 reader skipped it with a warning too, so it
/// never counted toward a budget.
fn migrate_payments_stream(
    source: &str,
    entries: Vec<Value>,
) -> tinystoragedrivers_core::Result<Option<(String, Vec<Value>)>> {
    if source != LEGACY_PAYMENTS_STREAM {
        return Ok(None);
    }
    let mut migrated = Vec::with_capacity(entries.len());
    for entry in entries {
        match serde_json::from_value::<LegacyPayment>(entry) {
            Ok(legacy) => {
                let stored = StoredPayment::new(&legacy.record, legacy.committed_hold.as_deref());
                migrated.push(
                    serde_json::to_value(stored)
                        .map_err(|error| StorageError::invalid_input(error.to_string()))?,
                );
            }
            Err(error) => log::warn!(
                "[x402::schema] dropping unreadable v1 payment entry from {source}: {error}"
            ),
        }
    }
    Ok(Some((PAYMENTS_STREAM.to_owned(), migrated)))
}

/// The v1 hold document, renamed field by field. Holds are leases that expire
/// in minutes, so a document that is not a hold map migrates to an empty one
/// rather than wedging the host's migration; the budget the holds guarded is
/// recomputed from the payments stream.
fn migrate_budgets(batch: Vec<SourceRecord>) -> Vec<TargetRecord> {
    batch
        .into_iter()
        .map(|record| {
            let holds = renamed_holds(&record.body).unwrap_or_else(|| {
                log::warn!(
                    "[x402::schema] v1 budget document `{}` is unreadable; migrating it empty",
                    record.id
                );
                Map::new()
            });
            TargetRecord::new(BUDGETS_COLLECTION, record.id, json!({ "holds": holds }))
                .created_ms(record.meta.created_ms)
                .expires_ms(record.meta.expires_ms)
        })
        .collect()
}

/// `{holds: {key: {amount, expires_at_ms}}}` as `{key: {amount, expires_ms}}`.
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
            Some((
                key.clone(),
                json!({ "amount": amount, "expires_ms": expires_ms }),
            ))
        })
        .collect()
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
