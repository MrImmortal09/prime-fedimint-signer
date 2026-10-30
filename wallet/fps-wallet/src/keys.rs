//! The client's keys as Prime's view material describes them, and the signer that asks Prime.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

use async_trait::async_trait;
use bitcoin::hashes::{Hash as _, sha256};
use fedimint_client_module::transaction::{
    RemoteKey, SigningRequest, TransactionSigner, TransactionSignerError,
};
use fedimint_core::config::ClientConfig;
use fedimint_core::encoding::Encodable as _;
use fedimint_core::secp256k1::schnorr::Signature;
use fedimint_core::secp256k1::{Keypair, PublicKey, SECP256K1, rand};
use fedimint_lnv2_client::{LightningOperationMeta, RefundKeys};
use fedimint_lnv2_common::LightningInvoice;
use fedimint_lnv2_common::contracts::OutgoingContract;
use fedimint_mintv2_client::{MintOperationMeta, NoteKeyProvider, NoteKeys, NoteSpendKey};
use fedimint_mintv2_common::Denomination;
use fps_protocol::{
    Branch, FederationId, FixedBytes, InputHint, Intent, Request, Response, SignRequest,
};
use fps_signer_core::ViewKeys;

use crate::descriptor::federation_descriptor;
use crate::device::{Device, DeviceError};

/// The keys of one federation, with Prime holding every secret one: the mint module's note keys,
/// the Lightning module's refund keys, and the signer for the client's transactions.
pub struct DeviceKeys {
    device: Device,
    federation: FederationId,
    view: ViewKeys,
    /// Gateway authorisations Prime returned with payments it signed, by contract refund key.
    gateway_auths: Mutex<BTreeMap<[u8; 33], [u8; 64]>>,
}

impl fmt::Debug for DeviceKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The view secret reveals every note of the wallet; keep it out of logs.
        f.debug_struct("DeviceKeys")
            .field("federation", &self.federation)
            .finish_non_exhaustive()
    }
}

/// Why pairing a federation with Prime failed.
#[derive(Debug, thiserror::Error)]
pub enum PairError {
    #[error("the federation config cannot be described: {0}")]
    Config(String),
    #[error(transparent)]
    Device(#[from] DeviceError),
    #[error("the view material is invalid: {0}")]
    ViewMaterial(String),
}

impl DeviceKeys {
    /// Pairs the federation `config` describes with Prime: the person at the device approves the
    /// federation, then the export of its view material.
    pub async fn pair(device: Device, config: &ClientConfig) -> Result<Self, PairError> {
        let descriptor = federation_descriptor(config).map_err(PairError::Config)?;
        let federation = descriptor.federation;

        match device
            .request(Request::ImportFederation(descriptor))
            .await?
        {
            Response::FederationImported(_) => {}
            other => return Err(DeviceError::Unexpected(format!("{other:?}")).into()),
        }

        let material = match device
            .request(Request::ExportViewMaterial { federation })
            .await?
        {
            Response::ViewMaterial(material) => material,
            other => return Err(DeviceError::Unexpected(format!("{other:?}")).into()),
        };
        if material.federation != federation || material.key_scheme != fps_protocol::KEY_SCHEME {
            return Err(PairError::ViewMaterial(format!(
                "for another federation or key scheme ({})",
                material.key_scheme
            )));
        }
        let spend_key = PublicKey::from_slice(&material.spend_key.0)
            .map_err(|error| PairError::ViewMaterial(error.to_string()))?;
        let view = ViewKeys::new(spend_key, material.chain_code.0, material.view_secret.0);

        Ok(Self::new(device, federation, view))
    }

    /// Keys for a federation already paired, from its stored view material.
    pub fn new(device: Device, federation: FederationId, view: ViewKeys) -> Self {
        Self {
            device,
            federation,
            view,
            gateway_auths: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn federation(&self) -> FederationId {
        self.federation
    }

    pub fn view(&self) -> &ViewKeys {
        &self.view
    }

    /// The intent Prime checks the transaction against, from what the operation is and from the
    /// inputs.
    fn intent(request: &SigningRequest, hints: &[InputHint]) -> Result<Intent, String> {
        if hints
            .iter()
            .any(|hint| matches!(hint, InputHint::Refund { .. }))
        {
            return Ok(Intent::Refund);
        }
        // A transaction a state machine submits on its own spends no notes of the wallet's
        // (receives are signed by the phone alone), so it can only be a self-transfer.
        let Some(operation) = &request.operation else {
            return Ok(Intent::Reissue);
        };
        match operation.operation_type.as_str() {
            "lnv2" => match serde_json::from_value(operation.meta.clone()) {
                Ok(LightningOperationMeta::Send(send)) => match send.invoice {
                    LightningInvoice::Bolt11(invoice) => Ok(Intent::PayInvoice {
                        invoice: invoice.to_string(),
                    }),
                },
                Ok(_) => Err("only Lightning payments spend from the wallet".to_owned()),
                Err(error) => Err(format!("unreadable Lightning operation: {error}")),
            },
            "walletv2" => Ok(Intent::Withdraw),
            "mintv2" => match serde_json::from_value(operation.meta.clone()) {
                Ok(MintOperationMeta::IssueExternal { amount, .. }) => Ok(Intent::PrepareSend {
                    amount_msat: amount.msats,
                }),
                Ok(MintOperationMeta::ClaimExternal { .. }) => Ok(Intent::Reclaim),
                Ok(_) => Ok(Intent::Reissue),
                Err(error) => Err(format!("unreadable mint operation: {error}")),
            },
            other => Err(format!("cannot sign for a {other} operation")),
        }
    }

    /// Remembers the gateway authorisation Prime returned with a payment it signed.
    fn remember_gateway_auth(&self, request: &SigningRequest, auth: [u8; 64]) {
        let Some(operation) = &request.operation else {
            return;
        };
        if let Ok(LightningOperationMeta::Send(send)) =
            serde_json::from_value::<LightningOperationMeta>(operation.meta.clone())
        {
            self.gateway_auths
                .lock()
                .expect("the lock is never poisoned")
                .insert(send.contract.refund_pk.serialize(), auth);
        }
    }
}

/// The reference Prime finds a key by: the input hint of the wire protocol, in CBOR.
fn key_ref(hint: &InputHint) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::into_writer(hint, &mut bytes).expect("serialising to a Vec cannot fail");
    bytes
}

/// A spend-branch or outgoing-branch note key, as Prime finds it.
pub(crate) fn note_key(nonce: PublicKey, tweak: [u8; 16], branch: Branch) -> RemoteKey {
    RemoteKey {
        public_key: nonce,
        key_ref: key_ref(&InputHint::Note {
            tweak: FixedBytes(tweak),
            branch,
        }),
    }
}

impl NoteKeyProvider for DeviceKeys {
    fn tweak_filter(&self) -> [u8; 32] {
        self.view.tweak_filter()
    }

    fn note_keys(&self, denomination: Denomination, tweak: [u8; 16]) -> NoteKeys {
        let nonce = self
            .view
            .note_nonce(denomination.0, &tweak)
            .expect("a tweak's point is never the negation of the spend key");
        NoteKeys {
            spend_key: NoteSpendKey::Remote(note_key(nonce, tweak, Branch::Spend)),
            blinding_key: tbs::BlindingKey(self.view.blinding_key(
                Branch::Spend,
                denomination.0,
                &tweak,
            )),
        }
    }

    fn holds_spend_keys(&self) -> bool {
        false
    }
}

#[async_trait]
impl RefundKeys for DeviceKeys {
    fn new_refund_key(&self) -> (PublicKey, PublicKey) {
        loop {
            // Only the point matters: Prime derives the refund key from it.
            let ephemeral = Keypair::new(SECP256K1, &mut rand::thread_rng()).public_key();
            if let Ok(refund) = self.view.refund_key(&ephemeral) {
                return (ephemeral, refund);
            }
        }
    }

    fn remote_key(&self, contract: &OutgoingContract) -> RemoteKey {
        RemoteKey {
            public_key: contract.refund_pk,
            key_ref: key_ref(&InputHint::Refund {
                contract: contract.consensus_encode_to_vec().into(),
            }),
        }
    }

    async fn gateway_auth(
        &self,
        contract: &OutgoingContract,
        _invoice: &LightningInvoice,
    ) -> Result<Signature, String> {
        let auth = self
            .gateway_auths
            .lock()
            .expect("the lock is never poisoned")
            .get(&contract.refund_pk.serialize())
            .copied()
            .ok_or("Prime has not authorised a payment for this contract")?;
        Signature::from_slice(&auth).map_err(|error| error.to_string())
    }
}

#[async_trait]
impl TransactionSigner for DeviceKeys {
    async fn sign(
        &self,
        request: &SigningRequest,
    ) -> Result<Vec<Signature>, TransactionSignerError> {
        let refuse = |reason: String| TransactionSignerError::Declined(reason);

        // Exactly the bytes fedimint hashes into the transaction id.
        let transaction = &request.transaction;
        let mut bytes = transaction.inputs.consensus_encode_to_vec();
        bytes.extend(transaction.outputs.consensus_encode_to_vec());
        bytes.extend(transaction.nonce.consensus_encode_to_vec());
        let txid = request.txid().to_byte_array();
        if sha256::Hash::hash(&bytes).to_byte_array() != txid {
            return Err(refuse("the transaction does not hash to its id".to_owned()));
        }

        // Prime signs only transactions whose every input is its own.
        let hints = request
            .inputs
            .iter()
            .enumerate()
            .map(|(index, input)| match input.keys.as_slice() {
                [key] => ciborium::from_reader::<InputHint, _>(key.key_ref.as_slice())
                    .map_err(|error| format!("input {index}: unreadable key reference: {error}")),
                _ => Err(format!(
                    "input {index} has {} keys on the device; Prime signs transactions whose every input is its own",
                    input.keys.len()
                )),
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(refuse)?;
        let intent = Self::intent(request, &hints).map_err(refuse)?;

        let response = self
            .device
            .request(Request::SignTransaction(SignRequest {
                federation: self.federation,
                transaction: bytes.into(),
                inputs: hints,
                intent,
            }))
            .await
            .map_err(|error| match error {
                DeviceError::Rejected { code, message } => {
                    TransactionSignerError::Declined(format!("{code:?}: {message}"))
                }
                other => TransactionSignerError::Unavailable(other.to_string()),
            })?;
        let Response::Signed(signed) = response else {
            return Err(TransactionSignerError::Unavailable(format!(
                "unexpected answer: {response:?}"
            )));
        };
        if signed.txid.0 != txid {
            return Err(TransactionSignerError::Unavailable(
                "Prime signed another transaction".to_owned(),
            ));
        }
        if let Some(auth) = signed.gateway_auth {
            self.remember_gateway_auth(request, auth.0);
        }
        signed
            .signatures
            .iter()
            .map(|signature| {
                Signature::from_slice(&signature.0)
                    .map_err(|error| TransactionSignerError::Unavailable(error.to_string()))
            })
            .collect()
    }
}
