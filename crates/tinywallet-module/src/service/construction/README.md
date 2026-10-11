# EVM construction and approval facts

ConstructEvmTransaction is a stateless, one-argument module operation. It accepts
public account metadata, an explicit native/ERC-20/contract action and resolved
network facts. It generates the TransactionSpec, signing payload and exact facts
the host reviews before signing. No request carries a phrase, private key,
approval flag or policy; unknown fields are rejected.

The host owns RPC configuration/resolution, credential custody and approval.
Before signing, it compares the returned sender with its intended account and
approves the returned network, nonce, recipient, token/amount, calldata and
maximum native debit. It passes the exact returned TransactionSpec through the
existing confidential SignTransaction operation, or the split signing flow.
The constructor never signs, broadcasts, reserves funds or stores a handle.
Lost replies/retries therefore produce identical public results without side
effects or orphaned resources. A host changing returned fields must obtain a new
construction result and approval for those fields.

The canonical serialized DTO budget is 64 KiB; a counting writer rejects
serialization before allocating a request copy. This check runs after TinyBus
decoding and does not limit the original frame, including JSON whitespace and
escape spelling; transport admission remains TinyBus's responsibility. Calldata is at most 16 KiB. Recipient/public
key, decimal fields and generated calldata have explicit or validation-implied
bounds, so repeated transaction/fact fields cannot produce unbounded output.
Gas multiplication and maximum native debit use checked u128 arithmetic,
matching the existing EVM legacy transaction implementation. ERC-20 amounts
retain full uint256 range through the owning ABI implementation. Facts show the
canonical decimal amounts and exact calldata that the payload commits to.
The constructor derives all approval fields from the validated intent and exact
transaction it returns; the contract DTO alone performs no validation when
deserializing arbitrary input. Hosts must trust only an admitted, compatible
module and approve the exact transaction subsequently sent for signing.

This constructor remains stateless and separate from the module-owned Web3
quote/execution and x402 payment/budget/ledger operations added in contract 1.3,
with explicit service lifecycles added in contract 1.4 and the native-wallet
execution surface added in contract 1.5.
Host adapters, root pins and release digest adoption remain gated on
independently reviewed compatible published artifacts.
