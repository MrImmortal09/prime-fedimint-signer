//! Ecash sends through Prime.
//!
//! The wallet cannot hand notes out itself, because Prime holds their keys. A send first stages
//! notes on Prime's hardened outgoing branch, whose keys only Prime can derive and the only keys
//! it ever releases (`docs/KEY_SCHEME.md`, decision D11). Prime then releases them as a fedimint
//! token, on its own screen or to the phone. Notes the receiver never redeems come back to the
//! wallet with a reclaim, which Prime signs only for notes it released.

use fedimint_client_module::transaction::RemoteKey;
use fedimint_core::Amount;
use fedimint_core::core::OperationId;
use fedimint_core::secp256k1::{PublicKey, rand};
use fedimint_mintv2_client::{
    ExternalNoteRequest, ExternalNotesError, IssuedExternalNote, MintClientModule,
};
use fedimint_mintv2_common::Denomination;
use fedimint_mintv2_common::config::client_denominations;
use fps_protocol::{
    Branch, Delivery, FixedBytes, NoteRef, ReleaseNote, ReleaseRequest, Released, Request, Response,
};
use serde_json::Value;

use crate::DeviceKeys;
use crate::device::DeviceError;
use crate::keys::note_key;

/// Why a send step failed.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("{0}")]
    Amount(String),
    #[error(transparent)]
    Device(#[from] DeviceError),
    #[error(transparent)]
    Mint(#[from] ExternalNotesError),
}

/// Notes staged on Prime's outgoing branch by one operation.
#[derive(Debug, Clone)]
pub struct StagedNotes {
    pub operation_id: OperationId,
    pub notes: Vec<IssuedExternalNote>,
}

impl StagedNotes {
    pub fn amount(&self) -> Amount {
        self.notes
            .iter()
            .map(|note| note.denomination.amount())
            .sum()
    }
}

/// `amount` in mint v2 client denominations, largest first.
fn denominations(amount: Amount) -> Result<Vec<Denomination>, SendError> {
    let mut remaining = amount.msats;
    let mut denominations = Vec::new();
    for denomination in client_denominations().rev() {
        while remaining >= denomination.amount().msats {
            remaining -= denomination.amount().msats;
            denominations.push(denomination);
        }
    }
    if remaining != 0 || denominations.is_empty() {
        return Err(SendError::Amount(format!(
            "{amount} is not a positive multiple of the smallest denomination"
        )));
    }
    Ok(denominations)
}

impl DeviceKeys {
    /// A random tweak that passes the wallet's recovery filter.
    fn grind_tweak(&self) -> [u8; 16] {
        loop {
            let tweak: [u8; 16] = rand::random();
            if self.view().tweak_passes_filter(&tweak) {
                return tweak;
            }
        }
    }

    /// Stages `amount` on Prime's outgoing branch. Prime approves the transaction that issues the
    /// notes; they stay Prime's until it releases them.
    pub async fn prepare_send(
        &self,
        mint: &MintClientModule,
        amount: Amount,
    ) -> Result<StagedNotes, SendError> {
        let notes: Vec<NoteRef> = denominations(amount)?
            .into_iter()
            .map(|denomination| NoteRef {
                denomination: denomination.0,
                tweak: FixedBytes(self.grind_tweak()),
            })
            .collect();

        let nonces = match self
            .device()
            .request(Request::DeriveOutgoingNonces {
                federation: self.federation(),
                notes: notes.clone(),
            })
            .await?
        {
            Response::OutgoingNonces { nonces } if nonces.len() == notes.len() => nonces,
            other => return Err(DeviceError::Unexpected(format!("{other:?}")).into()),
        };

        let requests = notes
            .iter()
            .zip(nonces)
            .map(|(note, nonce)| {
                let nonce = PublicKey::from_slice(&nonce.0)
                    .map_err(|error| DeviceError::Malformed(error.to_string()))?;
                Ok(ExternalNoteRequest {
                    denomination: Denomination(note.denomination),
                    tweak: note.tweak.0,
                    nonce,
                    blinding_key: tbs::BlindingKey(self.view().blinding_key(
                        Branch::Outgoing,
                        note.denomination,
                        &note.tweak.0,
                    )),
                })
            })
            .collect::<Result<Vec<_>, DeviceError>>()?;

        let (operation_id, notes) = mint.issue_external_notes(requests, Value::Null).await?;
        Ok(StagedNotes {
            operation_id,
            notes,
        })
    }

    /// Releases staged notes as a fedimint token. With [`Delivery::ShowOnDevice`] the token only
    /// appears on Prime's screen; the answer carries the amount alone.
    pub async fn release(
        &self,
        notes: &[IssuedExternalNote],
        delivery: Delivery,
        include_invite: bool,
    ) -> Result<Released, SendError> {
        let request = ReleaseRequest {
            federation: self.federation(),
            notes: notes
                .iter()
                .map(|note| ReleaseNote {
                    denomination: note.denomination.0,
                    tweak: FixedBytes(note.tweak),
                    signature: FixedBytes(note.signature.0.to_compressed()),
                })
                .collect(),
            include_invite,
            delivery,
        };
        match self
            .device()
            .request(Request::ReleaseNotes(request))
            .await?
        {
            Response::Released(released) => Ok(released),
            other => Err(DeviceError::Unexpected(format!("{other:?}")).into()),
        }
    }

    /// Spends released notes that nobody redeemed back into the wallet. Await the outcome with
    /// `MintClientModule::await_final_receive_operation_state`.
    pub async fn reclaim(
        &self,
        mint: &MintClientModule,
        notes: &[IssuedExternalNote],
    ) -> Result<OperationId, SendError> {
        let notes = notes
            .iter()
            .map(|note| {
                let key: RemoteKey = note_key(note.nonce, note.tweak, Branch::Outgoing);
                (note.note(), key)
            })
            .collect();
        Ok(mint.claim_external_notes(notes, Value::Null).await?)
    }
}
