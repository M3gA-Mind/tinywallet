#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::tests::callbacks;
use super::*;

fn fixture_bytes_field(tag: u8, bytes: &[u8]) -> Vec<u8> {
    let mut result = vec![tag, u8::try_from(bytes.len()).unwrap()];
    result.extend(bytes);
    result
}

fn fixture_varint(mut value: u64) -> Vec<u8> {
    let mut result = Vec::new();
    while value >= 0x80 {
        result.push(u8::try_from(value & 0x7f).unwrap() | 0x80);
        value >>= 7;
    }
    result.push(u8::try_from(value).unwrap());
    result
}

fn read_shortvec(bytes: &[u8], offset: &mut usize) -> usize {
    let mut value = 0usize;
    let mut shift = 0;
    loop {
        let byte = bytes[*offset];
        *offset += 1;
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Covers one complete native wallet configuration lifecycle"
)]
async fn native_wallet_configuration_uses_opaque_endpoint_references_and_invalidates_old_handles() {
    use tinywallet_bus::wire::{
        NativeWalletConfigRequest, NativeWalletServiceRequest, NativeWalletTransactionRequest,
        RpcEndpointKey, SolanaCluster, WalletChain,
    };
    use tinywallet_web3::crypto::defaults::EvmNetwork;

    let (callbacks, _, _, transports, _host, broker_task) = callbacks().await;
    let wallet = crate::service::Wallet {
        callbacks: Some(callbacks),
        ..crate::service::Wallet::default()
    };
    let configured = wallet
        .native_wallet_configure(NativeWalletConfigRequest {
            solana_cluster: SolanaCluster::Devnet,
            overridden_endpoints: vec![
                RpcEndpointKey::Evm {
                    network: EvmNetwork::BaseMainnet,
                },
                RpcEndpointKey::Btc,
            ],
        })
        .await
        .unwrap();
    let base_ref = configured
        .endpoint_references
        .iter()
        .find(|endpoint| {
            endpoint.key
                == (RpcEndpointKey::Evm {
                    network: EvmNetwork::BaseMainnet,
                })
        })
        .unwrap()
        .reference
        .clone();
    assert!(!base_ref.starts_with("http"));
    let old = configured.service.clone();
    let defaults = wallet
        .native_wallet_network_defaults(NativeWalletServiceRequest {
            service: old.clone(),
        })
        .await
        .unwrap();
    let base = defaults
        .iter()
        .find(|row| row.evm_network == Some(EvmNetwork::BaseMainnet))
        .unwrap();
    assert_eq!(base.rpc_url, base_ref);
    assert_eq!(base.rpc_source, RpcSource::EnvOverride);
    assert!(configured.endpoint_references.iter().all(|endpoint| {
        !endpoint.reference.starts_with("http") && !endpoint.reference.contains('/')
    }));
    let solana = defaults
        .iter()
        .find(|row| row.chain == WalletChain::Solana)
        .unwrap();
    assert_eq!(solana.rpc_url, "https://api.devnet.solana.com");
    let assets = wallet
        .native_wallet_supported_assets(NativeWalletServiceRequest {
            service: old.clone(),
        })
        .await
        .unwrap();
    assert!(assets.iter().any(|asset| {
        asset.chain == WalletChain::Solana
            && asset.symbol == "USDC"
            && asset.contract_address.as_deref()
                == Some("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU")
    }));
    let statuses = wallet
        .native_wallet_chain_status(NativeWalletServiceRequest {
            service: old.clone(),
        })
        .await
        .unwrap();
    assert_eq!(statuses.len(), 9);
    let balances = wallet
        .native_wallet_balances(NativeWalletServiceRequest {
            service: old.clone(),
        })
        .await
        .unwrap();
    assert_eq!(balances.len(), 6);

    // A provider read proves the opaque reference is carried to the host
    // callback while the URL stays inside the host.
    let _ = wallet
        .native_wallet_tx_status(NativeWalletTransactionRequest {
            service: old.clone(),
            chain: WalletChain::Evm,
            evm_network: Some(EvmNetwork::BaseMainnet),
            hash: "0xfeed".to_string(),
        })
        .await;
    assert!(
        wallet
            .native_wallet_tx_receipt(NativeWalletTransactionRequest {
                service: old.clone(),
                chain: WalletChain::Evm,
                evm_network: Some(EvmNetwork::BaseMainnet),
                hash: "0xfeed".to_string(),
            })
            .await
            .is_ok()
    );
    assert!(
        wallet
            .native_wallet_lookup_tx(NativeWalletTransactionRequest {
                service: old.clone(),
                chain: WalletChain::Evm,
                evm_network: Some(EvmNetwork::BaseMainnet),
                hash: "0xfeed".to_string(),
            })
            .await
            .is_ok()
    );
    let saw_opaque_reference = {
        let requests = transports.lock().unwrap();
        requests.iter().any(|request| match request {
            ChainTransportRequest::JsonRpc { network, .. }
            | ChainTransportRequest::RestGet { network, .. }
            | ChainTransportRequest::RestPost { network, .. } => {
                network.endpoint_ref.as_deref() == Some(base_ref.as_str())
            }
        })
    };
    assert!(saw_opaque_reference);

    let replacement = wallet
        .native_wallet_configure(NativeWalletConfigRequest {
            solana_cluster: SolanaCluster::Mainnet,
            overridden_endpoints: Vec::new(),
        })
        .await
        .unwrap();
    assert!(
        wallet
            .native_wallet_supported_assets(NativeWalletServiceRequest { service: old })
            .await
            .unwrap_err()
            .to_string()
            .contains("NativeWalletOperationFailed")
    );
    wallet
        .native_wallet_close(tinywallet_bus::wire::NativeWalletCloseRequest {
            service: replacement.service,
        })
        .await
        .unwrap();
    broker_task.abort();
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Covers ownership, approval and execution in one service flow"
)]
async fn native_wallet_checks_quote_owner_before_exact_signing_approval() {
    use tinywallet_bus::wire::{
        NativeWalletConfigRequest, NativeWalletExecuteRequest, NativeWalletPrepareTransferRequest,
        NativeWalletServiceHandle, PrepareTransferParams, QuoteOwner, SolanaCluster, WalletChain,
    };
    use tinywallet_web3::crypto::defaults::EvmNetwork;

    let (callbacks, _, native_requests, _, _host, broker_task) = callbacks().await;
    let wallet = crate::service::Wallet {
        callbacks: Some(callbacks),
        ..crate::service::Wallet::default()
    };
    let configured = wallet
        .native_wallet_configure(NativeWalletConfigRequest {
            solana_cluster: SolanaCluster::Mainnet,
            overridden_endpoints: Vec::new(),
        })
        .await
        .unwrap();
    let owner = QuoteOwner {
        thread_id: "native-wallet-owner-thread".to_string(),
        client_id: "native-wallet-owner-client".to_string(),
    };
    let quote = wallet
        .native_wallet_prepare_transfer(NativeWalletPrepareTransferRequest {
            service: configured.service.clone(),
            owner: Some(owner.clone()),
            params: PrepareTransferParams {
                chain: WalletChain::Evm,
                to_address: "0x3535353535353535353535353535353535353535".to_string(),
                amount_raw: "1000".to_string(),
                asset_symbol: None,
                evm_network: Some(EvmNetwork::BaseMainnet),
            },
        })
        .await
        .unwrap();
    let secret = || {
        SecretMaterial {
        mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string(),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        chain: tinywallet_bus::Chain::Evm,
    }
    };
    let wrong_owner = QuoteOwner {
        thread_id: "another-thread".to_string(),
        client_id: owner.client_id.clone(),
    };
    let error = Box::pin(
        wallet.native_wallet_execute_prepared(NativeWalletExecuteRequest {
            service: configured.service.clone(),
            owner: Some(wrong_owner),
            quote_id: quote.quote_id.clone(),
            confirmed: true,
            secret: secret(),
        }),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("not found"));
    assert!(native_requests.lock().unwrap().is_empty());

    let execution = Box::pin(
        wallet.native_wallet_execute_prepared(NativeWalletExecuteRequest {
            service: configured.service.clone(),
            owner: Some(owner),
            quote_id: quote.quote_id.clone(),
            confirmed: true,
            secret: secret(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(execution.transaction_hash, "0xfeedface");
    {
        let requests = native_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let approval = &requests[0];
        assert_eq!(approval.quote, quote);
        assert_eq!(approval.operation, "transfer");
        assert!(matches!(
            &approval.intent,
            tinywallet_bus::wire::NativeWalletExecutionIntent::Transaction {
                transaction: tinywallet_bus::wire::TransactionSpec::Evm { chain_id: 8453, .. }
            }
        ));
    }
    wallet
        .native_wallet_close(tinywallet_bus::wire::NativeWalletCloseRequest {
            service: NativeWalletServiceHandle {
                handle: configured.service.handle,
            },
        })
        .await
        .unwrap();
    broker_task.abort();
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Checks each chain's signer authorization facts"
)]
async fn native_wallet_signer_authorizes_exact_transaction_facts_for_all_chains() {
    use tinywallet_bus::wire::{
        NativeWalletExecutionIntent, PreparedKind, PreparedStatus, PreparedTransaction, Scheme,
        TransactionSpec, TronTransfer, Utxo,
    };
    use tinywallet_web3::crypto::seams::WalletSigner;
    use tinywallet_web3::crypto::wallet::WalletChain;

    let (callbacks, _, native_requests, _, _host, broker_task) = callbacks().await;
    let quote = PreparedTransaction {
        quote_id: "native_quote_for_signer_test".to_string(),
        kind: PreparedKind::NativeTransfer,
        chain: WalletChain::Evm,
        evm_network: None,
        from_address: "sender".to_string(),
        to_address: "recipient".to_string(),
        asset_symbol: "ETH".to_string(),
        amount_raw: "1".to_string(),
        amount_formatted: "0.000000000000000001 ETH".to_string(),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: None,
        estimated_fee_raw: "0".to_string(),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: 1,
        expires_at_ms: u64::MAX,
        notes: Vec::new(),
    };
    let signer = ModuleSigner(callbacks);
    let transactions = [
        (
            WalletChain::Btc,
            TransactionSpec::Btc {
                from: "bc1qfixture".to_string(),
                to: "bc1qrecipient".to_string(),
                amount_sat: 10,
                fee_sat: 1,
                utxos: vec![Utxo {
                    txid: "00".repeat(32),
                    vout: 0,
                    value: 11,
                }],
            },
        ),
        (
            WalletChain::Evm,
            TransactionSpec::Evm {
                to: "0x3535353535353535353535353535353535353535".to_string(),
                value_wei: "1".to_string(),
                data_hex: "0x".to_string(),
                nonce: 2,
                gas_limit: 21_000,
                gas_price_wei: "3".to_string(),
                chain_id: 8453,
            },
        ),
        (
            WalletChain::Tron,
            TransactionSpec::Tron {
                raw_data_hex: "00".to_string(),
                expected_to: "TRecipient".to_string(),
                expected_txid: "aa".repeat(32),
                transfer: TronTransfer::Native { amount_sun: 12 },
            },
        ),
    ];
    for (chain, transaction) in &transactions {
        let result = with_native_approval(
            NativeApprovalContext {
                callbacks: signer.0.clone(),
                quote: quote.clone(),
            },
            signer.sign_transaction(*chain, transaction),
        )
        .await;
        assert!(
            result.is_err(),
            "the fixture deliberately has no secret slot"
        );
    }
    let solana_message = [0, 1, 127, 255];
    let result = with_native_approval(
        NativeApprovalContext {
            callbacks: signer.0.clone(),
            quote: quote.clone(),
        },
        signer.sign_message(WalletChain::Solana, &solana_message, Scheme::Ed25519),
    )
    .await;
    assert!(
        result.is_err(),
        "the fixture deliberately has no secret slot"
    );

    let approvals = native_requests.lock().unwrap();
    assert_eq!(approvals.len(), 4);
    for ((_, expected), actual) in transactions.iter().zip(&approvals[..3]) {
        assert_eq!(actual.quote, quote);
        assert_eq!(actual.quote_id, quote.quote_id);
        assert_eq!(actual.operation, "transfer");
        assert_eq!(
            actual.intent,
            NativeWalletExecutionIntent::Transaction {
                transaction: expected.clone(),
            }
        );
    }
    assert_eq!(
        approvals[3].intent,
        NativeWalletExecutionIntent::SolanaMessage {
            message_hex: "00017fff".to_string(),
        }
    );
    broker_task.abort();
}

#[tokio::test]
async fn native_solana_signing_decodes_to_the_approved_transfer() {
    use base64::Engine as _;
    use tinywallet_bus::wire::{
        NativeWalletExecutionIntent, PreparedKind, PreparedStatus, PreparedTransaction,
        SecretMaterial, TransactionSpec,
    };
    use tinywallet_web3::crypto::seams::WalletSigner;
    use tinywallet_web3::crypto::wallet::WalletChain;

    const BLOCKHASH: &str = "GHtXQBsoZHVnNFa9YevAzFr17DJjgHXk3ycTKD5xD3Zi";
    const RECIPIENT: &str = "Vote111111111111111111111111111111111111111";
    const LAMPORTS: u64 = 9_876;
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let derivation_path = "m/44'/501'/0'/0'";
    let sender = tinywallet::key::derive(tinywallet_bus::Chain::Solana, mnemonic, derivation_path)
        .unwrap()
        .address()
        .to_string();
    let transaction = TransactionSpec::Solana {
        from: sender.clone(),
        to: RECIPIENT.to_string(),
        lamports: LAMPORTS,
        recent_blockhash: BLOCKHASH.to_string(),
    };
    let quote = PreparedTransaction {
        quote_id: "solana-native-signing-test".to_string(),
        kind: PreparedKind::NativeTransfer,
        chain: WalletChain::Solana,
        evm_network: None,
        from_address: sender.clone(),
        to_address: RECIPIENT.to_string(),
        asset_symbol: "SOL".to_string(),
        amount_raw: LAMPORTS.to_string(),
        amount_formatted: "0.000009876 SOL".to_string(),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: None,
        estimated_fee_raw: "5000".to_string(),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: 1,
        expires_at_ms: u64::MAX,
        notes: Vec::new(),
    };
    let (callbacks, _, native_requests, _, _host, broker_task) = callbacks().await;
    let signer = ModuleSigner(callbacks.clone());
    let approval = NativeApprovalContext {
        callbacks,
        quote: quote.clone(),
    };
    let secret = SecretMaterial {
        mnemonic: mnemonic.to_string(),
        derivation_path: derivation_path.to_string(),
        chain: tinywallet_bus::Chain::Solana,
    };
    let signed = with_execution_context(None, secret, None, async {
        with_native_approval(
            approval,
            signer.sign_transaction(WalletChain::Solana, &transaction),
        )
        .await
    })
    .await
    .unwrap();

    let requests = native_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].quote, quote);
    assert_eq!(
        requests[0].intent,
        NativeWalletExecutionIntent::Transaction { transaction }
    );
    drop(requests);

    let wire = base64::engine::general_purpose::STANDARD
        .decode(signed.raw)
        .unwrap();
    let mut offset = 0;
    assert_eq!(read_shortvec(&wire, &mut offset), 1, "one signature slot");
    offset += 64;
    let message = &wire[offset..];
    assert_eq!(&message[..3], &[1, 0, 1], "legacy transfer header");
    let mut message_offset = 3;
    assert_eq!(
        read_shortvec(message, &mut message_offset),
        3,
        "payer, recipient, system program"
    );
    let keys_start = message_offset;
    let keys_end = keys_start + 3 * 32;
    let keys = &message[keys_start..keys_end];
    assert_eq!(
        &keys[..32],
        &tinywallet::address::solana::decode(&sender).unwrap(),
        "payer matches the approved sender"
    );
    assert_eq!(
        &keys[32..64],
        &tinywallet::address::solana::decode(RECIPIENT).unwrap(),
        "recipient matches the approved transfer"
    );
    assert_eq!(
        &message[keys_end..keys_end + 32],
        &tinywallet::address::solana::decode(BLOCKHASH).unwrap(),
        "blockhash matches the typed transaction"
    );
    message_offset = keys_end + 32;
    assert_eq!(
        read_shortvec(message, &mut message_offset),
        1,
        "one system transfer instruction"
    );
    assert_eq!(message[message_offset], 2, "system program account index");
    message_offset += 1;
    assert_eq!(
        read_shortvec(message, &mut message_offset),
        2,
        "sender and recipient accounts"
    );
    assert_eq!(&message[message_offset..message_offset + 2], &[0, 1]);
    message_offset += 2;
    assert_eq!(
        read_shortvec(message, &mut message_offset),
        12,
        "system transfer instruction length"
    );
    assert_eq!(
        u32::from_le_bytes(
            message[message_offset..message_offset + 4]
                .try_into()
                .unwrap()
        ),
        2,
        "SystemInstruction::Transfer"
    );
    assert_eq!(
        u64::from_le_bytes(
            message[message_offset + 4..message_offset + 12]
                .try_into()
                .unwrap()
        ),
        LAMPORTS,
        "lamports match the approved quote"
    );
    broker_task.abort();
}

#[tokio::test]
async fn native_signer_can_derive_and_then_sign_with_one_scoped_secret() {
    use tinywallet_bus::wire::{Scheme, SecretMaterial, TransactionSpec, TronTransfer};
    use tinywallet_web3::crypto::seams::WalletSigner;
    use tinywallet_web3::crypto::wallet::WalletChain;

    let (callbacks, _, _, _, _host, broker_task) = callbacks().await;
    let signer = ModuleSigner(callbacks);
    let secret = SecretMaterial {
        mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string(),
        derivation_path: "m/44'/501'/0'/0'".to_string(),
        chain: tinywallet_bus::Chain::Solana,
    };

    let (account, signature) = with_execution_context(None, secret, None, async {
        let account = signer.derive_account(WalletChain::Solana).await?;
        let signature = signer
            .sign_message(WalletChain::Solana, b"native transfer", Scheme::Ed25519)
            .await?;
        Ok::<_, String>((account, signature))
    })
    .await
    .unwrap();

    assert_ne!(account.address, "");
    assert!(matches!(
        signature,
        tinywallet_bus::wire::Signature::Ed25519 { .. }
    ));

    let tron_secret = SecretMaterial {
        mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string(),
        derivation_path: "m/44'/195'/0'/0/0".to_string(),
        chain: tinywallet_bus::Chain::Tron,
    };
    let derived = tinywallet::key::derive(
        tinywallet::Chain::Tron,
        &tron_secret.mnemonic,
        &tron_secret.derivation_path,
    )
    .unwrap();
    let recipient = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
    let owner_hex = tinywallet::address::tron::to_hex(derived.address()).unwrap();
    let recipient_hex = tinywallet::address::tron::to_hex(recipient).unwrap();
    let decode_hex = |hex: &str| {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect::<Vec<_>>()
    };
    let mut transfer = fixture_bytes_field(0x0a, &decode_hex(&owner_hex));
    transfer.extend(fixture_bytes_field(0x12, &decode_hex(&recipient_hex)));
    let mut amount = fixture_varint(3 << 3);
    amount.extend(fixture_varint(1_000_000));
    transfer.extend(amount);
    let mut any = fixture_bytes_field(0x0a, b"type.googleapis.com/protocol.TransferContract");
    any.extend(fixture_bytes_field(0x12, &transfer));
    let mut contract = fixture_varint(1 << 3);
    contract.extend(fixture_varint(1));
    contract.extend(fixture_bytes_field(0x12, &any));
    let raw_data = fixture_bytes_field(0x5a, &contract);
    let raw_data_hex = super::super::hex(&raw_data);
    let expected_txid = tinywallet::tx::tron::recompute_txid(&raw_data_hex).unwrap();
    let transaction = TransactionSpec::Tron {
        raw_data_hex,
        expected_to: recipient.to_string(),
        expected_txid,
        transfer: TronTransfer::Native {
            amount_sun: 1_000_000,
        },
    };
    let signed_tx = with_execution_context(None, tron_secret, None, async {
        let account = signer.derive_account(WalletChain::Tron).await?;
        assert_eq!(account.address, derived.address());
        signer
            .sign_transaction(WalletChain::Tron, &transaction)
            .await
    })
    .await
    .unwrap();
    assert_eq!(signed_tx.raw.len(), 130);
    broker_task.abort();
}

struct LegacyHost;

#[tinybus::interface(name = "ai.tinyhumans.tinywallet.Host")]
impl LegacyHost {
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "TinyBus interface methods must be async even when this fixture returns immediately"
    )]
    async fn wallet_accounts(&self) -> tinybus::Result<tinywallet_bus::wire::HostResponse> {
        Ok(tinywallet_bus::wire::HostResponse::WalletAccounts(
            tinywallet_bus::wire::WalletAccountsResponse {
                configured: false,
                accounts: Vec::new(),
            },
        ))
    }
}

#[tokio::test]
async fn missing_native_approval_callback_fails_closed_without_signer_fallback() {
    use tinywallet_bus::wire::{
        PreparedKind, PreparedStatus, PreparedTransaction, TransactionSpec,
    };
    use tinywallet_web3::crypto::seams::WalletSigner;
    use tinywallet_web3::crypto::wallet::WalletChain;

    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker = tinybus::broker::Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    host.serve_at(
        crate::service::HOST_OBJECT_PATH.try_into().unwrap(),
        LegacyHost,
    )
    .await
    .unwrap();
    host.request_name(crate::service::HOST_INTERFACE)
        .await
        .unwrap();
    let client = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let callbacks = HostCallbacks::new(&client).unwrap();
    let signer = ModuleSigner(callbacks.clone());
    let quote = PreparedTransaction {
        quote_id: "legacy-host-quote".to_string(),
        kind: PreparedKind::NativeTransfer,
        chain: WalletChain::Evm,
        evm_network: None,
        from_address: "sender".to_string(),
        to_address: "recipient".to_string(),
        asset_symbol: "ETH".to_string(),
        amount_raw: "1".to_string(),
        amount_formatted: "1".to_string(),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: None,
        estimated_fee_raw: "0".to_string(),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: 1,
        expires_at_ms: u64::MAX,
        notes: Vec::new(),
    };
    let result = with_native_approval(
        NativeApprovalContext { callbacks, quote },
        signer.sign_transaction(
            WalletChain::Evm,
            &TransactionSpec::Evm {
                to: "0x3535353535353535353535353535353535353535".to_string(),
                value_wei: "1".to_string(),
                data_hex: "0x".to_string(),
                nonce: 0,
                gas_limit: 21_000,
                gas_price_wei: "1".to_string(),
                chain_id: 1,
            },
        ),
    )
    .await
    .unwrap_err();
    assert!(!result.contains("wallet secret is unavailable"));
    assert!(result.contains("AuthorizeNativeWalletExecution"));
    broker_task.abort();
}
