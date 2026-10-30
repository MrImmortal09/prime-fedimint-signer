//! What Prime needs to know about a federation, from the federation's client config.
//!
//! Prime authenticates only the guardian endpoints, against the federation id; the module
//! parameters are trusted from the phone at import (decision D12 in `docs/plan/10_DECISIONS.md`).

use bitcoin::hashes::Hash as _;
use fedimint_core::Amount;
use fedimint_core::config::ClientConfig;
use fedimint_core::encoding::Encodable as _;
use fedimint_core::module::AmountUnit;
use fedimint_lnv2_common::config::LightningClientConfig;
use fedimint_mintv2_common::config::MintClientConfig;
use fedimint_walletv2_common::config::WalletClientConfig;
use fps_protocol::{
    DenominationKey, FederationDescriptor, FixedBytes, LightningParams, MintParams, ModuleInfo,
    Network, WalletParams,
};

/// The descriptor of the federation `config` describes. Fails if a module's config does not
/// decode, or the mint is not in bitcoin.
pub fn federation_descriptor(config: &ClientConfig) -> Result<FederationDescriptor, String> {
    let mut descriptor = FederationDescriptor {
        federation: FixedBytes(config.calculate_federation_id().0.to_byte_array()),
        api_endpoints: config.global.api_endpoints.consensus_encode_to_vec().into(),
        name: config.global.federation_name().map(str::to_owned),
        modules: config
            .modules
            .iter()
            .map(|(instance, module)| ModuleInfo {
                instance: *instance,
                kind: module.kind.as_str().to_owned(),
                major: module.version.major,
                minor: module.version.minor,
            })
            .collect(),
        mint: None,
        lightning: None,
        wallet: None,
    };

    for (instance, module) in &config.modules {
        let wrong_config = |error| format!("module {instance} ({}): {error}", module.kind);
        match module.kind.as_str() {
            "mintv2" => {
                let mint = module
                    .cast::<MintClientConfig>()
                    .map_err(|error| wrong_config(error.to_string()))?;
                if mint.amount_unit != AmountUnit::BITCOIN {
                    return Err(wrong_config("the mint is not in bitcoin".to_owned()));
                }
                let fee_base_msat = mint.fee_consensus.base_fee().msats;
                // The fee rule is `amount * ppm / 1e6 + base`, so a million msat yields the rate.
                let fee_ppm =
                    mint.fee_consensus.fee(Amount::from_msats(1_000_000)).msats - fee_base_msat;
                descriptor.mint = Some(MintParams {
                    instance: *instance,
                    keys: mint
                        .tbs_agg_pks
                        .iter()
                        .map(|(denomination, key)| DenominationKey {
                            denomination: denomination.0,
                            key: FixedBytes(key.0.to_compressed()),
                        })
                        .collect(),
                    fee_base_msat,
                    fee_ppm,
                    amount_unit: 0,
                });
            }
            "lnv2" => {
                let lightning = module
                    .cast::<LightningClientConfig>()
                    .map_err(|error| wrong_config(error.to_string()))?;
                descriptor.lightning = Some(LightningParams {
                    instance: *instance,
                    network: network(lightning.network),
                    fee_base_msat: lightning.fee_consensus.base.msats,
                    fee_ppm: lightning.fee_consensus.parts_per_million,
                });
            }
            "walletv2" => {
                let wallet = module
                    .cast::<WalletClientConfig>()
                    .map_err(|error| wrong_config(error.to_string()))?;
                descriptor.wallet = Some(WalletParams {
                    instance: *instance,
                    network: network(wallet.network),
                    dust_limit_sat: wallet.dust_limit.to_sat(),
                    fee_base_msat: wallet.fee_consensus.base.msats,
                    fee_ppm: wallet.fee_consensus.parts_per_million,
                });
            }
            _ => {}
        }
    }

    Ok(descriptor)
}

fn network(network: bitcoin::Network) -> Network {
    match network {
        bitcoin::Network::Bitcoin => Network::Bitcoin,
        bitcoin::Network::Testnet => Network::Testnet,
        bitcoin::Network::Testnet4 => Network::Testnet4,
        bitcoin::Network::Signet => Network::Signet,
        bitcoin::Network::Regtest => Network::Regtest,
    }
}
