# Minimal wallet bus contract

The loadable module is the execution boundary. The bus crate owns serialized
identifiers, errors, transfer targets, method declarations and wire DTOs only.
It depends on serde and thiserror, with no implementation dependency under any
feature. Crypto re-exports those shared types and owns validation, hashing,
encoding and transaction verification. x402 and web3 retain their algorithms.

This reverses the old compatibility-shim edge: crypto depends on bus; bus never
depends on crypto or x402. Existing shared vocabulary and transaction JSON are
unchanged. Removed algorithm re-exports require the next minor package release;
legacy feature names are accepted but do not enable implementation code.

Contract 1.1 adds ValidateAddress with a typed request and result. Successful
validation returns the trimmed address and cannot establish ownership, balance
or funding. The Bitcoin sender flag applies the existing P2WPKH restriction.
Validation faults retain the existing error taxonomy and detail for product
presentation. They must never be copied into sanitized module-failure telemetry.
Existing members retain their arities, confidentiality and wire representations.

Hosts retain credential custody and approval policy. They may adopt this contract
only with a compatible published module artifact and verified digest. Contract
1.3 adds an optional typed Host interface for account facts, explicit Web3/x402
approval, chain transport, authenticated wallet-backend calls, and x402 workspace
budget configuration. The module now owns stateful swap/bridge/dapp quotes and
execution, x402 challenge parsing and payment signing, budget reservations, and
the payment ledger. EVM approval receives the exact transaction including the
RPC-derived nonce and gas fields; Solana approval receives the complete unsigned
transaction bytes. Secret material is carried only by confidential calls to an
attested wallet module. Existing 1.2 members retain their arities and wire forms.
