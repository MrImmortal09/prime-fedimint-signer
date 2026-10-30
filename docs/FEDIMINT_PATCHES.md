# The fedimint patch series

fedimint's client signs every input with keypairs it holds. Passport Prime holds the spend keys,
so the phone's client needs a signer boundary. The changes live in
[`patches/fedimint/`](../patches/fedimint), applied to fedimint `v0.13.0-beta.1`, because a fork of a
public repository cannot be private. `scripts/fedimint-checkout.sh` clones the tag into
`third_party/fedimint` and applies the series; the `wallet/` workspace builds against that tree.

To change a patch, commit on the `fps-signer` branch in `third_party/fedimint` and run
`scripts/fedimint-checkout.sh --export`. Format the wallet with `cargo fmt -p fps-wallet`: `--all`
also reformats every path dependency, the fedimint checkout included.

## 0001: the client signs through an external signer

`fedimint-client-module`, `fedimint-client`.

- A module marks inputs whose keys live elsewhere with `ClientInputBundle::with_remote_keys`. A
  `RemoteKey` is the public key the federation checks plus a reference the signer derives the secret
  key from. The module that creates the input defines the reference's format.
- When a transaction has remote keys, the client builds it unsigned (`TransactionBuilder::build_unsigned`),
  computes the operation meta, and passes both to the `TransactionSigner` set with
  `ClientBuilder::with_transaction_signer`. The meta comes first so the signer can see what the
  transaction is for, such as the invoice of a Lightning payment.
- The client checks every returned signature against its key and the transaction id before it
  submits (`UnsignedTransaction::sign`).
- Transactions with local keys only take the same path as before.

## 0002: mint v2 notes with keys held elsewhere

`fedimint-mintv2-client`.

- A `NoteKeyProvider` supplies each note's spend key and blinding key. `MintClientInit` derives both
  from the module root secret as fedimint always has. `MintClientInitWithKeys` takes a provider
  instead; ours, `fps_wallet::DeviceKeys`, derives nonces and blinding keys from Prime's view
  material and marks every spend key as remote.
- The wallet stores `WalletNote`s, whose spend key is local or remote. Remote notes are spent
  through the client's signer.
- A wallet whose keys are elsewhere puts its own notes into a transaction only when the transaction
  needs funding. Receiving therefore stays signed by the phone alone, and Prime never sees a
  transaction that mixes its keys with a token's.
- Such a wallet refuses `send`, which would hand its notes out as bearer e-cash.
  `issue_external_notes` issues notes to nonces and blinding keys the caller supplies (Prime's
  outgoing branch) and returns their signatures without adding them to the wallet.
  `claim_external_notes` spends such notes back into the wallet (a reclaim).
- A rejected transaction returns remote notes straight to the wallet. Reclaiming them would need
  the signer from inside a state transition, and a rejected transaction spent none of its inputs.
- Recovery recognises notes through the provider's tweak filter and keys, so a new phone with the
  same view material finds every spend-branch note.

## 0003: LNv2 refund keys held elsewhere

`fedimint-lnv2-client`.

- `LightningClientInit::refund_keys` lets an external signer hold the refund keys of outgoing
  contracts. The provider picks each contract's ephemeral point and refund key. Ours derives the
  refund key from the view material, so Prime can re-derive it from the contract.
- The gateway requires the refund key's signature over the invoice hash before it pays. Prime
  returns that signature with the funding signatures (`SignedTransaction::gateway_auth`), and the
  provider hands it to the send state machine.
- A refund goes through the client's signer. If it is not signed yet, for example because Prime is
  out of reach, the state machine waits and tries again, from ten seconds doubling to an hour,
  instead of failing.

## Limitations

- The client database encodings of mint v2 notes and LNv2 send state machines change without a
  migration, so a wallet created by stock fedimint cannot be opened with the patched client.
  fedimint's own mint v2 database snapshot test fails for that reason. Upstreaming needs the
  migrations.
- Signing happens while the database transaction that funds the transaction is open, which can
  take as long as the person at the device does. A database conflict in that time retries the
  whole step, with a new nonce, so the person is asked again.
- The gateway authorisation is kept in memory. A payment interrupted by an app restart before the
  gateway accepted it needs a new authorisation, which Prime cannot give without a new request type
  (Phase 6).
- Notes staged on Prime's outgoing branch and not yet released are invisible to recovery, because
  only Prime can derive their nonces.
