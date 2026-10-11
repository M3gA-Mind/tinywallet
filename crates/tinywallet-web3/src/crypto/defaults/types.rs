//! Implementation-owned static data for shared wallet network identifiers.

pub use tinywallet_bus::wire::{
    EvmNetwork, RpcSource, SolanaCluster, WalletAssetDefinition, WalletNetworkDefaults,
};

const SOLSCAN_TX_BASE: &str = "https://solscan.io/tx/";

/// The built-in public JSON-RPC endpoint for an EVM network.
#[must_use]
pub const fn default_rpc_url_for_evm(network: EvmNetwork) -> &'static str {
    network.default_rpc_url()
}

/// The explorer's transaction URL prefix for an EVM network.
#[must_use]
pub const fn explorer_tx_base_for_evm(network: EvmNetwork) -> &'static str {
    network.explorer_tx_base()
}

/// Human-readable kebab-case label for an EVM network.
#[must_use]
pub const fn network_label_for_evm(network: EvmNetwork) -> &'static str {
    network.network_label()
}

/// Default JSON-RPC endpoint for a Solana cluster.
#[must_use]
pub const fn default_rpc_url_for_solana(cluster: SolanaCluster) -> &'static str {
    cluster.rpc_url()
}

/// Human-readable kebab-case label for a Solana cluster.
#[must_use]
pub const fn network_label_for_solana(cluster: SolanaCluster) -> &'static str {
    cluster.network_label()
}

/// Explorer query suffix for a Solana cluster.
#[must_use]
pub const fn explorer_tx_suffix_for_solana(cluster: SolanaCluster) -> &'static str {
    cluster.explorer_tx_suffix()
}

/// USDC SPL-token mint for a Solana cluster.
#[must_use]
pub const fn usdc_mint_for_solana(cluster: SolanaCluster) -> &'static str {
    cluster.usdc_mint()
}

/// Explorer transaction URL prefix for a Solana cluster.
#[must_use]
pub const fn explorer_tx_base_for_solana(_cluster: SolanaCluster) -> &'static str {
    SOLSCAN_TX_BASE
}
