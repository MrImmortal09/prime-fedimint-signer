//! Spending policy.

use fps_policy::*;

fn spend(outflow_msat: u64) -> Spend {
    Spend {
        outflow_msat,
        routing: None,
    }
}

fn allowed(decision: Decision) -> bool {
    matches!(decision, Decision::Allow { .. })
}

#[test]
fn defaults_mirror_fedimints_gateway_fee_limit_and_ask_for_the_pin_on_large_amounts() {
    let config = PolicyConfig::default();
    // fedimint's SEND_FEE_LIMIT: 100 sats plus 1.5%.
    assert_eq!(config.routing_fee_limit_msat(1_000_000), 100_000 + 15_000);
    let state = PolicyState::default();
    assert_eq!(
        config.evaluate(&state, None, &spend(1_000)),
        Decision::Allow { require_pin: false }
    );
    assert_eq!(
        config.evaluate(&state, None, &spend(100_000_001)),
        Decision::Allow { require_pin: true }
    );
}

#[test]
fn gateway_fees_above_the_limit_are_refused() {
    let config = PolicyConfig::default();
    let state = PolicyState::default();
    let at_limit = Spend {
        outflow_msat: 1,
        routing: Some(Routing {
            invoice_msat: 10_000_000,
            fee_msat: 250_000,
        }),
    };
    assert!(allowed(config.evaluate(&state, None, &at_limit)));
    let over = Spend {
        routing: Some(Routing {
            invoice_msat: 10_000_000,
            fee_msat: 250_001,
        }),
        ..at_limit
    };
    assert!(!allowed(config.evaluate(&state, None, &over)));
}

#[test]
fn the_per_request_limit_applies_to_outflow() {
    let config = PolicyConfig {
        max_outflow_per_request_msat: Some(5_000),
        ..PolicyConfig::default()
    };
    let state = PolicyState::default();
    assert!(allowed(config.evaluate(&state, None, &spend(5_000))));
    assert!(!allowed(config.evaluate(&state, None, &spend(5_001))));
}

#[test]
fn the_daily_limit_accumulates_and_resets_at_midnight_utc() {
    let config = PolicyConfig {
        max_outflow_per_day_msat: Some(10_000),
        ..PolicyConfig::default()
    };
    let day = day_of(1_800_000_000);
    let mut state = PolicyState::default();
    state.record(Some(day), 6_000);
    assert!(allowed(config.evaluate(&state, Some(day), &spend(4_000))));
    assert!(!allowed(config.evaluate(&state, Some(day), &spend(4_001))));
    // A new day starts from zero.
    assert!(allowed(config.evaluate(
        &state,
        Some(day + 1),
        &spend(10_000)
    )));
    state.record(Some(day + 1), 1_000);
    assert_eq!(state.outflow_on(Some(day + 1)), 1_000);
    assert_eq!(state.day, Some(day + 1));
}

#[test]
fn without_a_trustworthy_clock_the_count_only_grows() {
    let config = PolicyConfig {
        max_outflow_per_day_msat: Some(10_000),
        ..PolicyConfig::default()
    };
    let mut state = PolicyState::default();
    state.record(None, 7_000);
    state.record(None, 2_000);
    assert_eq!(state.outflow_on(None), 9_000);
    assert!(!allowed(config.evaluate(&state, None, &spend(2_000))));
    // A clock appearing later does not wipe what was counted without it.
    assert_eq!(state.outflow_on(Some(20_000)), 9_000);
    // Nor does a clock that jumps backwards.
    let mut state = PolicyState::default();
    state.record(Some(100), 3_000);
    assert_eq!(state.outflow_on(Some(99)), 3_000);
    state.record(Some(99), 1_000);
    assert_eq!(state.day, Some(100));
    assert_eq!(state.outflow_msat, 4_000);
}

#[test]
fn settings_round_trip_and_fill_in_missing_fields() {
    let config = PolicyConfig {
        pin_above_msat: None,
        max_outflow_per_day_msat: Some(1),
        ..PolicyConfig::default()
    };
    let mut bytes = Vec::new();
    ciborium::into_writer(&config, &mut bytes).unwrap();
    let decoded: PolicyConfig = ciborium::from_reader(bytes.as_slice()).unwrap();
    assert_eq!(decoded, config);

    // Settings saved by an older build, with fields missing, take the defaults.
    #[derive(serde::Serialize)]
    struct Old {
        max_inputs: usize,
    }
    let mut bytes = Vec::new();
    ciborium::into_writer(&Old { max_inputs: 8 }, &mut bytes).unwrap();
    let decoded: PolicyConfig = ciborium::from_reader(bytes.as_slice()).unwrap();
    assert_eq!(decoded.max_inputs, 8);
    assert_eq!(
        decoded.max_routing_fee_ppm,
        PolicyConfig::default().max_routing_fee_ppm
    );
}
