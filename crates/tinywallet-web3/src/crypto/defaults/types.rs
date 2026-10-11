//! Compatibility adapters from bus identifiers to crypto-owned reference data.

pub use tinywallet_bus::wire::{
    EvmNetwork, RpcSource, SolanaCluster, WalletAssetDefinition, WalletNetworkDefaults,
};

use tinywallet_crypto::asset::{
    EvmNetwork as CryptoEvmNetwork, SolanaCluster as CryptoSolanaCluster,
};

/// Reference data associated with an EVM network identifier.
///
/// Import this trait alongside [`EvmNetwork`] to retain method-style access to
/// the defaults that were previously inherent methods on the bus enum.
pub trait EvmNetworkReferenceData {
    /// The built-in public JSON-RPC endpoint for this network.
    fn default_rpc_url(self) -> &'static str;

    /// The block explorer's transaction URL prefix.
    fn explorer_tx_base(self) -> &'static str;

    /// The kebab-case label used in summaries.
    fn network_label(self) -> &'static str;
}

impl EvmNetworkReferenceData for EvmNetwork {
    fn default_rpc_url(self) -> &'static str {
        default_rpc_url_for_evm(self)
    }

    fn explorer_tx_base(self) -> &'static str {
        explorer_tx_base_for_evm(self)
    }

    fn network_label(self) -> &'static str {
        network_label_for_evm(self)
    }
}

/// Reference data associated with a Solana cluster identifier.
///
/// Import this trait alongside [`SolanaCluster`] to retain method-style access
/// to the defaults that were previously inherent methods on the bus enum.
pub trait SolanaClusterReferenceData {
    /// Default JSON-RPC endpoint for this cluster.
    fn rpc_url(self) -> &'static str;

    /// The kebab-case label used in summaries.
    fn network_label(self) -> &'static str;

    /// Explorer query suffix for this cluster.
    fn explorer_tx_suffix(self) -> &'static str;

    /// USDC SPL-token mint address for this cluster.
    fn usdc_mint(self) -> &'static str;
}

impl SolanaClusterReferenceData for SolanaCluster {
    fn rpc_url(self) -> &'static str {
        default_rpc_url_for_solana(self)
    }

    fn network_label(self) -> &'static str {
        network_label_for_solana(self)
    }

    fn explorer_tx_suffix(self) -> &'static str {
        explorer_tx_suffix_for_solana(self)
    }

    fn usdc_mint(self) -> &'static str {
        usdc_mint_for_solana(self)
    }
}

const fn crypto_evm_network(network: EvmNetwork) -> CryptoEvmNetwork {
    match network {
        EvmNetwork::EthereumMainnet => CryptoEvmNetwork::Ethereum,
        EvmNetwork::BaseMainnet => CryptoEvmNetwork::Base,
        EvmNetwork::ArbitrumOne => CryptoEvmNetwork::Arbitrum,
        EvmNetwork::OptimismMainnet => CryptoEvmNetwork::Optimism,
        EvmNetwork::PolygonMainnet => CryptoEvmNetwork::Polygon,
        EvmNetwork::BscMainnet => CryptoEvmNetwork::Bsc,
    }
}

const fn crypto_solana_cluster(cluster: SolanaCluster) -> CryptoSolanaCluster {
    match cluster {
        SolanaCluster::Mainnet => CryptoSolanaCluster::Mainnet,
        SolanaCluster::Devnet => CryptoSolanaCluster::Devnet,
    }
}

/// The built-in public JSON-RPC endpoint for an EVM network.
#[must_use]
pub const fn default_rpc_url_for_evm(network: EvmNetwork) -> &'static str {
    crypto_evm_network(network).default_rpc_url()
}

/// The explorer's transaction URL prefix for an EVM network.
#[must_use]
pub const fn explorer_tx_base_for_evm(network: EvmNetwork) -> &'static str {
    crypto_evm_network(network).explorer_tx_base()
}

/// Human-readable kebab-case label for an EVM network.
#[must_use]
pub const fn network_label_for_evm(network: EvmNetwork) -> &'static str {
    crypto_evm_network(network).network_label()
}

/// Default JSON-RPC endpoint for a Solana cluster.
#[must_use]
pub const fn default_rpc_url_for_solana(cluster: SolanaCluster) -> &'static str {
    crypto_solana_cluster(cluster).rpc_url()
}

/// Human-readable kebab-case label for a Solana cluster.
#[must_use]
pub const fn network_label_for_solana(cluster: SolanaCluster) -> &'static str {
    crypto_solana_cluster(cluster).network_label()
}

/// Explorer query suffix for a Solana cluster.
#[must_use]
pub const fn explorer_tx_suffix_for_solana(cluster: SolanaCluster) -> &'static str {
    crypto_solana_cluster(cluster).explorer_tx_suffix()
}

/// USDC SPL-token mint for a Solana cluster.
#[must_use]
pub const fn usdc_mint_for_solana(cluster: SolanaCluster) -> &'static str {
    crypto_solana_cluster(cluster).usdc_mint()
}

/// Explorer transaction URL prefix for a Solana cluster.
#[must_use]
pub const fn explorer_tx_base_for_solana(cluster: SolanaCluster) -> &'static str {
    crypto_solana_cluster(cluster).explorer_tx_base()
}
