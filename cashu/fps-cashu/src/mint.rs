//! Mints: their canonical URLs, the checks a mint must pass before the wallet uses it, and the
//! client the flows talk to it through (CDK's [`HttpClient`]; only the `cashu` request types go
//! through it, the flows that build them are this crate's own).

use std::str::FromStr;

use cashu::nuts::nut23::{
    MeltQuoteBolt11Request, MeltQuoteBolt11Response, MintQuoteBolt11Response,
};
use cashu::nuts::{
    BlindedMessage, CheckStateRequest, CurrencyUnit, Id, KeySet, KeySetInfo, MeltRequest, MintInfo,
    MintQuoteBolt11Request, MintRequest, MintResponse, MintVersion, PaymentMethod, ProofState,
    PublicKey, RestoreRequest, RestoreResponse, SwapRequest, SwapResponse,
};
use cashu::{Amount, Bolt11Invoice, MintUrl};
use cdk::wallet::{HttpClient, MintConnector};
use cdk::{MeltQuoteCreateResponse, MeltQuoteRequest, MeltQuoteResponse};
use cdk::{MintQuoteRequest, MintQuoteResponse};
use fps_cashu_core::keyset::{MAX_KEYSETS, UNIT};
use fps_protocol::{ByteBuf, CashuKey, CashuKeyset, CashuMintDescriptor, FixedBytes};

use crate::CashuError;

/// The canonical form of a mint URL: CDK's `MintUrl` (lower-case scheme and host, no trailing
/// `/`), which Prime derives the mint's keys from and accepts in no other form. Also refuses what
/// Prime refuses: anything but `https` or `http`, user info, a query or a fragment, more than 256
/// bytes, or characters that are not printable ASCII.
pub fn canonical_url(text: &str) -> Result<String, CashuError> {
    let text = text.trim();
    let url = fps_cashu_core::url::canonicalize(text)
        .ok_or_else(|| CashuError::invalid(format!("{text:?} is not a mint URL")))?;
    fps_cashu_core::url::check_mint_url(&url)
        .map_err(|error| CashuError::invalid(format!("{text:?}: {}", error.message)))?;
    Ok(url)
}

/// The NUTs the wallet relies on, and what for.
const REQUIRED_NUTS: [(&str, &str); 6] = [
    ("07", "proof states"),
    ("08", "fee return"),
    ("09", "restore"),
    ("10", "spending conditions"),
    ("11", "P2PK locks"),
    ("12", "DLEQ proofs"),
];

/// Refuses a mint that lacks NUT-07, 08, 09, 10, 11 or 12, or whose version names a release that
/// signs the old `SIG_ALL` message (Nutshell 0.20.2 or earlier, CDK 0.13 or earlier): such a mint
/// would treat the wallet's locks differently from Prime.
pub fn check_mint_info(info: &MintInfo) -> Result<(), CashuError> {
    let nuts = &info.nuts;
    let supported = [
        nuts.nut07.supported,
        nuts.nut08.supported,
        nuts.nut09.supported,
        nuts.nut10.supported,
        nuts.nut11.supported,
        nuts.nut12.supported,
    ];
    let missing: Vec<String> = REQUIRED_NUTS
        .iter()
        .zip(supported)
        .filter(|(_, supported)| !supported)
        .map(|((nut, what), _)| format!("NUT-{nut} ({what})"))
        .collect();
    if !missing.is_empty() {
        return Err(CashuError::invalid(format!(
            "the mint does not support {}, which this wallet needs",
            missing.join(", ")
        )));
    }
    if let Some(reason) = info.version.as_ref().and_then(old_sig_all) {
        return Err(CashuError::invalid(reason));
    }
    Ok(())
}

/// Why a mint's version rules it out, if it does: a release that signs the `SIG_ALL` message as
/// NUT-11 had it before November 2025, or a version of those implementations that cannot be read.
fn old_sig_all(version: &MintVersion) -> Option<String> {
    let name = version.name.to_ascii_lowercase();
    let (implementation, last_old): (&str, &[u64]) = if name.starts_with("nutshell") {
        ("Nutshell", &[0, 20, 2])
    } else if name.starts_with("cdk") {
        ("CDK", &[0, 13, u64::MAX])
    } else {
        return None;
    };
    let numbers = version_numbers(&version.version);
    if numbers.is_empty() {
        return Some(format!(
            "the mint runs {version}, whose version cannot be read, so it may sign the old SIG_ALL message"
        ));
    }
    let ordering = numbers
        .iter()
        .chain(std::iter::repeat(&0))
        .zip(last_old)
        .map(|(number, last)| number.cmp(last))
        .find(|ordering| ordering.is_ne());
    let old = ordering.is_none_or(std::cmp::Ordering::is_lt);
    old.then(|| {
        format!(
            "the mint runs {version}, a {implementation} release that signs the old SIG_ALL message; it needs to be updated"
        )
    })
}

/// The numbers a version begins with: `0.20.3.1` gives `[0, 20, 3, 1]`, `v0.18.1-rc2` gives
/// `[0, 18, 1]`.
fn version_numbers(version: &str) -> Vec<u64> {
    let version = version.trim();
    let version = version.strip_prefix(['v', 'V']).unwrap_or(version);
    let mut numbers = Vec::new();
    for part in version.split('.') {
        let digits = part.bytes().take_while(u8::is_ascii_digit).count();
        let Some(Ok(number)) = part.get(..digits).map(str::parse::<u64>) else {
            break;
        };
        numbers.push(number);
        if digits != part.len() {
            break;
        }
    }
    numbers
}

/// One mint, over HTTPS (or HTTP).
pub(crate) struct MintClient {
    url: String,
    connector: HttpClient,
}

fn failed(url: &str, what: &str, error: impl std::fmt::Display) -> CashuError {
    CashuError::mint(format!("{what} at the mint {url} failed: {error}"))
}

impl MintClient {
    /// A client for the mint at `url`, a canonical URL.
    pub(crate) fn new(url: &str) -> Result<Self, CashuError> {
        let mint_url = MintUrl::from_str(url)
            .map_err(|_| CashuError::invalid(format!("{url:?} is not a mint URL")))?;
        crate::tls::install_tls();
        Ok(Self {
            url: url.to_owned(),
            connector: HttpClient::new(mint_url, None),
        })
    }

    pub(crate) async fn info(&self) -> Result<MintInfo, CashuError> {
        self.connector
            .get_mint_info()
            .await
            .map_err(|error| failed(&self.url, "Reading the mint's info", error))
    }

    /// Every keyset the mint lists, of every unit.
    pub(crate) async fn keysets(&self) -> Result<Vec<KeySetInfo>, CashuError> {
        Ok(self
            .connector
            .get_mint_keysets()
            .await
            .map_err(|error| failed(&self.url, "Reading the keysets", error))?
            .keysets)
    }

    async fn keys(&self, id: Id) -> Result<KeySet, CashuError> {
        let keyset = self
            .connector
            .get_mint_keyset(id)
            .await
            .map_err(|error| failed(&self.url, "Reading a keyset's keys", error))?;
        if keyset.id != id {
            return Err(CashuError::mint(format!(
                "the mint {} answered keyset {} when asked for {id}",
                self.url, keyset.id
            )));
        }
        Ok(keyset)
    }

    /// The mint as Prime pins it: its `sat` keysets with their keys, active ones first. Inactive
    /// keysets are left out if, with them, the descriptor would not validate.
    pub(crate) async fn descriptor(&self) -> Result<CashuMintDescriptor, CashuError> {
        let mut listed: Vec<KeySetInfo> = self
            .keysets()
            .await?
            .into_iter()
            .filter(|keyset| keyset.unit == CurrencyUnit::Sat)
            .collect();
        listed.sort_by_key(|keyset| !keyset.active);
        listed.truncate(MAX_KEYSETS);
        if !listed.iter().any(|keyset| keyset.active) {
            return Err(CashuError::invalid(format!(
                "the mint {} has no active keyset in sats",
                self.url
            )));
        }
        let mut keysets = Vec::with_capacity(listed.len());
        for info in &listed {
            let keyset = self.keys(info.id).await?;
            keysets.push(CashuKeyset {
                id: ByteBuf(info.id.to_bytes()),
                unit: UNIT.to_owned(),
                input_fee_ppk: info.input_fee_ppk,
                final_expiry: info.final_expiry,
                active: info.active,
                keys: keyset
                    .keys
                    .iter()
                    .map(|(amount, key)| CashuKey {
                        amount: u64::from(*amount),
                        pubkey: FixedBytes(key.to_bytes()),
                    })
                    .collect(),
            });
        }
        let mut descriptor = CashuMintDescriptor {
            url: self.url.clone(),
            unit: UNIT.to_owned(),
            keysets,
        };
        if fps_cashu_core::validate_mint(&descriptor, None).is_err() {
            descriptor.keysets.retain(|keyset| keyset.active);
        }
        fps_cashu_core::validate_mint(&descriptor, None).map_err(|error| {
            CashuError::invalid(format!(
                "the mint {}'s keys cannot be pinned: {}",
                self.url, error.message
            ))
        })?;
        Ok(descriptor)
    }

    /// A Lightning invoice to mint `amount` sats.
    pub(crate) async fn mint_quote(
        &self,
        amount: u64,
    ) -> Result<MintQuoteBolt11Response<String>, CashuError> {
        let request = MintQuoteRequest::Bolt11(MintQuoteBolt11Request {
            amount: Amount::from(amount),
            unit: CurrencyUnit::Sat,
            description: None,
            pubkey: None,
        });
        match self.connector.post_mint_quote(request).await {
            Ok(MintQuoteResponse::Bolt11(quote)) => Ok(quote),
            Ok(_) => Err(CashuError::mint(format!(
                "the mint {} answered a Lightning quote request with another kind of quote",
                self.url
            ))),
            Err(error) => Err(failed(&self.url, "Asking for an invoice", error)),
        }
    }

    pub(crate) async fn mint_quote_state(
        &self,
        quote: &str,
    ) -> Result<MintQuoteBolt11Response<String>, CashuError> {
        match self
            .connector
            .get_mint_quote_status(PaymentMethod::BOLT11, quote)
            .await
        {
            Ok(MintQuoteResponse::Bolt11(state)) => Ok(state),
            Ok(_) => Err(CashuError::mint(format!(
                "the mint {} answered with another kind of quote",
                self.url
            ))),
            Err(error) => Err(failed(&self.url, "Reading the quote", error)),
        }
    }

    pub(crate) async fn mint(
        &self,
        request: MintRequest<String>,
    ) -> Result<MintResponse, CashuError> {
        self.connector
            .post_mint(&PaymentMethod::BOLT11, request)
            .await
            .map_err(|error| failed(&self.url, "Minting", error))
    }

    /// A swap. The error is CDK's, so a caller can tell an input that is already spent.
    pub(crate) async fn swap(&self, request: SwapRequest) -> Result<SwapResponse, cdk::Error> {
        self.connector.post_swap(request).await
    }

    /// A quote to pay `invoice`.
    pub(crate) async fn melt_quote(
        &self,
        invoice: Bolt11Invoice,
    ) -> Result<MeltQuoteBolt11Response<String>, CashuError> {
        let request = MeltQuoteRequest::Bolt11(MeltQuoteBolt11Request {
            request: invoice,
            unit: CurrencyUnit::Sat,
            options: None,
        });
        match self.connector.post_melt_quote(request).await {
            Ok(MeltQuoteCreateResponse::Bolt11(quote)) => Ok(quote),
            Ok(_) => Err(CashuError::mint(format!(
                "the mint {} answered a Lightning payment quote with another kind of quote",
                self.url
            ))),
            Err(error) => Err(failed(&self.url, "Asking to pay the invoice", error)),
        }
    }

    pub(crate) async fn melt(
        &self,
        request: MeltRequest<String>,
    ) -> Result<MeltQuoteBolt11Response<String>, CashuError> {
        match self
            .connector
            .post_melt(&PaymentMethod::BOLT11, request)
            .await
        {
            Ok(MeltQuoteResponse::Bolt11(answer)) => Ok(answer),
            Ok(_) => Err(CashuError::mint(format!(
                "the mint {} answered a Lightning payment with another kind of quote",
                self.url
            ))),
            Err(error) => Err(failed(&self.url, "Paying the invoice", error)),
        }
    }

    /// NUT-09: the mint's signatures on any of `outputs` it has signed before.
    pub(crate) async fn restore(
        &self,
        outputs: Vec<BlindedMessage>,
    ) -> Result<RestoreResponse, CashuError> {
        self.connector
            .post_restore(RestoreRequest { outputs })
            .await
            .map_err(|error| failed(&self.url, "Restoring outputs", error))
    }

    /// NUT-07: whether the proofs with these `Y`s are spent.
    pub(crate) async fn states(&self, ys: Vec<PublicKey>) -> Result<Vec<ProofState>, CashuError> {
        Ok(self
            .connector
            .post_check_state(CheckStateRequest { ys })
            .await
            .map_err(|error| failed(&self.url, "Checking proof states", error))?
            .states)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(name: &str, version: &str) -> MintVersion {
        MintVersion::new(name.to_owned(), version.to_owned())
    }

    #[test]
    fn canonical_urls() {
        assert_eq!(
            canonical_url(" HTTPS://TestNut.Cashu.Space/ ").unwrap(),
            "https://testnut.cashu.space"
        );
        assert!(canonical_url("ftp://mint.example").is_err());
        assert!(canonical_url("not a url").is_err());
    }

    #[test]
    fn old_sig_all_releases_are_refused() {
        for (name, number) in [
            ("Nutshell", "0.20.2"),
            ("cdk-mintd", "0.13.4"),
            ("Nutshell", "dev"),
        ] {
            assert!(
                old_sig_all(&version(name, number)).is_some(),
                "{name}/{number}"
            );
        }
        for (name, number) in [
            ("Nutshell", "0.20.3"),
            ("cdk-mintd", "0.18.1"),
            ("other-mint", "0.1.0"),
        ] {
            assert!(
                old_sig_all(&version(name, number)).is_none(),
                "{name}/{number}"
            );
        }
    }
}
