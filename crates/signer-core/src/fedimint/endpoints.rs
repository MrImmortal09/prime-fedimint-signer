//! Guardian API endpoints and the federation id derived from them.

use bitcoin::hashes::{sha256, Hash};

use crate::encoding::{DecodeError, DecodeResult, Reader, Writer};

/// Most guardians accepted in a federation descriptor.
pub const MAX_GUARDIANS: u64 = 64;

/// One entry of `GlobalClientConfig::api_endpoints: BTreeMap<PeerId, PeerUrl>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guardian {
    pub peer: u16,
    /// `PeerUrl::url`, a `SafeUrl` encoded as its string form.
    pub url: String,
    /// `PeerUrl::name`.
    pub name: String,
}

/// `FederationId`: SHA-256 of the consensus-encoded endpoints map (`calculate_federation_id`).
pub fn federation_id(encoded_endpoints: &[u8]) -> [u8; 32] {
    sha256::Hash::hash(encoded_endpoints).to_byte_array()
}

/// Decodes the endpoints map, requiring strictly increasing peer ids as fedimint does.
pub fn decode_api_endpoints(bytes: &[u8]) -> DecodeResult<Vec<Guardian>> {
    let mut reader = Reader::new(bytes);
    // A peer id, an empty URL and an empty name take at least three bytes.
    let count = reader.read_len("api endpoints", 3, MAX_GUARDIANS)?;
    let mut guardians: Vec<Guardian> = Vec::with_capacity(count);
    for _ in 0..count {
        let peer = reader.read_u16("peer id")?;
        if guardians
            .last()
            .is_some_and(|previous| previous.peer >= peer)
        {
            return Err(DecodeError::Invalid {
                what: "api endpoints",
                reason: "peer ids are not strictly increasing".to_owned(),
            });
        }
        let url = reader.read_string("peer url")?.to_owned();
        let name = reader.read_string("peer name")?.to_owned();
        guardians.push(Guardian { peer, url, name });
    }
    reader.finish("api endpoints")?;
    Ok(guardians)
}

/// Encodes an endpoints map; guardians must be sorted by peer id.
pub fn encode_api_endpoints(guardians: &[Guardian]) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.write_bigsize(guardians.len() as u64);
    for guardian in guardians {
        writer.write_bigsize(u64::from(guardian.peer));
        writer.write_string(&guardian.url);
        writer.write_string(&guardian.name);
    }
    writer.into_bytes()
}
