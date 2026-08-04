//! The environment gate shared by the two cost tiers that cannot run on a
//! developer machine.
//!
//! The issue asks for a disposable-cluster integration tier and a self-hosted
//! PostHog staging smoke tier. Neither can run against a laptop with no cluster
//! and no PostHog project, and a test that silently passes in that situation is
//! worse than no test at all — it manufactures evidence.
//!
//! So both tiers are written as real suites behind one gate. When the gate's
//! variable is absent the suite prints an explicit reason and returns; when it is
//! present the suite runs for real and fails loudly. The requirement matrix marks
//! those rows `gated` and names the variable, and the matrix linter refuses a
//! gated row that does not.

#![allow(dead_code)]

/// Whether a tier may run, and why not when it may not.
pub enum Gate {
    Open(GateEnvironment),
    Closed(String),
}

/// The values a gated tier needs, read once so a half-configured environment is a
/// stated refusal rather than a confusing mid-test failure.
pub struct GateEnvironment {
    values: std::collections::BTreeMap<&'static str, String>,
}

impl GateEnvironment {
    /// A required value. Present by construction: [`open`] refuses the gate
    /// unless every declared variable is set.
    pub fn get(&self, key: &'static str) -> &str {
        self.values
            .get(key)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("{key} was declared required but is missing"))
    }
}

/// Open the gate named by `primary` when it and every entry of `also` is set.
///
/// The primary variable is the switch an operator flips; the others are the
/// configuration that switch implies. Setting the switch without the rest is a
/// misconfiguration, and is reported as such rather than skipped, because a
/// half-configured CI job that quietly skips is how a tier stops running without
/// anybody noticing.
pub fn open(primary: &'static str, also: &[&'static str]) -> Gate {
    let Ok(switch) = std::env::var(primary) else {
        return Gate::Closed(format!(
            "{primary} is not set, so this tier's environment is absent"
        ));
    };
    if switch.trim().is_empty() || switch == "0" || switch.eq_ignore_ascii_case("false") {
        return Gate::Closed(format!(
            "{primary} is set to {switch:?}, which disables this tier"
        ));
    }

    let mut values = std::collections::BTreeMap::new();
    values.insert(primary, switch);
    let mut missing = Vec::new();
    for key in also {
        match std::env::var(key) {
            Ok(value) if !value.trim().is_empty() => {
                values.insert(key, value);
            }
            _ => missing.push(*key),
        }
    }
    if !missing.is_empty() {
        panic!(
            "{primary} is set but this tier is misconfigured: {missing:?} are missing. \
             Either configure them or unset {primary} — a partially configured tier \
             must not silently skip."
        );
    }
    Gate::Open(GateEnvironment { values })
}

/// Report a closed gate on stdout, in a form a CI log reader can grep for.
pub fn skip(tier: &str, test: &str, reason: &str) {
    println!("ACCEPTANCE-SKIP tier={tier} test={test} reason={reason}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate is closed, with a stated reason, when its switch is absent.
    #[test]
    fn an_absent_switch_closes_the_gate_with_a_reason() {
        let key = "FKST_ACCEPTANCE_GATE_SELF_TEST_ABSENT";
        std::env::remove_var(key);
        match open(key, &[]) {
            Gate::Closed(reason) => assert!(reason.contains(key), "{reason}"),
            Gate::Open(_) => panic!("an unset switch must not open the gate"),
        }
    }

    /// An explicitly false switch is closed too, so a job can turn a tier off
    /// without unsetting a variable its image already carries.
    #[test]
    fn an_explicitly_disabled_switch_closes_the_gate() {
        let key = "FKST_ACCEPTANCE_GATE_SELF_TEST_FALSE";
        std::env::set_var(key, "false");
        let closed = matches!(open(key, &[]), Gate::Closed(_));
        std::env::remove_var(key);
        assert!(closed, "an explicit false must close the gate");
    }

    /// A configured gate opens and hands back its values.
    #[test]
    fn a_fully_configured_switch_opens_the_gate() {
        let key = "FKST_ACCEPTANCE_GATE_SELF_TEST_ON";
        let extra = "FKST_ACCEPTANCE_GATE_SELF_TEST_EXTRA";
        std::env::set_var(key, "1");
        std::env::set_var(extra, "value");
        let opened = match open(key, &[extra]) {
            Gate::Open(environment) => environment.get(extra).to_string(),
            Gate::Closed(reason) => panic!("{reason}"),
        };
        std::env::remove_var(key);
        std::env::remove_var(extra);
        assert_eq!(opened, "value");
    }
}
