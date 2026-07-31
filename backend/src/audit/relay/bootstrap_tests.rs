//! Startup-wiring tests: when a client exists, and when the lifecycle queue is
//! actually attached.

use secrecy::SecretString;

use super::super::AuditDeliveryMode;
use super::*;

fn configured(mode: AuditDeliveryMode) -> AuditDeliveryConfig {
    AuditDeliveryConfig {
        mode,
        relay_url: Some("http://relay.internal:8090".to_string()),
        write_token: SecretString::from("write-secret".to_string()),
        read_token: SecretString::from("read-secret".to_string()),
        ..AuditDeliveryConfig::default()
    }
}

#[test]
fn no_configured_half_builds_no_client() {
    let client = client(&AuditDeliveryConfig::default(), RelayClientMetrics::new())
        .expect("an unconfigured relay is not an error");
    assert!(client.is_none());
}

#[test]
fn a_configured_write_half_builds_one_client() {
    let client = client(
        &configured(AuditDeliveryMode::Required),
        RelayClientMetrics::new(),
    )
    .expect("the client builds")
    .expect("a configured relay yields a client");
    assert_eq!(Arc::strong_count(&client), 1);
}

#[test]
fn a_read_only_deployment_still_builds_a_client_for_the_activity_merge() {
    // Delivery stays disabled, but the operations read must still be able to
    // reach the relay: the two halves are independently configurable.
    let read_only = AuditDeliveryConfig {
        mode: AuditDeliveryMode::Disabled,
        relay_url: Some("http://relay.internal:8090".to_string()),
        read_token: SecretString::from("read-secret".to_string()),
        ..AuditDeliveryConfig::default()
    };
    assert!(client(&read_only, RelayClientMetrics::new())
        .expect("the client builds")
        .is_some());
}

#[tokio::test]
async fn disabled_delivery_never_attaches_the_lifecycle_queue() {
    let audit = AuditHandle::disabled();
    let built = client(
        &configured(AuditDeliveryMode::Disabled),
        RelayClientMetrics::new(),
    )
    .expect("the client builds");
    let wired = with_lifecycle_relay(
        audit,
        built.as_ref(),
        &configured(AuditDeliveryMode::Disabled),
    );
    assert!(
        !wired.lifecycle_relay_attached(),
        "a disabled deployment must keep its local lifecycle path"
    );
}

#[tokio::test]
async fn a_relay_mode_attaches_the_lifecycle_queue() {
    let audit = AuditHandle::disabled();
    let built = client(
        &configured(AuditDeliveryMode::Required),
        RelayClientMetrics::new(),
    )
    .expect("the client builds");
    let wired = with_lifecycle_relay(
        audit,
        built.as_ref(),
        &configured(AuditDeliveryMode::Required),
    );
    assert!(wired.lifecycle_relay_attached());
}
