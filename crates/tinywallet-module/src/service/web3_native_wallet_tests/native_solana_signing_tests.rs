#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

use base64::Engine as _;
use tinywallet_bus::wire::{
    NativeWalletExecutionIntent, PreparedKind, PreparedStatus, PreparedTransaction, SecretMaterial,
    TransactionSpec,
};
use tinywallet_web3::crypto::seams::WalletSigner;
use tinywallet_web3::crypto::wallet::WalletChain;

const BLOCKHASH: &str = "GHtXQBsoZHVnNFa9YevAzFr17DJjgHXk3ycTKD5xD3Zi";
const RECIPIENT: &str = "Vote111111111111111111111111111111111111111";
const LAMPORTS: u64 = 9_876;

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
async fn native_solana_signing_decodes_to_the_approved_transfer() {
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
    let signed_transaction = with_execution_context(None, secret, None, async {
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

    assert_signed_wire_matches_quote(&signed_transaction.raw, &sender, RECIPIENT, LAMPORTS);
    broker_task.abort();
}

fn assert_signed_wire_matches_quote(raw: &str, sender: &str, recipient: &str, lamports: u64) {
    let wire = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .unwrap();
    let mut offset = 0;
    assert_eq!(read_shortvec(&wire, &mut offset), 1, "one signature slot");
    offset += 64;
    let message = &wire[offset..];
    assert_eq!(&message[..3], &[1, 0, 1], "legacy transfer header");
    let mut message_offset = 3;
    assert_eq!(read_shortvec(message, &mut message_offset), 3);
    let keys_start = message_offset;
    let keys_end = keys_start + 3 * 32;
    let keys = &message[keys_start..keys_end];
    assert_eq!(
        &keys[..32],
        &tinywallet::address::solana::decode(sender).unwrap(),
        "payer matches the approved sender"
    );
    assert_eq!(
        &keys[32..64],
        &tinywallet::address::solana::decode(recipient).unwrap(),
        "recipient matches the approved transfer"
    );
    assert_eq!(
        &message[keys_end..keys_end + 32],
        &tinywallet::address::solana::decode(BLOCKHASH).unwrap(),
        "blockhash matches the typed transaction"
    );
    message_offset = keys_end + 32;
    assert_eq!(read_shortvec(message, &mut message_offset), 1);
    assert_eq!(message[message_offset], 2, "system program account index");
    message_offset += 1;
    assert_eq!(read_shortvec(message, &mut message_offset), 2);
    assert_eq!(&message[message_offset..message_offset + 2], &[0, 1]);
    message_offset += 2;
    assert_eq!(read_shortvec(message, &mut message_offset), 12);
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
        lamports,
        "lamports match the approved quote"
    );
}
