//! The wallet's own vocabulary: which chain, which account, and what status a
//! host reports about them.

use serde::{Deserialize, Serialize};

pub use tinywallet_bus::wire::WalletChain;

/// How a wallet came to exist.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WalletSetupSource {
    /// A fresh recovery phrase generated for the user.
    Generated,
    /// A recovery phrase the user brought.
    Imported,
}

/// A derived per-chain account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletAccount {
    /// The chain family the account belongs to.
    pub chain: WalletChain,
    /// The address in the chain's canonical text form.
    pub address: String,
    /// The derivation path the account was derived at.
    pub derivation_path: String,
}

/// What a host reports about the wallet: whether it is set up, and its
/// accounts.
// The four flags are the wire shape hosts already serialize to the frontend;
// folding them into an enum would change the JSON.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletStatus {
    /// Whether a wallet exists.
    pub configured: bool,
    /// Whether onboarding finished.
    pub onboarding_completed: bool,
    /// Whether the user consented to local wallet custody.
    pub consent_granted: bool,
    /// Whether the encrypted secret material is stored.
    pub secret_stored: bool,
    /// How the wallet was created, when configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<WalletSetupSource>,
    /// Recovery phrase length, when configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mnemonic_word_count: Option<u8>,
    /// The derived accounts, one per chain.
    pub accounts: Vec<WalletAccount>,
    /// When the wallet state was last written, in milliseconds since the epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
}
