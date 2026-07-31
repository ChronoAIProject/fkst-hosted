//! Telemetry tests: closed labels, every series rendered even when zero, and the
//! accepted/verified distinction preserved in the exposition.

use super::*;

#[test]
fn every_label_tuple_is_rendered_even_when_zero() {
    let metrics = RelayMetrics::new();
    let body = metrics.render(true);
    for kind in IngressKind::ALL {
        for result in IngressResult::ALL {
            let series = format!(
                "fkst_audit_relay_ingress_total{{kind=\"{}\",result=\"{}\"}} 0",
                kind.as_str(),
                result.as_str()
            );
            assert!(body.contains(&series), "missing `{series}`");
        }
    }
    for state in RecordState::ALL {
        assert!(body.contains(&format!(
            "fkst_audit_relay_records{{state=\"{}\"}} 0",
            state.as_str()
        )));
    }
}

#[test]
fn counters_increment_under_their_own_labels() {
    let metrics = RelayMetrics::new();
    metrics.record_ingress(IngressKind::RequestStart, IngressResult::Created);
    metrics.record_capture(CaptureResult::Accepted, 3);
    metrics.record_verification(VerificationResult::Verified, 2);
    metrics.record_dead_letter(DeadLetterReason::Permanent);
    metrics.record_incomplete(4);

    assert_eq!(
        metrics.ingress_count(IngressKind::RequestStart, IngressResult::Created),
        1
    );
    assert_eq!(metrics.capture_count(CaptureResult::Accepted), 3);
    assert_eq!(metrics.verification_count(VerificationResult::Verified), 2);
    assert_eq!(metrics.dead_letter_count(DeadLetterReason::Permanent), 1);
    assert_eq!(metrics.incomplete_count(), 4);
}

#[test]
fn capture_acceptance_and_query_verification_are_separate_families() {
    let metrics = RelayMetrics::new();
    metrics.record_capture(CaptureResult::Accepted, 1);
    let body = metrics.render(true);
    assert!(body.contains("fkst_audit_relay_capture_total{result=\"accepted\"} 1"));
    assert!(body.contains("fkst_audit_relay_verification_total{result=\"verified\"} 0"));
    // The word "delivered" must not appear anywhere: acceptance is not delivery.
    assert!(!body.contains("delivered"));
}

#[test]
fn the_gauge_block_is_published_and_read_back_consistently() {
    let metrics = RelayMetrics::new();
    let mut gauges = StorageGauges {
        db_bytes: 4_096,
        ..StorageGauges::default()
    };
    gauges.records[0] = 7;
    gauges.oldest_age_secs[0] = 42;
    metrics.publish(gauges.clone());
    assert_eq!(metrics.gauges(), gauges);
    let body = metrics.render(false);
    assert!(body.contains("fkst_audit_relay_db_bytes 4096"));
    assert!(body.contains("fkst_audit_relay_ingress_ready 0"));
    assert!(body.contains(&format!(
        "fkst_audit_relay_records{{state=\"{}\"}} 7",
        RecordState::ALL[0].as_str()
    )));
}
