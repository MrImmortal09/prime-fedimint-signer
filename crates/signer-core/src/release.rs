//! Releasing outgoing-branch notes as a mint v2 ecash token.

use std::collections::HashSet;

use fps_protocol::{Delivery, NoteRef, RejectCode, ReleaseRequest};

use crate::analysis::Ledger;
use crate::error::{SignerError, SignerResult};
use crate::federation::Federation;
use crate::fedimint::ecash::{EcashToken, Invite, SpendableNote};
use crate::fedimint::mintv2::denomination_amount_msat;
use crate::fedimint::primitives::g1_from_bytes;
use crate::keys::FederationKeys;
use crate::review::{format_msat, Review};
use crate::tbs;
use fps_protocol::Branch;

/// Most notes in one token.
pub const MAX_RELEASE_NOTES: usize = 64;

/// A checked release, ready to be approved.
pub struct ReleasePlan {
    pub token: EcashToken,
    pub amount_msat: u64,
    pub notes: Vec<NoteRef>,
    pub delivery: Delivery,
    pub review: Review,
}

/// Checks a release request and builds the token.
///
/// Only outgoing-branch keys are ever released (decision D11). Each note's mint signature is
/// checked unless `verify_signatures` is off: an invalid signature means the receiver could not
/// redeem, and releasing the key would gain nothing.
pub fn plan_release(
    federation: &Federation,
    keys: &FederationKeys,
    request: &ReleaseRequest,
    ledger: &dyn Ledger,
    verify_signatures: bool,
) -> SignerResult<ReleasePlan> {
    if request.federation.0 != federation.id || keys.federation() != federation.id {
        return Err(SignerError::new(
            RejectCode::UnknownFederation,
            "the request is for a different federation",
        ));
    }
    if request.notes.is_empty() {
        return Err(SignerError::new(
            RejectCode::InvalidTransaction,
            "there are no notes to release",
        ));
    }
    if request.notes.len() > MAX_RELEASE_NOTES {
        return Err(SignerError::new(
            RejectCode::TooLarge,
            format!("at most {MAX_RELEASE_NOTES} notes per token"),
        ));
    }

    let mut seen = HashSet::new();
    let mut notes = Vec::with_capacity(request.notes.len());
    let mut refs = Vec::with_capacity(request.notes.len());
    let mut amount_msat = 0u64;
    for (index, note) in request.notes.iter().enumerate() {
        let denomination = note.denomination;
        let tweak = note.tweak.0;
        let (Some(amount), Some(mint_key)) = (
            denomination_amount_msat(denomination),
            federation.mint.keys.get(&denomination),
        ) else {
            return Err(SignerError::new(
                RejectCode::InvalidNote,
                format!("note {index}: this mint does not issue denomination 2^{denomination}"),
            ));
        };
        if !seen.insert((denomination, tweak)) {
            return Err(SignerError::new(
                RejectCode::InvalidTransaction,
                format!("note {index} appears twice"),
            ));
        }
        if ledger.is_released(&federation.id, denomination, &tweak) {
            return Err(SignerError::new(
                RejectCode::AlreadyReleased,
                format!("note {index} was already released"),
            ));
        }
        let signature = g1_from_bytes(&note.signature.0).ok_or_else(|| {
            SignerError::new(
                RejectCode::InvalidNote,
                format!("note {index}: the signature is not a valid G1 point"),
            )
        })?;
        let keypair = keys.note_keypair(Branch::Outgoing, denomination, &tweak)?;
        if verify_signatures
            && !tbs::verify(
                &tbs::nonce_message(&keypair.public_key()),
                &signature,
                mint_key,
            )
        {
            return Err(SignerError::new(
                RejectCode::InvalidNote,
                format!("note {index}: the mint signature does not verify"),
            ));
        }
        amount_msat = amount_msat
            .checked_add(amount)
            .ok_or_else(|| SignerError::new(RejectCode::InvalidTransaction, "amounts overflow"))?;
        notes.push(SpendableNote {
            denomination,
            secret_key: keypair.secret_bytes(),
            signature,
        });
        refs.push(NoteRef {
            denomination,
            tweak: note.tweak,
        });
    }

    let invite = if request.include_invite {
        // The endpoints are authenticated by the federation id, so the URL comes from them rather
        // than from the phone. Guardians are sorted; take the first.
        let guardian = federation
            .guardians
            .first()
            .expect("an imported federation has at least one guardian");
        Some(Invite {
            peer: guardian.peer,
            url: guardian.url.clone(),
        })
    } else {
        None
    };

    let mut review = Review::new("Send ecash", amount_msat)
        .line("Amount", format_msat(amount_msat))
        .line("Notes", notes.len().to_string())
        .line(
            "Delivery",
            match request.delivery {
                Delivery::ShowOnDevice => "Shown on this screen",
                Delivery::ReturnToPhone => "Sent to your phone",
            },
        )
        .line("Federation", federation.display_name());
    if request.delivery == Delivery::ReturnToPhone {
        review = review.warn("Your phone will hold this ecash until the receiver redeems it.");
    }

    Ok(ReleasePlan {
        token: EcashToken {
            federation: federation.id,
            notes,
            invite,
        },
        amount_msat,
        notes: refs,
        delivery: request.delivery,
        review,
    })
}
