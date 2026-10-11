//! Static reference data: networks, clusters, asset catalogs and explorer
//! links.
//!
//! Pure data and pure functions. What a deployment can change (the endpoint a
//! chain talks to, and which Solana cluster) is asked of
//! [`RpcEndpoints`](crate::crypto::seams::RpcEndpoints); the environment
//! variables that drive it stay in the host.

mod catalog;
mod types;

pub use catalog::{
    asset_catalog, default_rpc_url, evm_asset_catalog, explorer_tx_url,
    explorer_tx_url_for_evm_network, find_asset, find_asset_for_network, network_defaults,
};
pub use types::{
    EvmNetwork, RpcSource, SolanaCluster, WalletAssetDefinition, WalletNetworkDefaults,
};
pub use types::{
    default_rpc_url_for_evm, default_rpc_url_for_solana, explorer_tx_base_for_evm,
    explorer_tx_base_for_solana, explorer_tx_suffix_for_solana, network_label_for_evm,
    network_label_for_solana, usdc_mint_for_solana,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod test;
