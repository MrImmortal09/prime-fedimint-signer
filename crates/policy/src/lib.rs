//! Spending policy: limits the device enforces on its own, whatever the phone asks.
//!
//! The policy sees one number per request, its *outflow*: value that leaves the user's control if
//! the request goes through (a Lightning contract, a peg-out, released ecash, and every fee). It
//! refuses requests over the per-request or daily limit, caps the Lightning gateway fee, and asks
//! for the PIN above a threshold.
//!
//! Days are UTC days since the Unix epoch. Without a trustworthy clock the daily counter never
//! resets, which can only make the limit stricter.

use serde::{Deserialize, Serialize};

/// Seconds in a day.
const DAY_SECS: u64 = 86_400;

/// The user's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PolicyConfig {
    /// Refuse a single request whose outflow exceeds this.
    pub max_outflow_per_request_msat: Option<u64>,
    /// Refuse a request that would take the day's outflow above this.
    pub max_outflow_per_day_msat: Option<u64>,
    /// Ask for the PIN when a request's outflow exceeds this.
    pub pin_above_msat: Option<u64>,
    /// Gateway fee ceiling: base plus parts per million of the invoice amount. The default is
    /// fedimint's own `PaymentFee::SEND_FEE_LIMIT`, 100 sats plus 1.5%.
    pub max_routing_fee_base_msat: u64,
    pub max_routing_fee_ppm: u64,
    /// Check every input note's mint signature before signing (one pairing per note).
    pub verify_note_signatures: bool,
    /// Check every released note's mint signature before releasing.
    pub verify_release_signatures: bool,
    /// Release notes staged by an approved "prepare send" without asking a second time.
    pub release_prepared_without_review: bool,
    pub max_inputs: usize,
    pub max_outputs: usize,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            max_outflow_per_request_msat: None,
            max_outflow_per_day_msat: None,
            pin_above_msat: Some(100_000_000),
            max_routing_fee_base_msat: 100_000,
            max_routing_fee_ppm: 15_000,
            verify_note_signatures: true,
            verify_release_signatures: true,
            release_prepared_without_review: true,
            max_inputs: 64,
            max_outputs: 64,
        }
    }
}

/// What the policy has counted so far.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PolicyState {
    /// UTC day of `outflow_msat`, or `None` if it was counted without a clock.
    pub day: Option<u64>,
    pub outflow_msat: u64,
}

/// One request, as the policy sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spend {
    pub outflow_msat: u64,
    /// For a Lightning payment: the invoice amount and the gateway's fee.
    pub routing: Option<Routing>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Routing {
    pub invoice_msat: u64,
    pub fee_msat: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow { require_pin: bool },
    Deny { reason: String },
}

/// The UTC day of a Unix timestamp.
pub fn day_of(unix_secs: u64) -> u64 {
    unix_secs / DAY_SECS
}

impl PolicyConfig {
    /// The most a gateway may charge to route `invoice_msat`.
    pub fn routing_fee_limit_msat(&self, invoice_msat: u64) -> u64 {
        (invoice_msat.saturating_mul(self.max_routing_fee_ppm) / 1_000_000)
            .saturating_add(self.max_routing_fee_base_msat)
    }

    pub fn evaluate(&self, state: &PolicyState, today: Option<u64>, spend: &Spend) -> Decision {
        if let Some(routing) = spend.routing {
            let limit = self.routing_fee_limit_msat(routing.invoice_msat);
            if routing.fee_msat > limit {
                return Decision::Deny {
                    reason: format!(
                        "the gateway fee of {} msat is above the limit of {limit} msat",
                        routing.fee_msat
                    ),
                };
            }
        }
        if let Some(limit) = self.max_outflow_per_request_msat {
            if spend.outflow_msat > limit {
                return Decision::Deny {
                    reason: format!(
                        "{} msat is above the per-request limit of {limit} msat",
                        spend.outflow_msat
                    ),
                };
            }
        }
        if let Some(limit) = self.max_outflow_per_day_msat {
            let total = state.outflow_on(today).saturating_add(spend.outflow_msat);
            if total > limit {
                return Decision::Deny {
                    reason: format!(
                        "this would bring today's spending to {total} msat, above the daily limit of {limit} msat"
                    ),
                };
            }
        }
        Decision::Allow {
            require_pin: self
                .pin_above_msat
                .is_some_and(|threshold| spend.outflow_msat > threshold),
        }
    }
}

impl PolicyState {
    /// Outflow already counted for `today`.
    pub fn outflow_on(&self, today: Option<u64>) -> u64 {
        match (self.day, today) {
            // A new day starts a new count.
            (Some(counted), Some(today)) if today > counted => 0,
            _ => self.outflow_msat,
        }
    }

    /// Adds an approved request's outflow.
    pub fn record(&mut self, today: Option<u64>, outflow_msat: u64) {
        let base = self.outflow_on(today);
        self.outflow_msat = base.saturating_add(outflow_msat);
        // Keep the latest day seen; without a clock keep counting into the last known day.
        if let Some(today) = today {
            self.day = Some(self.day.map_or(today, |day| day.max(today)));
        }
    }
}
