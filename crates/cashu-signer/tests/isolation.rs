//! The two state files stay apart: Cashu never writes `fps-state.cbor`, Fedimint never writes
//! `fps-cashu.cbor`, and a device over two empty files knows nothing of either backend. The
//! simulator's reset, which removes both files, is the app's and is checked on the simulator.

mod common;

use common::mint::{self, Fixture};
use common::*;
use fps_policy::PolicyConfig;
use fps_protocol::*;
use fps_signer::State;

#[test]
fn cashu_never_writes_the_fedimint_state() {
    let fixture = Fixture::new();
    // A populated signer state: the frozen one, with its federation, ledger, answers, policy and
    // today's count.
    let (fedimint, cashu) = (Watched::holding(FROZEN_STATE), Watched::default());
    let mut device = open_device(mint::APP_SEED, &fedimint, &cashu, NOW);

    cashu_info(&mut device, id(100));
    added(&mut device, id(101), &fixture.descriptor);
    approval(send(&mut device, id(102), add_mint(&fixture.descriptor)));
    device.reject(&id(102)).unwrap();
    signed(&mut device, id(103), mint::send(&fixture));
    approval(send(&mut device, id(104), sign(mint::melt(&fixture))));
    device.reject(&id(104)).unwrap();
    approval(send(&mut device, id(105), sign(mint::send(&fixture))));
    reply(send(
        &mut device,
        id(106),
        Request::GetResult { request: id(105) },
    ));
    // Busy, while the Cashu review waits.
    rejection(send(&mut device, id(107), sign(mint::melt(&fixture))));
    reply(send(
        &mut device,
        id(108),
        Request::Cancel { request: id(105) },
    ));
    reply(send(
        &mut device,
        id(109),
        Request::GetResult { request: id(103) },
    ));
    let mut spelled = fixture.descriptor.clone();
    spelled.url.push('/');
    rejection(send(&mut device, id(110), add_mint(&spelled)));
    // Refused by the policy it reads from the signer: with Fedimint's count, over the daily limit.
    assert_eq!(
        rejection(send(&mut device, id(111), sign(large_spend(&fixture)))).code,
        RejectCode::PolicyLimit
    );

    assert_eq!(cashu.write_count(), 5);
    assert_eq!(fedimint.write_count(), 0);
    assert!(fedimint.bytes().unwrap() == FROZEN_STATE);
    assert_eq!(device.state(), &State::decode(FROZEN_STATE).unwrap());
}

/// A send of 65,536 sats, over the frozen state's daily limit together with Fedimint's count.
fn large_spend(fixture: &Fixture) -> CashuSignRequest {
    fixture.swap(
        0,
        vec![fixture.proof(mint::C, 0, 200, 65_536)],
        vec![fixture.leaving(mint::C, 65_536, 200)],
    )
}

#[test]
fn fedimint_never_writes_the_cashu_state() {
    let golden = Golden::load();
    let populated = populated_cashu_state();
    let (fedimint, cashu) = (Watched::default(), Watched::holding(&populated));
    let mut device = open_device(golden.app_seed, &fedimint, &cashu, NOW);
    let federation = golden.federation();

    approved(
        &mut device,
        id(1),
        Request::ImportFederation(golden.descriptor.clone()),
    );
    for (n, case) in [(2, "reissue"), (3, "pay_invoice"), (4, "prepare_send")] {
        approved(&mut device, id(n), golden.sign(case));
    }
    approved(
        &mut device,
        id(5),
        Request::ExportViewMaterial { federation },
    );
    approval(send(&mut device, id(6), golden.sign("withdraw")));
    device.reject(&id(6)).unwrap();
    approval(send(&mut device, id(7), golden.sign("refund")));
    reply(send(
        &mut device,
        id(8),
        Request::GetResult { request: id(7) },
    ));
    reply(send(&mut device, id(9), Request::Cancel { request: id(7) }));
    reply(send(&mut device, id(10), Request::GetInfo));
    reply(send(&mut device, id(11), Request::GetStatus));
    reply(send(
        &mut device,
        id(12),
        Request::GetResult { request: id(99) },
    ));
    reply(send(
        &mut device,
        id(13),
        Request::Cancel { request: id(99) },
    ));
    assert!(matches!(
        send(
            &mut device,
            id(14),
            golden.release("prepare_send", Delivery::ShowOnDevice)
        ),
        fps_signer::Outcome::Released { .. }
    ));
    device.set_policy(compat_policy()).unwrap();
    device.forget_federation(&federation).unwrap();
    // Cashu answers that only read leave it as it was too: a stored Cashu answer found by
    // `GetResult`, and one replayed under an id whose Fedimint answer is stored as well.
    cashu_info(&mut device, id(15));
    let cancelled = reply(send(
        &mut device,
        id(16),
        Request::GetResult { request: id(41) },
    ));
    assert_eq!(
        (cancelled.id, cancelled.body),
        (id(41), Response::Cancelled)
    );
    assert!(device.state().stored_result(&id(2)).is_some());
    let replayed = reply(send(&mut device, id(2), sign(mint::send(&Fixture::new()))));
    assert!(matches!(
        replayed.body,
        Response::Cashu(CashuResponse::Signed(_))
    ));

    assert!(fedimint.write_count() > 8);
    assert_eq!(cashu.write_count(), 0);
    assert!(cashu.bytes().unwrap() == populated);
}

#[test]
fn a_device_over_empty_files_knows_neither_backend() {
    let fixture = Fixture::new();
    let golden = Golden::load();
    let (mut device, fedimint, cashu) = cashu_device();
    approved(
        &mut device,
        id(1),
        Request::ImportFederation(golden.descriptor.clone()),
    );
    added(&mut device, id(2), &fixture.descriptor);
    signed(&mut device, id(3), mint::send(&fixture));
    device
        .set_policy(PolicyConfig {
            pin_above_msat: None,
            ..PolicyConfig::default()
        })
        .unwrap();
    assert!(fedimint.bytes().is_some() && cashu.bytes().is_some());
    drop(device);

    // What a device opened after the simulator's reset sees, since the reset removes
    // `fps-state.cbor` and `fps-cashu.cbor` (and their `.tmp` and `.new`) before the app opens:
    // storages that hold nothing. The reset itself runs only in the app.
    let (fedimint, cashu) = (Watched::default(), Watched::default());
    let mut device = open_device(mint::APP_SEED, &fedimint, &cashu, NOW);
    assert!(device.federations().is_empty());
    assert!(device.cashu_mints().is_empty());
    assert_eq!(device.policy(), &PolicyConfig::default());
    for request in [1, 2, 3] {
        assert_eq!(
            rejection(send(
                &mut device,
                id(10),
                Request::GetResult {
                    request: id(request)
                }
            ))
            .code,
            RejectCode::NotFound
        );
    }
    assert_eq!(
        rejection(send(&mut device, id(11), sign(mint::send(&fixture)))).code,
        RejectCode::UnknownFederation
    );
    assert_eq!((fedimint.write_count(), cashu.write_count()), (0, 0));
}
