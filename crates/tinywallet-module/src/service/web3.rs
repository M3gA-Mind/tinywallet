//! Module-owned quote runtime over the host's typed network and account seams.

use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::Value;
use tinywallet_bus::wire::{
    AuthorizeWeb3ExecutionRequest, ChainTransportRequest, ChainTransportResult, NetworkRequest,
    SecretMaterial, WalletBackendRequest, Web3ExecutionIntent,
};
use tinywallet_crypto::rpc::{NetworkId, Transport, TransportError, TransportResult};
use tinywallet_web3::crypto::defaults::{EvmNetwork, RpcSource, SolanaCluster, default_rpc_url};
use tinywallet_web3::crypto::seams::{RpcEndpoints, WalletAccounts, WalletSigner, Web3Backend};
use tinywallet_web3::crypto::wallet::{WalletChain, WalletEngine, WalletSeams, WalletStatus};
use tinywallet_web3::quote::QuoteOwner as LibraryQuoteOwner;
use tinywallet_web3::seams::QuoteScope;

use super::host::HostCallbacks;

tokio::task_local! {
    static ACTIVE_QUOTE_OWNER: Option<LibraryQuoteOwner>;
    static ACTIVE_SECRET: Arc<SecretSlot>;
    static ACTIVE_WEB3_APPROVAL: Option<Web3ApprovalContext>;
}

#[derive(Clone)]
pub(crate) struct Web3ApprovalContext {
    pub(crate) quote_id: String,
    pub(crate) operation: String,
    pub(crate) solana_intent: Option<Web3ExecutionIntent>,
}

struct SecretSlot(Mutex<Option<SecretMaterial>>);

impl Drop for SecretSlot {
    fn drop(&mut self) {
        if let Ok(slot) = self.0.get_mut()
            && let Some(secret) = slot.as_mut()
        {
            zeroize::Zeroize::zeroize(&mut secret.mnemonic);
        }
    }
}

/// The stateful web3 service owned by this loaded wallet module instance.
pub(crate) struct Web3Runtime {
    /// Module-owned TTL'd swap, bridge and dapp quote service.
    pub(crate) service: Arc<tinywallet_web3::crypto::service::Web3Service>,
    pub(crate) callbacks: HostCallbacks,
}

impl Web3Runtime {
    /// Compose the owner library over host-owned custody facts and transports.
    pub(crate) fn new(host: HostCallbacks) -> Self {
        let engine = Arc::new(WalletEngine::new(WalletSeams {
            transport: Arc::new(HostTransport(host.clone())),
            endpoints: Arc::new(DefaultEndpoints),
            signer: Arc::new(ModuleSigner(host.clone())),
            accounts: Arc::new(HostAccounts(host.clone())),
            scope: Arc::new(TaskQuoteScope),
        }));
        Self {
            service: Arc::new(tinywallet_web3::crypto::service::Web3Service::new(
                engine,
                Arc::new(HostWeb3Backend(host.clone())),
            )),
            callbacks: host,
        }
    }

    pub(crate) fn execution_approval(&self, quote_id: &str) -> Result<Web3ApprovalContext, String> {
        let quote = self
            .service
            .stored_quotes()
            .into_iter()
            .find(|quote| quote.quote_id == quote_id)
            .ok_or_else(|| format!("quote '{quote_id}' not found"))?;
        let solana_intent = match quote.unsigned {
            tinywallet_web3::crypto::service::UnsignedTx::Evm { .. } => None,
            tinywallet_web3::crypto::service::UnsignedTx::Solana { tx_blob_hex } => {
                Some(Web3ExecutionIntent::Solana { tx_blob_hex })
            }
        };
        let operation = match quote.kind {
            tinywallet_web3::crypto::service::Web3QuoteKind::Swap => "swap",
            tinywallet_web3::crypto::service::Web3QuoteKind::Bridge => "bridge",
            tinywallet_web3::crypto::service::Web3QuoteKind::DappCall => "dapp_call",
        };
        Ok(Web3ApprovalContext {
            quote_id: quote.quote_id,
            operation: operation.to_string(),
            solana_intent,
        })
    }
}

/// Run an execution with one scoped secret; the slot wipes any unconsumed copy
/// when the operation returns or fails.
pub(crate) async fn with_execution_context<F: Future>(
    owner: Option<tinywallet_bus::wire::QuoteOwner>,
    secret: SecretMaterial,
    approval: Option<Web3ApprovalContext>,
    future: F,
) -> F::Output {
    let slot = Arc::new(SecretSlot(Mutex::new(Some(secret))));
    ACTIVE_WEB3_APPROVAL
        .scope(
            approval,
            ACTIVE_SECRET.scope(slot, with_quote_owner(owner, future)),
        )
        .await
}

fn take_secret() -> Result<SecretMaterial, String> {
    ACTIVE_SECRET
        .try_with(|slot| {
            slot.0
                .lock()
                .map_err(|_| "wallet secret slot is unavailable".to_string())?
                .take()
                .ok_or_else(|| "wallet secret was already consumed".to_string())
        })
        .map_err(|_| "wallet secret is unavailable outside a confirmed execution".to_string())?
}

pub(super) fn clone_secret() -> Result<SecretMaterial, String> {
    ACTIVE_SECRET
        .try_with(|slot| {
            slot.0
                .lock()
                .map_err(|_| "wallet secret slot is unavailable".to_string())?
                .as_ref()
                .cloned()
                .ok_or_else(|| "wallet secret was already consumed".to_string())
        })
        .map_err(|_| "wallet secret is unavailable outside a confidential payment".to_string())?
}

/// Run a quote operation with the owner that arrived in the typed bus request.
pub(crate) async fn with_quote_owner<F: Future>(
    owner: Option<tinywallet_bus::wire::QuoteOwner>,
    future: F,
) -> F::Output {
    ACTIVE_QUOTE_OWNER
        .scope(
            owner.map(|owner| LibraryQuoteOwner {
                thread_id: owner.thread_id,
                client_id: owner.client_id,
            }),
            future,
        )
        .await
}

struct TaskQuoteScope;

impl QuoteScope for TaskQuoteScope {
    fn current_owner(&self) -> Option<LibraryQuoteOwner> {
        ACTIVE_QUOTE_OWNER
            .try_with(Clone::clone)
            .unwrap_or_default()
    }
}

struct HostAccounts(HostCallbacks);

#[async_trait]
impl WalletAccounts for HostAccounts {
    async fn status(&self) -> Result<WalletStatus, String> {
        let response = self.0.wallet_accounts().await?;
        let accounts = response
            .accounts
            .into_iter()
            .map(|account| {
                Ok(tinywallet_web3::crypto::wallet::WalletAccount {
                    chain: match account.chain {
                        tinywallet_bus::Chain::Evm => WalletChain::Evm,
                        tinywallet_bus::Chain::Btc => WalletChain::Btc,
                        tinywallet_bus::Chain::Solana => WalletChain::Solana,
                        tinywallet_bus::Chain::Tron => WalletChain::Tron,
                        _ => {
                            return Err(format!(
                                "host returned unsupported wallet chain: {:?}",
                                account.chain
                            ));
                        }
                    },
                    address: account.address,
                    derivation_path: account.derivation_path,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(WalletStatus {
            configured: response.configured,
            onboarding_completed: response.configured,
            consent_granted: false,
            secret_stored: response.configured,
            source: None,
            mnemonic_word_count: None,
            accounts,
            updated_at_ms: None,
        })
    }
}

struct HostWeb3Backend(HostCallbacks);

#[async_trait]
impl Web3Backend for HostWeb3Backend {
    async fn routes(&self) -> Result<Value, String> {
        Ok(self
            .0
            .wallet_backend(WalletBackendRequest::Routes)
            .await?
            .data)
    }

    async fn swap_tx(&self, body: &Value) -> Result<Value, String> {
        Ok(self
            .0
            .wallet_backend(WalletBackendRequest::SwapTransaction { body: body.clone() })
            .await?
            .data)
    }

    async fn bridge_tx(&self, body: &Value) -> Result<Value, String> {
        Ok(self
            .0
            .wallet_backend(WalletBackendRequest::BridgeTransaction { body: body.clone() })
            .await?
            .data)
    }
}

pub(super) struct HostTransport(pub(super) HostCallbacks);

#[async_trait]
impl Transport for HostTransport {
    async fn json_rpc(
        &self,
        network: NetworkId,
        method: &str,
        params: Value,
    ) -> TransportResult<Value> {
        self.call(ChainTransportRequest::JsonRpc {
            network: network_request(network)
                .map_err(|message| TransportError::Unreachable { network, message })?,
            method: method.to_string(),
            params,
        })
        .await
        .and_then(|result| match result {
            ChainTransportResult::Ok { body } => Ok(body),
            ChainTransportResult::Unreachable { message } => {
                Err(TransportError::Unreachable { network, message })
            }
            ChainTransportResult::RpcError { message } => {
                Err(TransportError::Rpc { network, message })
            }
        })
    }

    async fn rest_get(&self, network: NetworkId, path: &str) -> TransportResult<String> {
        self.call(ChainTransportRequest::RestGet {
            network: network_request(network)
                .map_err(|message| TransportError::Unreachable { network, message })?,
            path: path.to_string(),
        })
        .await
        .and_then(|result| rest_result(network, result))
    }

    async fn rest_post(
        &self,
        network: NetworkId,
        path: &str,
        body: String,
        content_type: &str,
    ) -> TransportResult<String> {
        self.call(ChainTransportRequest::RestPost {
            network: network_request(network)
                .map_err(|message| TransportError::Unreachable { network, message })?,
            path: path.to_string(),
            body,
            content_type: content_type.to_string(),
        })
        .await
        .and_then(|result| rest_result(network, result))
    }
}

impl HostTransport {
    async fn call(&self, request: ChainTransportRequest) -> TransportResult<ChainTransportResult> {
        let network_request = match &request {
            ChainTransportRequest::JsonRpc { network, .. }
            | ChainTransportRequest::RestGet { network, .. }
            | ChainTransportRequest::RestPost { network, .. } => *network,
        };
        let network = network_id(network_request).map_err(|message| TransportError::Rpc {
            network: NetworkId {
                chain: tinywallet_crypto::Chain::Evm,
                evm_chain_id: None,
            },
            message,
        })?;
        self.0
            .chain_transport(request)
            .await
            .map_err(|message| TransportError::Unreachable { network, message })
    }
}

fn rest_result(network: NetworkId, result: ChainTransportResult) -> TransportResult<String> {
    match result {
        ChainTransportResult::Ok {
            body: Value::String(body),
        } => Ok(body),
        ChainTransportResult::Ok { body } => Err(TransportError::Rpc {
            network,
            message: format!("REST provider returned a non-string body: {body}"),
        }),
        ChainTransportResult::Unreachable { message } => {
            Err(TransportError::Unreachable { network, message })
        }
        ChainTransportResult::RpcError { message } => Err(TransportError::Rpc { network, message }),
    }
}

fn network_request(network: NetworkId) -> Result<NetworkRequest, String> {
    Ok(NetworkRequest {
        chain: match network.chain {
            tinywallet_crypto::Chain::Evm => tinywallet_bus::Chain::Evm,
            tinywallet_crypto::Chain::Btc => tinywallet_bus::Chain::Btc,
            tinywallet_crypto::Chain::Solana => tinywallet_bus::Chain::Solana,
            tinywallet_crypto::Chain::Tron => tinywallet_bus::Chain::Tron,
            _ => return Err(format!("unsupported transport chain: {:?}", network.chain)),
        },
        evm_chain_id: network.evm_chain_id,
    })
}

fn network_id(network: NetworkRequest) -> Result<NetworkId, String> {
    let chain = match network.chain {
        tinywallet_bus::Chain::Evm => tinywallet_crypto::Chain::Evm,
        tinywallet_bus::Chain::Btc => tinywallet_crypto::Chain::Btc,
        tinywallet_bus::Chain::Solana => tinywallet_crypto::Chain::Solana,
        tinywallet_bus::Chain::Tron => tinywallet_crypto::Chain::Tron,
        _ => return Err(format!("unsupported transport chain: {:?}", network.chain)),
    };
    Ok(NetworkId {
        chain,
        evm_chain_id: network.evm_chain_id,
    })
}

struct DefaultEndpoints;

impl RpcEndpoints for DefaultEndpoints {
    fn url(&self, chain: WalletChain, network: Option<EvmNetwork>) -> String {
        match chain {
            WalletChain::Evm => network
                .unwrap_or(EvmNetwork::EthereumMainnet)
                .default_rpc_url()
                .to_string(),
            other => default_rpc_url(other, self.solana_cluster()).to_string(),
        }
    }

    fn source(&self, _chain: WalletChain, _network: Option<EvmNetwork>) -> RpcSource {
        RpcSource::Default
    }

    fn solana_cluster(&self) -> SolanaCluster {
        SolanaCluster::Mainnet
    }
}

struct ModuleSigner(HostCallbacks);

#[async_trait]
impl WalletSigner for ModuleSigner {
    async fn derive_account(
        &self,
        _chain: WalletChain,
    ) -> Result<tinywallet_bus::wire::DerivedAccount, String> {
        let mut secret = take_secret()?;
        let result = super::derive_account(&secret).map_err(super::failure_message);
        zeroize::Zeroize::zeroize(&mut secret.mnemonic);
        result
    }

    async fn sign_transaction(
        &self,
        chain: WalletChain,
        transaction: &tinywallet_bus::wire::TransactionSpec,
    ) -> Result<tinywallet_bus::wire::SignedTransaction, String> {
        if chain == WalletChain::Evm {
            let approval = ACTIVE_WEB3_APPROVAL
                .try_with(Clone::clone)
                .map_err(|_| "EVM quote approval context is unavailable".to_string())?
                .ok_or_else(|| "EVM quote approval context is unavailable".to_string())?;
            let tinywallet_bus::wire::TransactionSpec::Evm {
                to,
                value_wei,
                data_hex,
                nonce,
                gas_limit,
                gas_price_wei,
                chain_id,
            } = transaction
            else {
                return Err("EVM signer received a non-EVM transaction".to_string());
            };
            let decision = self
                .0
                .authorize_web3_execution(AuthorizeWeb3ExecutionRequest {
                    quote_id: approval.quote_id,
                    operation: approval.operation,
                    transaction: Web3ExecutionIntent::Evm {
                        network: format!("eip155:{chain_id}"),
                        to: to.clone(),
                        data: Some(data_hex.clone()),
                        value: value_wei.clone(),
                        nonce: *nonce,
                        gas_limit: *gas_limit,
                        gas_price_wei: gas_price_wei.clone(),
                        chain_id: *chain_id,
                    },
                })
                .await?;
            if !decision.approved {
                return Err(decision
                    .reason
                    .unwrap_or_else(|| "wallet transaction was not approved".to_string()));
            }
        }
        let mut request = tinywallet_bus::wire::SignRequest {
            secret: take_secret()?,
            transaction: transaction.clone(),
        };
        if request.secret.chain != chain.to_chain() {
            zeroize::Zeroize::zeroize(&mut request.secret.mnemonic);
            return Err(
                "wallet secret chain does not match the requested signing chain".to_string(),
            );
        }
        let result = super::sign_transaction(&request).map_err(super::failure_message);
        zeroize::Zeroize::zeroize(&mut request.secret.mnemonic);
        result
    }

    async fn sign_message(
        &self,
        chain: WalletChain,
        message: &[u8],
        scheme: tinywallet_bus::wire::Scheme,
    ) -> Result<tinywallet_bus::wire::Signature, String> {
        let mut request = tinywallet_bus::wire::SignMessageRequest {
            secret: take_secret()?,
            message_hex: super::hex(message),
            scheme,
        };
        if request.secret.chain != chain.to_chain() {
            zeroize::Zeroize::zeroize(&mut request.secret.mnemonic);
            return Err(
                "wallet secret chain does not match the requested signing chain".to_string(),
            );
        }
        let result = super::sign_message(&request).map_err(super::failure_message);
        zeroize::Zeroize::zeroize(&mut request.secret.mnemonic);
        result
    }
}
