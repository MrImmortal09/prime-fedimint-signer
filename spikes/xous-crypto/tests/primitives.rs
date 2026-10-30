//! Correctness checks for the Phase 0 spike primitives.
//!
//! These run on the host. The same code compiling for `armv7a-unknown-xous-elf` is checked by
//! `scripts/device-build.sh`; cross-checks against fedimint's own `tbs` crate come with the golden
//! vectors in Phase 1A.

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bls12_381::{G1Affine, G2Affine, G2Projective, Scalar};
use fps_spike_xous_crypto as spike;
use group::Curve;
use serde::{Deserialize, Serialize};

fn hex32(s: &str) -> [u8; 32] {
    hex64::<32>(s)
}

fn hex64<const N: usize>(s: &str) -> [u8; N] {
    assert_eq!(s.len(), N * 2, "bad hex length for {s}");
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex digit");
    }
    out
}

/// Vectors 0 to 4 of the official BIP-340 test vector file
/// (https://github.com/bitcoin/bips/blob/master/bip-0340/test-vectors.csv).
/// Fields: secret key (empty when the vector is verification-only), x-only public key, aux_rand,
/// message, signature.
const BIP340: [(&str, &str, &str, &str, &str); 5] = [
    (
        "0000000000000000000000000000000000000000000000000000000000000003",
        "F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9",
        "0000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000",
        "E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0",
    ),
    (
        "B7E151628AED2A6ABF7158809CF4F3C762E7160F38B4DA56A784D9045190CFEF",
        "DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659",
        "0000000000000000000000000000000000000000000000000000000000000001",
        "243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89",
        "6896BD60EEAE296DB48A229FF71DFE071BDE413E6D43F917DC8DCF8C78DE33418906D11AC976ABCCB20B091292BFF4EA897EFCB639EA871CFA95F6DE339E4B0A",
    ),
    (
        "C90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B14E5C9",
        "DD308AFEC5777E13121FA72B9CC1B7CC0139715309B086C960E18FD969774EB8",
        "C87AA53824B4D7AE2EB035A2B5BBBCCC080E76CDC6D1692C4B0B62D798E6D906",
        "7E2D58D8B3BCDF1ABADEC7829054F90DDA9805AAB56C77333024B9D0A508B75C",
        "5831AAEED7B44BB74E5EAB94BA9D4294C49BCF2A60728D8B4C200F50DD313C1BAB745879A5AD954A72C45A91C3A51D3C7ADEA98D82F8481E0E1E03674A6F3FB7",
    ),
    (
        "0B432B2677937381AEF05BB02A66ECD012773062CF3FA2549E44F58ED2401710",
        "25D1DFF95105F5253C4022F628A996AD3A0D95FBF21D468A1B33F8C160D8F517",
        "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
        "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
        "7EB0509757E246F19449885651611CB965ECC1A187DD51B64FDA1EDC9637D5EC97582B9CB13DB3933705B32BA982AF5AF25FD78881EBB32771FC5922EFC66EA3",
    ),
    (
        "",
        "D69C3509BB99E412E68B0FE8544E72837DFA30746D8BE2AA65975F29D22DC7B9",
        "",
        "4DF3C3F68FCC83B27E9D42C90431A72499F17875C81A599B566C9889B9696703",
        "00000000000000000000003B78CE563F89A0ED9414F5AA28AD0D96D6795F9C6376AFB1548AF603B3EB45C9F8207DEE1060CB71C04E80F593060B07D28308D7F4",
    ),
];

#[test]
fn schnorr_matches_bip340_vectors() {
    for (index, (secret, public, aux, message, signature)) in BIP340.iter().enumerate() {
        let public = hex32(public);
        let message = hex32(message);
        let signature = hex64::<64>(signature);
        if !secret.is_empty() {
            let produced = spike::schnorr_sign(&hex32(secret), &message, &hex32(aux))
                .expect("valid secret key");
            assert_eq!(produced, signature, "vector {index}: signature differs");
        }
        assert!(
            spike::schnorr_verify(&public, &message, &signature),
            "vector {index}: signature does not verify"
        );
    }
}

#[test]
fn schnorr_rejects_a_tampered_message_or_signature() {
    let (_, public, _, message, signature) = BIP340[1];
    let public = hex32(public);
    let mut message = hex32(message);
    let mut signature = hex64::<64>(signature);
    assert!(spike::schnorr_verify(&public, &message, &signature));

    message[0] ^= 1;
    assert!(!spike::schnorr_verify(&public, &message, &signature));
    message[0] ^= 1;
    signature[63] ^= 1;
    assert!(!spike::schnorr_verify(&public, &message, &signature));
}

#[test]
fn schnorr_refuses_an_invalid_secret_key() {
    assert!(spike::schnorr_sign(&[0u8; 32], &[1u8; 32], &[0u8; 32]).is_none());
    assert!(spike::schnorr_sign(&[0xffu8; 32], &[1u8; 32], &[0u8; 32]).is_none());
}

#[test]
fn hash_to_g1_is_deterministic_and_domain_separated() {
    let a = spike::hash_to_g1_sha256(b"nonce");
    assert_eq!(a, spike::hash_to_g1_sha256(b"nonce"));
    assert_ne!(a, spike::hash_to_g1_sha256(b"nonce2"));
    // The two fedimint variants must never collide on the same input.
    assert_ne!(a, spike::hash_to_g1_sha3(b"nonce"));
    assert_eq!(
        spike::hash_to_g1_sha3(b"nonce"),
        spike::hash_to_g1_sha3(b"nonce")
    );
}

#[test]
fn hash_to_g1_lands_in_the_prime_order_subgroup() {
    for i in 0u32..16 {
        for point in [
            spike::hash_to_g1_sha256(&i.to_be_bytes()),
            spike::hash_to_g1_sha3(&i.to_be_bytes()),
        ] {
            assert!(bool::from(point.is_on_curve()));
            assert!(bool::from(point.is_torsion_free()));
            assert!(!bool::from(point.is_identity()));
            // Compressed encoding round trips, as fedimint's consensus encoding requires.
            let restored = G1Affine::from_compressed(&point.to_compressed());
            assert_eq!(Option::<G1Affine>::from(restored), Some(point));
        }
    }
}

#[test]
fn bls_signatures_verify_and_reject_the_wrong_key() {
    assert!(spike::bls_sign_and_verify(42, b"denomination 2^20"));

    let msg = spike::hash_to_g1_sha256(b"note");
    let sig = (msg * Scalar::from(7u64)).to_affine();
    let right = (G2Projective::generator() * Scalar::from(7u64)).to_affine();
    let wrong = (G2Projective::generator() * Scalar::from(8u64)).to_affine();
    assert!(spike::bls_verify(&msg, &sig, &right));
    assert!(!spike::bls_verify(&msg, &sig, &wrong));
    assert!(!spike::bls_verify(
        &spike::hash_to_g1_sha256(b"other"),
        &sig,
        &right
    ));
}

#[test]
fn blind_signature_unblinds_to_a_valid_signature() {
    // The mint signs a blinded message; the client unblinds with the inverse blinding key.
    // This is the tbs flow the signer's change and release checks rely on.
    let mint_secret = Scalar::from(123_456_789u64);
    let mint_key: G2Affine = (G2Projective::generator() * mint_secret).to_affine();
    let message = spike::hash_to_g1_sha256(b"nonce public key");
    let blinding = Scalar::from(987_654_321u64);

    let blinded = spike::blind(&message, &blinding);
    assert_ne!(blinded, message);
    let blinded_signature = (blinded * mint_secret).to_affine();
    let signature = (blinded_signature * blinding.invert().unwrap()).to_affine();

    assert!(spike::bls_verify(&message, &signature, &mint_key));
    assert!(!spike::bls_verify(&message, &blinded_signature, &mint_key));
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Frame {
    #[serde(with = "serde_bytes")]
    data: Vec<u8>,
}

#[test]
fn cbor_round_trips_and_encodes_bytes_compactly() {
    let frame = Frame {
        data: vec![0xab; 32],
    };
    let (decoded, size) = spike::cbor_roundtrip(&frame).expect("round trip");
    assert_eq!(decoded, frame);
    // map(1) + text "data" + bytes(32): 1 + 5 + 2 + 32. A plain Vec<u8> would cost a byte or two
    // per element, which matters on an animated QR.
    assert_eq!(size, 40);
}

#[test]
fn bolt11_invoices_parse_with_their_signature_checked() {
    use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
    use std::time::Duration;

    let secp = Secp256k1::new();
    let node_key = SecretKey::from_slice(&[0x11; 32]).unwrap();
    let payment_hash = sha256::Hash::hash(b"preimage");
    let invoice = InvoiceBuilder::new(Currency::Regtest)
        .description("coffee".into())
        .payment_hash(payment_hash)
        .payment_secret(PaymentSecret([42u8; 32]))
        .duration_since_epoch(Duration::from_secs(1_700_000_000))
        .min_final_cltv_expiry_delta(144)
        .amount_milli_satoshis(250_000)
        .build_signed(|hash| secp.sign_ecdsa_recoverable(hash, &node_key))
        .expect("invoice builds")
        .to_string();

    let (hash, amount) = spike::bolt11_summary(&invoice).expect("parses");
    assert_eq!(hash, payment_hash.to_byte_array());
    assert_eq!(amount, Some(250_000));

    // A single changed character breaks the bech32 checksum or the signature.
    let mut corrupted = invoice.into_bytes();
    let last = corrupted.len() - 8;
    corrupted[last] = if corrupted[last] == b'q' { b'p' } else { b'q' };
    assert!(spike::bolt11_summary(std::str::from_utf8(&corrupted).unwrap()).is_none());
    assert!(spike::bolt11_summary("not an invoice").is_none());
}
