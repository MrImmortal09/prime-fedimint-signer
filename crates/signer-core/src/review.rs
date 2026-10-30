//! What the device shows the user before approving a request.

/// A review screen, rendered by the device UI. Values are already formatted for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub title: String,
    /// The headline amount, in msat.
    pub amount_msat: u64,
    pub lines: Vec<ReviewLine>,
    /// Things the user should notice; never blocking by themselves.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewLine {
    pub label: String,
    pub value: String,
}

impl Review {
    pub fn new(title: impl Into<String>, amount_msat: u64) -> Self {
        Self {
            title: title.into(),
            amount_msat,
            lines: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn line(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.lines.push(ReviewLine {
            label: label.into(),
            value: value.into(),
        });
        self
    }

    pub fn warn(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }
}

/// `12,345 sats`, or `12,345.678 sats` when there is a msat remainder.
pub fn format_msat(msat: u64) -> String {
    let sats = msat / 1000;
    let rest = msat % 1000;
    let grouped = group_thousands(sats);
    if rest == 0 {
        format!("{grouped} sats")
    } else {
        let fraction = format!("{rest:03}");
        format!("{grouped}.{} sats", fraction.trim_end_matches('0'))
    }
}

fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
