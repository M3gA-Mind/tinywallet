//! Typed callbacks from the wallet implementation to its composing host.

use tinybus::{Connection, Error as BusError, Proxy};
use tinywallet_bus::wire::{
    AuthorizationResult, AuthorizeWeb3ExecutionRequest, AuthorizeX402PaymentRequest,
    ChainTransportRequest, ChainTransportResult, HostResponse, WalletAccountsResponse,
    WalletBackendRequest, WalletBackendResponse, X402ConfigResponse,
};

use super::{HOST_INTERFACE, HOST_OBJECT_PATH};

/// A typed handle to the host services the module is permitted to request.
#[derive(Clone)]
pub(crate) struct HostCallbacks {
    proxy: Proxy,
}

impl std::fmt::Debug for HostCallbacks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostCallbacks").finish_non_exhaustive()
    }
}

impl HostCallbacks {
    /// Bind the callback handle to the host interface on the module connection.
    pub(crate) fn new(connection: &Connection) -> Result<Self, BusError> {
        Ok(Self {
            proxy: connection.proxy(HOST_INTERFACE, HOST_OBJECT_PATH, HOST_INTERFACE)?,
        })
    }

    /// Perform one authenticated wallet-backend transport operation.
    pub(crate) async fn wallet_backend(
        &self,
        request: WalletBackendRequest,
    ) -> Result<WalletBackendResponse, String> {
        self.call("WalletBackend", (request,)).await
    }

    /// Read public account state from the host's custody store.
    pub(crate) async fn wallet_accounts(&self) -> Result<WalletAccountsResponse, String> {
        self.call("WalletAccounts", ()).await
    }

    /// Read the module ledger's workspace root and budget policy.
    pub(crate) async fn x402_config(&self) -> Result<X402ConfigResponse, String> {
        self.call("X402Config", ()).await
    }

    /// Execute one chain transport operation using host endpoint and proxy
    /// policy.
    pub(crate) async fn chain_transport(
        &self,
        request: ChainTransportRequest,
    ) -> Result<ChainTransportResult, String> {
        self.call("ChainTransport", (request,)).await
    }

    /// Request approval for the exact facts in a stored Web3 transaction.
    pub(crate) async fn authorize_web3_execution(
        &self,
        request: AuthorizeWeb3ExecutionRequest,
    ) -> Result<AuthorizationResult, String> {
        self.call("AuthorizeWeb3Execution", (request,)).await
    }

    /// Request host approval for exact x402 payment terms.
    pub(crate) async fn authorize_x402_payment(
        &self,
        request: AuthorizeX402PaymentRequest,
    ) -> Result<AuthorizationResult, String> {
        self.call("AuthorizeX402Payment", (request,)).await
    }

    async fn call<R: serde::de::DeserializeOwned>(
        &self,
        member: &str,
        args: impl serde::Serialize,
    ) -> Result<R, String> {
        match self.proxy.call::<HostResponse>(member, args).await {
            Ok(response) => extract_response(member, response),
            Err(error) => Err(error.to_string()),
        }
    }
}

fn extract_response<R: serde::de::DeserializeOwned>(
    expected: &str,
    response: HostResponse,
) -> Result<R, String> {
    let value = match (expected, response) {
        ("WalletBackend", HostResponse::WalletBackend(value)) => serde_json::to_value(value),
        ("WalletAccounts", HostResponse::WalletAccounts(value)) => serde_json::to_value(value),
        ("X402Config", HostResponse::X402Config(value)) => serde_json::to_value(value),
        ("ChainTransport", HostResponse::ChainTransport(value)) => serde_json::to_value(value),
        ("AuthorizeWeb3Execution", HostResponse::Authorization(value)) => {
            serde_json::to_value(value)
        }
        ("AuthorizeX402Payment", HostResponse::Authorization(value)) => serde_json::to_value(value),
        (_, response) => {
            return Err(format!(
                "host callback {expected} returned the wrong response kind: {response:?}"
            ));
        }
    }
    .map_err(|error| format!("encoding host callback response {expected}: {error}"))?;
    serde_json::from_value(value)
        .map_err(|error| format!("decoding host callback response {expected}: {error}"))
}
