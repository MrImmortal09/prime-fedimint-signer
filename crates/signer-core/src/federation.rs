//! A federation as the device knows it: an imported descriptor, validated.

use std::collections::{BTreeMap, BTreeSet};

use bls12_381::G2Affine;
use fps_protocol::{FederationDescriptor, FederationSummary, Network, RejectCode};

use crate::error::{SignerError, SignerResult};
use crate::fedimint::endpoints::{decode_api_endpoints, federation_id, Guardian};
use crate::fedimint::primitives::g2_from_bytes;
use crate::fedimint::{lnv2, mintv2, module_fee_msat, walletv2};

/// Longest federation name shown on the device.
pub const MAX_NAME_CHARS: usize = 64;
/// Most module instances accepted in a descriptor.
pub const MAX_MODULES: usize = 32;

/// fedimint `FeeConsensus`: a per-item base fee plus parts per million of the item's amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeRule {
    pub base_msat: u64,
    pub ppm: u64,
}

impl FeeRule {
    pub fn fee_msat(&self, amount_msat: u64) -> u64 {
        module_fee_msat(self.base_msat, self.ppm, amount_msat)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleEntry {
    pub kind: String,
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintModule {
    pub instance: u16,
    /// Aggregate public key per denomination exponent.
    pub keys: BTreeMap<u8, G2Affine>,
    pub fee: FeeRule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightningModule {
    pub instance: u16,
    pub network: bitcoin::Network,
    pub fee: FeeRule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletModule {
    pub instance: u16,
    pub network: bitcoin::Network,
    pub dust_limit_sat: u64,
    pub fee: FeeRule,
}

/// What kind of module an instance is, for the ones the signer understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownModule {
    Mint,
    Lightning,
    Wallet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Federation {
    pub id: [u8; 32],
    pub name: Option<String>,
    pub guardians: Vec<Guardian>,
    pub modules: BTreeMap<u16, ModuleEntry>,
    pub mint: MintModule,
    pub lightning: Option<LightningModule>,
    pub wallet: Option<WalletModule>,
}

pub fn bitcoin_network(network: Network) -> bitcoin::Network {
    match network {
        Network::Bitcoin => bitcoin::Network::Bitcoin,
        Network::Testnet => bitcoin::Network::Testnet,
        Network::Testnet4 => bitcoin::Network::Testnet4,
        Network::Signet => bitcoin::Network::Signet,
        Network::Regtest => bitcoin::Network::Regtest,
    }
}

fn invalid(message: impl Into<String>) -> SignerError {
    SignerError::new(RejectCode::InvalidDescriptor, message)
}

fn fee_rule(base_msat: u64, ppm: u64, max_ppm: u64, module: &str) -> SignerResult<FeeRule> {
    if ppm > max_ppm {
        return Err(invalid(format!(
            "{module} fee of {ppm} ppm exceeds fedimint's limit of {max_ppm}"
        )));
    }
    Ok(FeeRule { base_msat, ppm })
}

/// Names come from federation meta and are not authenticated: keep them short and printable.
fn display_name(name: &Option<String>) -> SignerResult<Option<String>> {
    let Some(name) = name else { return Ok(None) };
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return Ok(None);
    }
    if cleaned.chars().count() > MAX_NAME_CHARS {
        return Err(invalid(format!(
            "federation name is longer than {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(Some(cleaned.to_owned()))
}

impl Federation {
    /// Validates a descriptor. Only the endpoints are authenticated, by hashing them to the
    /// federation id; everything else is trusted from the paired phone (decision D12).
    pub fn from_descriptor(descriptor: &FederationDescriptor) -> SignerResult<Self> {
        let id = descriptor.federation.0;
        if federation_id(descriptor.api_endpoints.as_slice()) != id {
            return Err(SignerError::new(
                RejectCode::FederationIdMismatch,
                "the guardian endpoints do not hash to this federation id",
            ));
        }
        let guardians = decode_api_endpoints(descriptor.api_endpoints.as_slice())
            .map_err(SignerError::invalid_descriptor)?;
        if guardians.is_empty() {
            return Err(invalid("a federation needs at least one guardian"));
        }

        if descriptor.modules.len() > MAX_MODULES {
            return Err(invalid(format!("more than {MAX_MODULES} modules")));
        }
        let mut modules = BTreeMap::new();
        for module in &descriptor.modules {
            let entry = ModuleEntry {
                kind: module.kind.clone(),
                major: module.major,
                minor: module.minor,
            };
            if modules.insert(module.instance, entry).is_some() {
                return Err(invalid(format!(
                    "module instance {} is listed twice",
                    module.instance
                )));
            }
        }
        let expect_kind = |instance: u16, kind: &str| -> SignerResult<()> {
            match modules.get(&instance) {
                Some(entry) if entry.kind == kind => Ok(()),
                Some(entry) => Err(invalid(format!(
                    "module instance {instance} is {}, not {kind}",
                    entry.kind
                ))),
                None => Err(invalid(format!(
                    "module instance {instance} is not in the module list"
                ))),
            }
        };

        let mint_params = descriptor.mint.as_ref().ok_or_else(|| {
            SignerError::new(
                RejectCode::Unsupported,
                "the federation has no mint v2 module",
            )
        })?;
        expect_kind(mint_params.instance, mintv2::KIND)?;
        if mint_params.amount_unit != 0 {
            return Err(SignerError::new(
                RejectCode::Unsupported,
                format!(
                    "mint amount unit {} is not bitcoin",
                    mint_params.amount_unit
                ),
            ));
        }
        let mut keys = BTreeMap::new();
        for key in &mint_params.keys {
            if key.denomination > mintv2::MAX_DENOMINATION {
                return Err(invalid(format!(
                    "denomination 2^{} is above fedimint's range",
                    key.denomination
                )));
            }
            let point = g2_from_bytes(&key.key.0).ok_or_else(|| {
                invalid(format!(
                    "mint key for denomination 2^{} is not a valid G2 point",
                    key.denomination
                ))
            })?;
            if keys.insert(key.denomination, point).is_some() {
                return Err(invalid(format!(
                    "denomination 2^{} has two keys",
                    key.denomination
                )));
            }
        }
        if keys.is_empty() {
            return Err(invalid("the mint lists no denominations"));
        }
        let mint = MintModule {
            instance: mint_params.instance,
            keys,
            fee: fee_rule(
                mint_params.fee_base_msat,
                mint_params.fee_ppm,
                1_000,
                "mint",
            )?,
        };

        let lightning = match &descriptor.lightning {
            None => None,
            Some(params) => {
                expect_kind(params.instance, lnv2::KIND)?;
                Some(LightningModule {
                    instance: params.instance,
                    network: bitcoin_network(params.network),
                    fee: fee_rule(params.fee_base_msat, params.fee_ppm, 1_000, "lightning")?,
                })
            }
        };
        let wallet = match &descriptor.wallet {
            None => None,
            Some(params) => {
                expect_kind(params.instance, walletv2::KIND)?;
                Some(WalletModule {
                    instance: params.instance,
                    network: bitcoin_network(params.network),
                    dust_limit_sat: params.dust_limit_sat,
                    fee: fee_rule(params.fee_base_msat, params.fee_ppm, 10_000, "wallet")?,
                })
            }
        };
        if let (Some(lightning), Some(wallet)) = (&lightning, &wallet) {
            if lightning.network != wallet.network {
                return Err(invalid(
                    "the lightning and wallet modules disagree on the network",
                ));
            }
        }
        let typed: BTreeSet<u16> = [
            Some(mint.instance),
            lightning.as_ref().map(|m| m.instance),
            wallet.as_ref().map(|m| m.instance),
        ]
        .into_iter()
        .flatten()
        .collect();
        let typed_count = 1 + usize::from(lightning.is_some()) + usize::from(wallet.is_some());
        if typed.len() != typed_count {
            return Err(invalid("two module configs share an instance id"));
        }

        Ok(Self {
            id,
            name: display_name(&descriptor.name)?,
            guardians,
            modules,
            mint,
            lightning,
            wallet,
        })
    }

    /// The module behind an instance id, if the signer understands it.
    pub fn known_module(&self, instance: u16) -> Option<KnownModule> {
        if instance == self.mint.instance {
            Some(KnownModule::Mint)
        } else if self
            .lightning
            .as_ref()
            .is_some_and(|m| m.instance == instance)
        {
            Some(KnownModule::Lightning)
        } else if self.wallet.as_ref().is_some_and(|m| m.instance == instance) {
            Some(KnownModule::Wallet)
        } else {
            None
        }
    }

    /// The module kind name for any listed instance, including unsupported ones.
    pub fn module_kind(&self, instance: u16) -> Option<&str> {
        self.modules.get(&instance).map(|entry| entry.kind.as_str())
    }

    /// The name to show: the meta name, or a short form of the id.
    pub fn display_name(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("Federation {}", short_hex(&self.id)),
        }
    }

    pub fn summary(&self) -> FederationSummary {
        FederationSummary {
            federation: self.id.into(),
            name: self.name.clone(),
            guardians: self.guardians.len() as u16,
            modules: self.modules.values().map(|m| m.kind.clone()).collect(),
        }
    }
}

/// First and last four bytes in hex, as Foundation shows fingerprints.
pub fn short_hex(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if hex.len() <= 16 {
        return hex;
    }
    format!("{}…{}", &hex[..8], &hex[hex.len() - 8..])
}
