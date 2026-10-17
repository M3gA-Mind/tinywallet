//! The payment record as it is stored, and the v1 shape it migrates from.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Where a stored payment stands. The same four states, spelled the same way,
/// as the implementation's public status type, which converts to and from this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredPaymentStatus {
    /// Signed and sent, not yet settled.
    Pending,
    /// A trusted settlement receipt confirms payment.
    Settled,
    /// A trusted result established that the payment did not settle.
    Failed,
    /// Refused before anything was signed.
    Denied,
}

/// One payment as stored in `wallet.payments`: `snake_case`, every instant an
/// epoch-millisecond `*_ms` field, no wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPayment {
    /// Unique id of the attempt.
    pub id: String,
    /// The URL the payment bought access to.
    pub url: String,
    /// The asset paid in.
    pub asset: String,
    /// The amount in atomic units of `asset`.
    pub amount_atomic: u64,
    /// The amount formatted for people.
    pub amount_display: String,
    /// Who was paid.
    pub recipient: String,
    /// The network, in CAIP-2 form.
    pub network: String,
    /// The settlement transaction, once reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
    /// Where the payment stands.
    pub status: StoredPaymentStatus,
    /// When this state was written, in epoch milliseconds.
    pub timestamp_ms: i64,
    /// When an exported authorization becomes unusable, in epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_valid_until_ms: Option<i64>,
    /// Solana's `lastValidBlockHeight` for the exported transaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solana_last_valid_block_height: Option<u64>,
    /// When the authorization was observed expired, in epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_expired_at_ms: Option<i64>,
    /// The block height at which the expiry was observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_expiry_observed_height: Option<u64>,
    /// The session that made the payment.
    pub session_id: String,
    /// The conversation thread, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// The budget hold this entry committed, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_hold: Option<String>,
}

/// A v1 `x402.payments` entry: the camelCase wire record with RFC 3339
/// instants, and the hold metadata beside it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyPayment {
    id: String,
    url: String,
    asset: String,
    amount_atomic: u64,
    amount_display: String,
    recipient: String,
    network: String,
    #[serde(default)]
    tx_signature: Option<String>,
    status: StoredPaymentStatus,
    timestamp: DateTime<Utc>,
    #[serde(default)]
    authorization_valid_until: Option<DateTime<Utc>>,
    #[serde(default)]
    solana_last_valid_block_height: Option<u64>,
    #[serde(default)]
    authorization_expired_at: Option<DateTime<Utc>>,
    #[serde(default)]
    authorization_expiry_observed_height: Option<u64>,
    session_id: String,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default, rename = "committed_hold")]
    committed_hold: Option<String>,
}

impl From<LegacyPayment> for StoredPayment {
    fn from(legacy: LegacyPayment) -> Self {
        Self {
            id: legacy.id,
            url: legacy.url,
            asset: legacy.asset,
            amount_atomic: legacy.amount_atomic,
            amount_display: legacy.amount_display,
            recipient: legacy.recipient,
            network: legacy.network,
            tx_signature: legacy.tx_signature,
            status: legacy.status,
            timestamp_ms: legacy.timestamp.timestamp_millis(),
            authorization_valid_until_ms: legacy
                .authorization_valid_until
                .map(|at| at.timestamp_millis()),
            solana_last_valid_block_height: legacy.solana_last_valid_block_height,
            authorization_expired_at_ms: legacy
                .authorization_expired_at
                .map(|at| at.timestamp_millis()),
            authorization_expiry_observed_height: legacy.authorization_expiry_observed_height,
            session_id: legacy.session_id,
            thread_id: legacy.thread_id,
            committed_hold: legacy.committed_hold,
        }
    }
}
