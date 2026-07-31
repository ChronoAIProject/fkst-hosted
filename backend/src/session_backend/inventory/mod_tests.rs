//! The policy value and the metadata-state vocabulary.

use crate::reconcile_config::ReconcileConfig;

use super::*;

#[test]
fn the_policy_mirrors_the_deployment_configuration() {
    let config = ReconcileConfig {
        pod_session_max_lifetime_secs: 86_400,
        pod_min_lifetime_secs: 240,
        session_idle_grace_secs: 600,
        sandbox_inventory_max_source_items: 1234,
        ..ReconcileConfig::default()
    };
    assert_eq!(
        RuntimeLifetimePolicy::from_reconcile_config(&config),
        RuntimeLifetimePolicy {
            max_lifetime_seconds: 86_400,
            minimum_lifetime_seconds: 240,
            idle_grace_seconds: 600,
            max_items: 1234,
        }
    );
}

#[test]
fn the_default_deployment_policy_is_unlimited_lifetime() {
    // The shipped default is `FKST_POD_SESSION_MAX_LIFETIME_SECS=0`; the zero must
    // reach the policy verbatim so the timing layer can read it as "unlimited"
    // rather than "expires immediately".
    let policy = RuntimeLifetimePolicy::from_reconcile_config(&ReconcileConfig::default());
    assert_eq!(policy.max_lifetime_seconds, 0);
    assert_eq!(policy.minimum_lifetime_seconds, 120);
    assert_eq!(policy.idle_grace_seconds, 300);
    assert_eq!(policy.max_items, 5000);
}

#[test]
fn every_metadata_state_has_a_distinct_stable_spelling() {
    assert_eq!(RuntimeMetadataState::Complete.as_str(), "complete");
    assert_eq!(RuntimeMetadataState::Partial.as_str(), "partial");
    assert_eq!(RuntimeMetadataState::Malformed.as_str(), "malformed");
    assert_eq!(RuntimeMetadataState::Malformed.to_string(), "malformed");
}
