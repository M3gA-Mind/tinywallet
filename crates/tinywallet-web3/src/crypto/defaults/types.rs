//! Implementation-owned static data for shared wallet network identifiers.

pub use tinywallet_bus::wire::{
    EvmNetwork, RpcSource, SolanaCluster, WalletAssetDefinition, WalletNetworkDefaults,
};

const ETHERSCAN_TX_BASE: &str = "https://etherscan.io/tx/";
const BASESCAN_TX_BASE: &str = "https://basescan.org/tx/";
const ARBISCAN_TX_BASE: &str = "https://arbiscan.io/tx/";
const OPTIMISTIC_TX_BASE: &str = "https://optimistic.etherscan.io/tx/";
const POLYGONSCAN_TX_BASE: &str = "https://polygonscan.com/tx/";
const BSCSCAN_TX_BASE: &str = "https://bscscan.com/tx/";
const SOLSCAN_DEVNET_SUFFIX: &str = "?cluster=devnet";
const SOLSCAN_TX_BASE: &str = "https://solscan.io/tx/";
const DEFAULT_SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const DEVNET_SOLANA_RPC_URL: &str = "https://api.devnet.solana.com";
const SOLANA_USDC_MINT_MAINNET: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const SOLANA_USDC_MINT_DEVNET: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";

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
        EvmNetwork::EthereumMainnet => ETHERSCAN_TX_BASE,
        EvmNetwork::BaseMainnet => BASESCAN_TX_BASE,
        EvmNetwork::ArbitrumOne => ARBISCAN_TX_BASE,
        EvmNetwork::OptimismMainnet => OPTIMISTIC_TX_BASE,
        EvmNetwork::PolygonMainnet => POLYGONSCAN_TX_BASE,
        EvmNetwork::BscMainnet => BSCSCAN_TX_BASE,
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
        SolanaCluster::Mainnet => DEFAULT_SOLANA_RPC_URL,
        SolanaCluster::Devnet => DEVNET_SOLANA_RPC_URL,
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
        SolanaCluster::Devnet => SOLSCAN_DEVNET_SUFFIX,
    }
}

/// USDC SPL-token mint for a Solana cluster.
#[must_use]
pub const fn usdc_mint_for_solana(cluster: SolanaCluster) -> &'static str {
    match cluster {
        SolanaCluster::Mainnet => SOLANA_USDC_MINT_MAINNET,
        SolanaCluster::Devnet => SOLANA_USDC_MINT_DEVNET,
    }
}

/// Explorer transaction URL prefix for a Solana cluster.
#[must_use]
pub const fn explorer_tx_base_for_solana(_cluster: SolanaCluster) -> &'static str {
    SOLSCAN_TX_BASE
}
