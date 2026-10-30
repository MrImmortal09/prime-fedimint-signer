//! Moving signer state into the Slint UI.

use fps_policy::PolicyConfig;
use fps_signer::{Approval, Platform, Signer, Storage};
use fps_signer_core::review::format_msat;
use slint_keyos_platform::slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{AppWindow, Callbacks, FederationRow, ReviewRow, Screen};

/// The PIN threshold the settings switch turns on: 100,000 sats.
pub const PIN_THRESHOLD_MSAT: u64 = 100_000_000;

pub fn refresh_home<S: Storage, P: Platform>(ui: &AppWindow, signer: &Signer<S, P>) {
    let rows: Vec<FederationRow> = signer
        .federations()
        .into_iter()
        .map(|summary| FederationRow {
            name: summary
                .name
                .clone()
                .unwrap_or_else(|| format!("Federation {}", &summary.federation.to_hex()[..8]))
                .into(),
            detail: format!(
                "{} guardian{} · {}",
                summary.guardians,
                if summary.guardians == 1 { "" } else { "s" },
                summary.modules.join(", ")
            )
            .into(),
        })
        .collect();
    let callbacks = ui.global::<Callbacks>();
    callbacks.set_federations(ModelRc::new(VecModel::from(rows)));
    let released = signer.state().released.len();
    callbacks.set_ledger_note(
        if released == 0 {
            String::new()
        } else {
            format!(
                "{released} released note{}",
                if released == 1 { "" } else { "s" }
            )
        }
        .into(),
    );
}

pub fn show_review(ui: &AppWindow, approval: &Approval) {
    let review = &approval.review;
    let callbacks = ui.global::<Callbacks>();
    callbacks.set_review_title(review.title.clone().into());
    callbacks.set_review_amount(if review.amount_msat == 0 {
        SharedString::new()
    } else {
        format_msat(review.amount_msat).into()
    });
    let rows: Vec<ReviewRow> = review
        .lines
        .iter()
        .map(|line| ReviewRow {
            label: line.label.clone().into(),
            value: line.value.clone().into(),
        })
        .collect();
    callbacks.set_review_rows(ModelRc::new(VecModel::from(rows)));
    let warnings: Vec<SharedString> = review.warnings.iter().map(|w| w.clone().into()).collect();
    callbacks.set_review_warnings(ModelRc::new(VecModel::from(warnings)));
    callbacks.set_review_needs_pin(approval.require_pin);
}

pub fn show_screen(ui: &AppWindow, screen: &Screen) {
    let callbacks = ui.global::<Callbacks>();
    callbacks.set_result_title(screen.title.clone().into());
    callbacks.set_result_message(screen.message.clone().into());
    callbacks.set_result_frames(screen.frames.clone());
    callbacks.set_result_secret(screen.secret);
    callbacks.set_result_has_next(screen.has_next);
}

pub fn load_settings(ui: &AppWindow, policy: &PolicyConfig) {
    let callbacks = ui.global::<Callbacks>();
    callbacks.set_setting_pin(policy.pin_above_msat.is_some());
    callbacks.set_setting_verify_notes(policy.verify_note_signatures);
    callbacks.set_setting_auto_release(policy.release_prepared_without_review);
    let daily = policy
        .max_outflow_per_day_msat
        .map(format_msat)
        .unwrap_or_else(|| "none".to_owned());
    callbacks.set_setting_summary(
        format!(
            "Gateway fees are capped at {} plus {}.{}%. Daily limit: {daily}.",
            format_msat(policy.max_routing_fee_base_msat),
            policy.max_routing_fee_ppm / 10_000,
            (policy.max_routing_fee_ppm % 10_000) / 1_000
        )
        .into(),
    );
}

/// The policy with the settings screen's switches applied.
pub fn policy_from_settings(ui: &AppWindow, current: &PolicyConfig) -> PolicyConfig {
    let callbacks = ui.global::<Callbacks>();
    PolicyConfig {
        pin_above_msat: callbacks.get_setting_pin().then_some(PIN_THRESHOLD_MSAT),
        verify_note_signatures: callbacks.get_setting_verify_notes(),
        release_prepared_without_review: callbacks.get_setting_auto_release(),
        ..current.clone()
    }
}
