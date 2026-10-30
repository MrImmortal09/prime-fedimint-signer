//! The watch-only client against a federation, with Prime signing every spend.
//!
//! Each test runs a federation in this process, pairs a fresh device, and builds a client whose
//! notes and refunds only Prime can sign.

mod common;

use common::{
    CheckingGateway, Decision, INVOICE_PREIMAGE, fixtures, fund, gateway, paired_wallet,
    payable_invoice, peg_in, received_on_chain, recovering_client, settled_balance,
    unpayable_invoice,
};
use fedimint_client_module::error::TransactionSubmitError;
use fedimint_client_module::transaction::TransactionSignerError;
use fedimint_core::Amount;
use fedimint_core::base32::{self, FEDIMINT_PREFIX};
use fedimint_lnv2_client::{FinalSendOperationState, LightningClientModule};
use fedimint_mintv2_client::{
    ECash, ExternalNotesError, FinalReceiveOperationState, MintClientModule, SendECashError,
};
use fedimint_walletv2_client::WalletClientModule;
use fps_protocol::Delivery;
use fps_wallet::send::SendError;
use serde_json::Value;

#[tokio::test(flavor = "multi_thread")]
async fn receiving_needs_no_device_and_paying_needs_its_approval() -> anyhow::Result<()> {
    let gateway_conn = CheckingGateway::new();
    let fed = fixtures(gateway_conn.clone()).new_fed_not_degraded().await;
    let (device, _keys, wallet) = paired_wallet(&fed, &gateway_conn).await;
    assert_eq!(device.reviews(), ["Add federation", "Connect wallet app"]);
    let after_pairing = device.requests();

    // Funds arrive, from the dummy module and as ecash from a stranger, without the device.
    fund(&wallet, Amount::from_sats(10_000)).await;
    let stranger = fed.new_client().await;
    fund(&stranger, Amount::from_sats(5_000)).await;
    let (_, ecash) = stranger
        .get_first_module::<MintClientModule>()?
        .send(Amount::from_sats(1_000), Value::Null, false)
        .await?;
    let mint = wallet.get_first_module::<MintClientModule>()?;
    let receive = mint.receive(ecash, Value::Null).await?;
    assert_eq!(
        mint.await_final_receive_operation_state(receive).await?,
        FinalReceiveOperationState::Success
    );
    assert_eq!(
        device.requests(),
        after_pairing,
        "receiving never reached the device"
    );
    let balance = settled_balance(&wallet).await;
    assert!(balance > Amount::from_sats(10_900), "{balance}");

    // The wallet cannot hand its notes out itself: the device holds their keys.
    assert!(matches!(
        mint.send(Amount::from_sats(100), Value::Null, false).await,
        Err(SendECashError::SpendKeysHeldElsewhere)
    ));

    // A Lightning payment: the device reviews the invoice, signs the funding and authorises the
    // gateway, which checks the authorisation before it pays.
    let lightning = wallet.get_first_module::<LightningClientModule>()?;
    let payment = lightning
        .send(payable_invoice(), Some(gateway()), Value::Null)
        .await?;
    assert_eq!(
        lightning.await_final_send_operation_state(payment).await?,
        FinalSendOperationState::Success(INVOICE_PREIMAGE)
    );
    assert_eq!(device.reviews().last().unwrap(), "Pay Lightning invoice");
    assert_eq!(gateway_conn.authorised(), 1);
    assert_eq!(device.funded_contracts(), 1);
    let paid = settled_balance(&wallet).await;
    assert!(
        paid + Amount::from_sats(1_000) < balance,
        "{paid} after {balance}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_payment_is_refunded_with_the_device_signing() -> anyhow::Result<()> {
    let gateway_conn = CheckingGateway::new();
    let fed = fixtures(gateway_conn.clone()).new_fed_not_degraded().await;
    let (device, _keys, wallet) = paired_wallet(&fed, &gateway_conn).await;
    fund(&wallet, Amount::from_sats(10_000)).await;
    let balance = settled_balance(&wallet).await;

    let lightning = wallet.get_first_module::<LightningClientModule>()?;
    let payment = lightning
        .send(unpayable_invoice(), Some(gateway()), Value::Null)
        .await?;
    assert_eq!(
        lightning.await_final_send_operation_state(payment).await?,
        FinalSendOperationState::Refunded
    );
    assert_eq!(
        device.reviews()[2..],
        ["Pay Lightning invoice", "Refund failed payment"]
    );
    // Only fees are lost.
    let refunded = settled_balance(&wallet).await;
    assert!(
        refunded + Amount::from_sats(10) > balance,
        "{refunded} after {balance}"
    );
    assert!(refunded < balance);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ecash_leaves_from_the_outgoing_branch_and_comes_back_if_unclaimed() -> anyhow::Result<()> {
    let gateway_conn = CheckingGateway::new();
    let fed = fixtures(gateway_conn.clone()).new_fed_not_degraded().await;
    let (device, keys, wallet) = paired_wallet(&fed, &gateway_conn).await;
    fund(&wallet, Amount::from_sats(10_000)).await;
    let mint = wallet.get_first_module::<MintClientModule>()?;

    // Staging needs the device's approval; releasing what it staged does not.
    let staged = keys.prepare_send(&mint, Amount::from_sats(1_024)).await?;
    assert_eq!(staged.amount(), Amount::from_sats(1_024));
    assert_eq!(device.reviews().last().unwrap(), "Prepare ecash to send");
    assert_eq!(device.prepared_notes(), staged.notes.len());
    let reviews = device.reviews().len();
    let released = keys
        .release(&staged.notes, Delivery::ReturnToPhone, true)
        .await?;
    assert_eq!(device.reviews().len(), reviews, "no second review");
    assert_eq!(released.amount_msat, Amount::from_sats(1_024).msats);
    assert_eq!(device.released_notes(), staged.notes.len());

    // A stranger who never met this wallet redeems the token.
    let token = released.token.expect("returned to the phone");
    let ecash: ECash = base32::decode_prefixed(FEDIMINT_PREFIX, &token)?;
    assert_eq!(ecash.amount(), Amount::from_sats(1_024));
    let stranger = fed.new_client().await;
    let stranger_mint = stranger.get_first_module::<MintClientModule>()?;
    let receive = stranger_mint.receive(ecash, Value::Null).await?;
    assert_eq!(
        stranger_mint
            .await_final_receive_operation_state(receive)
            .await?,
        FinalReceiveOperationState::Success
    );

    // A token shown on the device and never scanned comes back with a reclaim.
    let staged = keys.prepare_send(&mint, Amount::from_sats(2_048)).await?;
    let released = keys
        .release(&staged.notes, Delivery::ShowOnDevice, false)
        .await?;
    assert_eq!(released.token, None, "the phone never sees it");
    let before = settled_balance(&wallet).await;
    let reclaim = keys.reclaim(&mint, &staged.notes).await?;
    assert_eq!(device.reviews().last().unwrap(), "Reclaim unredeemed ecash");
    assert_eq!(
        mint.await_final_receive_operation_state(reclaim).await?,
        FinalReceiveOperationState::Success
    );
    let after = settled_balance(&wallet).await;
    assert!(
        after > before + Amount::from_sats(2_040),
        "{after} after {before}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_withdrawal_reaches_the_chain_with_the_device_signing() -> anyhow::Result<()> {
    let gateway_conn = CheckingGateway::new();
    let fixtures = fixtures(gateway_conn.clone());
    let chain = fixtures.bitcoin();
    let fed = fixtures.new_fed_not_degraded().await;
    peg_in(&fed, &chain, bitcoin::Amount::from_sat(1_000_000)).await?;
    let (device, _keys, wallet) = paired_wallet(&fed, &gateway_conn).await;
    fund(&wallet, Amount::from_sats(50_000)).await;
    let balance = settled_balance(&wallet).await;

    let onchain = wallet.get_first_module::<WalletClientModule>()?;
    let address = chain.get_new_address().await;
    let value = bitcoin::Amount::from_sat(20_000);
    let fee = onchain.send_fee().await?;
    let quote = onchain.send_fee_quote(value + fee).await?;
    let withdrawal = onchain
        .send(
            address.as_unchecked().clone(),
            value,
            Some(fee),
            Value::Null,
        )
        .await?;
    assert!(matches!(
        onchain.await_final_send_operation_state(withdrawal).await?,
        fedimint_walletv2_client::FinalSendOperationState::Success(_)
    ));
    assert_eq!(device.reviews().last().unwrap(), "Withdraw on chain");
    assert_eq!(
        received_on_chain(&chain, &address).await?,
        Amount::from_sats(value.to_sat())
    );

    // The wallet paid the value, the miner fee and the federation's fees as fedimint quoted them.
    let spent = balance
        .checked_sub(settled_balance(&wallet).await)
        .expect("the balance went down");
    assert_eq!(
        spent,
        Amount::from_sats((value + fee).to_sat()) + quote.total().get_bitcoin()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declined_spend_leaves_the_wallet_as_it_was() -> anyhow::Result<()> {
    let gateway_conn = CheckingGateway::new();
    let fed = fixtures(gateway_conn.clone()).new_fed_not_degraded().await;
    let (device, keys, wallet) = paired_wallet(&fed, &gateway_conn).await;
    fund(&wallet, Amount::from_sats(10_000)).await;
    let balance = settled_balance(&wallet).await;
    let mint = wallet.get_first_module::<MintClientModule>()?;

    device.decide_next(Decision::Reject);
    let declined = keys.prepare_send(&mint, Amount::from_sats(1_024)).await;
    assert!(
        matches!(
            declined,
            Err(SendError::Mint(ExternalNotesError::Failed(
                TransactionSubmitError::Signer(TransactionSignerError::Declined(_))
            )))
        ),
        "{declined:?}"
    );
    assert_eq!(device.prepared_notes(), 0);

    device.decide_next(Decision::Reject);
    let lightning = wallet.get_first_module::<LightningClientModule>()?;
    assert!(
        lightning
            .send(payable_invoice(), Some(gateway()), Value::Null)
            .await
            .is_err()
    );
    assert_eq!(device.funded_contracts(), 0);

    assert_eq!(settled_balance(&wallet).await, balance);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_phone_recovers_the_wallet_from_the_view_material() -> anyhow::Result<()> {
    let gateway_conn = CheckingGateway::new();
    let fed = fixtures(gateway_conn.clone()).new_fed_not_degraded().await;
    let (_device, keys, wallet) = paired_wallet(&fed, &gateway_conn).await;
    fund(&wallet, Amount::from_sats(10_000)).await;
    let lightning = wallet.get_first_module::<LightningClientModule>()?;
    let payment = lightning
        .send(payable_invoice(), Some(gateway()), Value::Null)
        .await?;
    lightning.await_final_send_operation_state(payment).await?;
    let balance = settled_balance(&wallet).await;

    let config = wallet.config().await;
    let recovered = recovering_client(config, &keys, &gateway_conn).await;
    recovered.wait_for_all_recoveries().await?;
    assert_eq!(settled_balance(&recovered).await, balance);
    Ok(())
}
