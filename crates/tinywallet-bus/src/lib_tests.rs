//! Tests that pin the shared `TinyWallet` vocabulary and compatibility rule.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    BUS_NAME, CONFIDENTIAL_METHODS, CONTRACT_VERSION, HOST_INTERFACE, HOST_METHODS,
    HOST_OBJECT_PATH, METHODS, OBJECT_PATH, is_compatible, names,
};

#[test]
fn the_contract_accepts_its_own_version_and_newer_minors() {
    assert!(is_compatible(CONTRACT_VERSION));
    assert!(is_compatible((CONTRACT_VERSION.0, CONTRACT_VERSION.1 + 1)));
    assert!(!is_compatible((CONTRACT_VERSION.0 + 1, 0)));
    // A module older than the host cannot serve every member the host may call.
    assert!(!is_compatible((CONTRACT_VERSION.0, 0)) || CONTRACT_VERSION.1 == 0);
}

#[test]
fn the_identity_matches_the_path_the_module_serves() {
    // Both spellings appear in the module's `serve_at` / `request_name` pair,
    // and a host that guesses one of them gets `NameHasNoOwner` at call time.
    assert_eq!(BUS_NAME, "ai.tinyhumans.tinywallet.Wallet");
    assert_eq!(OBJECT_PATH, "/ai/tinyhumans/tinywallet/Wallet");
}

#[test]
fn the_method_list_is_sorted_complete_and_free_of_duplicates() {
    let mut sorted = METHODS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.as_slice(),
        METHODS,
        "sorted dispatch order, no repeats"
    );
    assert_eq!(METHODS.len(), 20);
    for member in [
        names::methods::ATTACH_SIGNATURE,
        names::methods::BUILD_UNSIGNED,
        names::methods::CONSTRUCT_EVM_TRANSACTION,
        names::methods::DERIVE_ACCOUNT,
        names::methods::EXPORT_KEY,
        names::methods::SIGN_MESSAGE,
        names::methods::SIGN_TRANSACTION,
        names::methods::VALIDATE_ADDRESS,
        names::methods::WEB3_ROUTES,
        names::methods::WEB3_SWAP_QUOTE,
        names::methods::WEB3_BRIDGE_QUOTE,
        names::methods::WEB3_DAPP_QUOTE,
        names::methods::WEB3_EXECUTE,
        names::methods::WEB3_CLOSE,
        names::methods::WEB3_CONFIGURE,
        names::methods::X402_PREPARE_PAYMENT,
        names::methods::X402_RECORD_PAYMENT,
        names::methods::X402_SPENDING_SUMMARY,
        names::methods::X402_CLOSE,
        names::methods::X402_CONFIGURE,
    ] {
        assert!(
            METHODS.contains(&member),
            "{member} is missing from METHODS"
        );
    }
}

#[test]
fn host_callbacks_are_the_used_typed_capabilities() {
    let mut sorted = HOST_METHODS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.as_slice(), HOST_METHODS);
    assert_eq!(HOST_METHODS.len(), 5);
    assert_eq!(HOST_INTERFACE, "ai.tinyhumans.tinywallet.Host");
    assert_eq!(HOST_OBJECT_PATH, "/ai/tinyhumans/tinywallet/Host");
    assert!(HOST_METHODS.contains(&names::host_methods::AUTHORIZE_WEB3_EXECUTION));
    assert!(HOST_METHODS.contains(&names::host_methods::AUTHORIZE_X402_PAYMENT));
    assert!(HOST_METHODS.contains(&names::host_methods::CHAIN_TRANSPORT));
    assert!(HOST_METHODS.contains(&names::host_methods::WALLET_ACCOUNTS));
    assert!(HOST_METHODS.contains(&names::host_methods::WALLET_BACKEND));
}

#[test]
fn every_confidential_member_is_a_member() {
    // The confidential set is a subset, not a parallel list: a name that
    // drifted out of `METHODS` would be a member nobody can call, advertised as
    // one that carries a recovery phrase.
    for member in CONFIDENTIAL_METHODS {
        assert!(METHODS.contains(member), "{member} is not a member");
    }
    // `BuildUnsigned` and `AttachSignature` carry no secret by construction —
    // that is the whole point of the two-round-trip flow.
    assert!(!CONFIDENTIAL_METHODS.contains(&names::methods::BUILD_UNSIGNED));
    assert!(!CONFIDENTIAL_METHODS.contains(&names::methods::ATTACH_SIGNATURE));
}
