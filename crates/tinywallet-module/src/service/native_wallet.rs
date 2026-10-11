//! Explicitly configured native-wallet engine, with host-only endpoint aliases.

use std::collections::HashMap;
use std::sync::Arc;

use tinywallet_bus::wire::{
    HostEndpointReference, NativeWalletConfigRequest, NativeWalletConfigResponse,
    NativeWalletServiceHandle, RpcEndpointKey, RpcSource, SolanaCluster,
};
use tinywallet_web3::crypto::defaults::{EvmNetwork, default_rpc_url, default_rpc_url_for_evm};
use tinywallet_web3::crypto::seams::RpcEndpoints;
use tinywallet_web3::crypto::wallet::{WalletChain, WalletEngine, WalletSeams};

use super::host::HostCallbacks;
use super::web3::{HostTransport, ModuleSigner, TaskQuoteScope};

/// The active handle and implementation engine are replaced atomically by the
/// outer `Wallet` write lock. Operations hold its read lock for their entire
/// future, so close/configure waits for signing, transport and approval calls.
pub(crate) struct NativeWalletRuntime {
    pub(crate) engine: WalletEngine,
    pub(crate) callbacks: HostCallbacks,
    handle: NativeWalletServiceHandle,
}

struct ConfiguredEndpoints {
    cluster: SolanaCluster,
    references: HashMap<RpcEndpointKey, String>,
}

impl NativeWalletRuntime {
    pub(crate) fn new(
        callbacks: HostCallbacks,
        config: NativeWalletConfigRequest,
    ) -> Result<(Self, NativeWalletConfigResponse), String> {
        let mut references = HashMap::new();
        let mut advertised = Vec::with_capacity(config.overridden_endpoints.len());
        for key in config.overridden_endpoints {
            if references.contains_key(&key) {
                return Err("native wallet endpoint keys must be unique".to_string());
            }
            let reference = uuid::Uuid::new_v4().to_string();
            references.insert(key, reference.clone());
            advertised.push(HostEndpointReference { key, reference });
        }
        let endpoints = Arc::new(ConfiguredEndpoints {
            cluster: config.solana_cluster,
            references,
        });
        let engine = WalletEngine::new(WalletSeams {
            transport: Arc::new(HostTransport::with_endpoints(
                callbacks.clone(),
                endpoints.clone(),
            )),
            endpoints: endpoints.clone(),
            signer: Arc::new(ModuleSigner(callbacks.clone())),
            accounts: Arc::new(super::web3::HostAccounts(callbacks.clone())),
            scope: Arc::new(TaskQuoteScope),
        });
        let handle = NativeWalletServiceHandle {
            handle: uuid::Uuid::new_v4().to_string(),
        };
        let response = NativeWalletConfigResponse {
            service: handle.clone(),
            endpoint_references: advertised,
        };
        Ok((
            Self {
                engine,
                callbacks,
                handle,
            },
            response,
        ))
    }

    pub(crate) fn require_handle(&self, handle: &NativeWalletServiceHandle) -> Result<(), String> {
        if handle != &self.handle {
            return Err("native wallet service handle is unavailable".to_string());
        }
        Ok(())
    }
}

impl RpcEndpoints for ConfiguredEndpoints {
    fn url(&self, chain: WalletChain, network: Option<EvmNetwork>) -> String {
        self.references
            .get(&Self::key(chain, network))
            .cloned()
            .unwrap_or_else(|| match chain {
                WalletChain::Evm => {
                    default_rpc_url_for_evm(network.unwrap_or(EvmNetwork::EthereumMainnet))
                        .to_string()
                }
                other => default_rpc_url(other, self.cluster).to_string(),
            })
    }

    fn source(&self, chain: WalletChain, network: Option<EvmNetwork>) -> RpcSource {
        if self.references.contains_key(&Self::key(chain, network)) {
            RpcSource::EnvOverride
        } else {
            RpcSource::Default
        }
    }

    fn endpoint_ref(&self, chain: WalletChain, network: Option<EvmNetwork>) -> Option<String> {
        self.references.get(&Self::key(chain, network)).cloned()
    }

    fn solana_cluster(&self) -> SolanaCluster {
        self.cluster
    }
}

impl ConfiguredEndpoints {
    fn key(chain: WalletChain, network: Option<EvmNetwork>) -> RpcEndpointKey {
        match chain {
            WalletChain::Evm => RpcEndpointKey::Evm {
                network: network.unwrap_or(EvmNetwork::EthereumMainnet),
            },
            WalletChain::Btc => RpcEndpointKey::Btc,
            WalletChain::Solana => RpcEndpointKey::Solana,
            WalletChain::Tron => RpcEndpointKey::Tron,
        }
    }
}
