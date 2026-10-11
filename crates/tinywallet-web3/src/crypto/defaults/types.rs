//! Implementation-owned network defaults and compatibility extension traits.

pub use tinywallet_bus::wire::{
    EvmNetwork, RpcSource, SolanaCluster, WalletAssetDefinition, WalletNetworkDefaults,
};

const SOLSCAN_TX_BASE: &str = "https://solscan.io/tx/";

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

/// The built-in public JSON-RPC endpoint for an EVM network.
#[must_use]
pub const fn default_rpc_url_for_evm(network: EvmNetwork) -> &'static str {
    match network {
        EvmNetwork::EthereumMainnet => "https://ethereum-rpc.publicnode.com",
        EvmNetwork::BaseMainnet => "https://mainnet.base.org",
        EvmNetwork::ArbitrumOne => "https://arb1.arbitrum.io/rpc",
        EvmNetwork::OptimismMainnet => "https://mainnet.optimism.io",
        EvmNetwork::PolygonMainnet => "https://polygon-rpc.com",
        EvmNetwork::BscMainnet => "https://bsc-dataseed.binance.org",
    }
}

/// The explorer's transaction URL prefix for an EVM network.
#[must_use]
pub const fn explorer_tx_base_for_evm(network: EvmNetwork) -> &'static str {
    match network {
        EvmNetwork::EthereumMainnet => "https://etherscan.io/tx/",
        EvmNetwork::BaseMainnet => "https://basescan.org/tx/",
        EvmNetwork::ArbitrumOne => "https://arbiscan.io/tx/",
        EvmNetwork::OptimismMainnet => "https://optimistic.etherscan.io/tx/",
        EvmNetwork::PolygonMainnet => "https://polygonscan.com/tx/",
        EvmNetwork::BscMainnet => "https://bscscan.com/tx/",
    }
}

/// Human-readable kebab-case label for an EVM network.
#[must_use]
pub const fn network_label_for_evm(network: EvmNetwork) -> &'static str {
    match network {
        EvmNetwork::EthereumMainnet => "ethereum-mainnet",
        EvmNetwork::BaseMainnet => "base-mainnet",
        EvmNetwork::ArbitrumOne => "arbitrum-one",
        EvmNetwork::OptimismMainnet => "optimism-mainnet",
        EvmNetwork::PolygonMainnet => "polygon-mainnet",
        EvmNetwork::BscMainnet => "bsc-mainnet",
    }
}

/// Default JSON-RPC endpoint for a Solana cluster.
#[must_use]
pub const fn default_rpc_url_for_solana(cluster: SolanaCluster) -> &'static str {
    match cluster {
        SolanaCluster::Mainnet => "https://api.mainnet-beta.solana.com",
        SolanaCluster::Devnet => "https://api.devnet.solana.com",
    }
}

/// Human-readable kebab-case label for a Solana cluster.
#[must_use]
pub const fn network_label_for_solana(cluster: SolanaCluster) -> &'static str {
    match cluster {
        SolanaCluster::Mainnet => "solana-mainnet-beta",
        SolanaCluster::Devnet => "solana-devnet",
    }
}

/// Explorer query suffix for a Solana cluster.
#[must_use]
pub const fn explorer_tx_suffix_for_solana(cluster: SolanaCluster) -> &'static str {
    match cluster {
        SolanaCluster::Mainnet => "",
        SolanaCluster::Devnet => "?cluster=devnet",
    }
}

/// USDC SPL-token mint for a Solana cluster.
#[must_use]
pub const fn usdc_mint_for_solana(cluster: SolanaCluster) -> &'static str {
    match cluster {
        SolanaCluster::Mainnet => "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        SolanaCluster::Devnet => "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
    }
}

/// Explorer transaction URL prefix for a Solana cluster.
#[must_use]
pub const fn explorer_tx_base_for_solana(_cluster: SolanaCluster) -> &'static str {
    SOLSCAN_TX_BASE
}
