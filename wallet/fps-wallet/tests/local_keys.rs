//! A client that holds its own keys still works as fedimint always has: the patch series leaves
//! the local signing path alone.

use fedimint_client::transaction::TransactionBuilder;
use fedimint_core::Amount;
use fedimint_core::core::OperationId;
use fedimint_dummy_client::{DummyClientInit, DummyClientModule};
use fedimint_dummy_server::DummyInit;
use fedimint_mintv2_client::{FinalReceiveOperationState, MintClientInit, MintClientModule};
use fedimint_mintv2_server::MintInit;
use fedimint_testing::fixtures::Fixtures;
use serde_json::Value;

#[tokio::test(flavor = "multi_thread")]
async fn send_and_receive() -> anyhow::Result<()> {
    let fixtures =
        Fixtures::new_primary(MintClientInit, MintInit).with_module(DummyClientInit, DummyInit);
    let fed = fixtures.new_fed_not_degraded().await;
    let (sender, receiver) = fed.two_clients().await;

    let dummy = sender.get_first_module::<DummyClientModule>()?;
    let operation_id = OperationId::new_random();
    let range = sender
        .finalize_and_submit_transaction(
            operation_id,
            "issue",
            |_| (),
            TransactionBuilder::new().with_inputs(dummy.create_input(Amount::from_sats(10_000))),
        )
        .await?;
    sender
        .await_primary_bitcoin_module_outputs(operation_id, range.into_iter().collect())
        .await?;

    let (_, ecash) = sender
        .get_first_module::<MintClientModule>()?
        .send(Amount::from_sats(1_000), Value::Null, false)
        .await?;
    let receive = receiver.get_first_module::<MintClientModule>()?;
    let operation_id = receive.receive(ecash, Value::Null).await?;
    assert_eq!(
        receive
            .await_final_receive_operation_state(operation_id)
            .await?,
        FinalReceiveOperationState::Success
    );
    Ok(())
}
