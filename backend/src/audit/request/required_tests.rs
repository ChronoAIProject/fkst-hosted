//! The two required-mode responses: their stable codes, their statuses, and the
//! rejection marker that distinguishes "nothing happened" from "something may
//! have".

use axum::http::StatusCode;

use super::*;
use crate::audit::request::response::{AuditErrorCode, AuditRejection};

#[test]
fn an_unavailable_ingress_is_a_pre_handler_rejection() {
    let response = ingress_unavailable();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.extensions().get::<AuditErrorCode>(),
        Some(&AuditErrorCode(codes::AUDIT_INGRESS_UNAVAILABLE))
    );
    assert!(
        response.extensions().get::<AuditRejection>().is_some(),
        "nothing ran, so it is a rejection"
    );
}

#[test]
fn an_unconfirmed_completion_is_not_a_rejection() {
    let response = completion_unconfirmed();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.extensions().get::<AuditErrorCode>(),
        Some(&AuditErrorCode(codes::AUDIT_COMPLETION_UNCONFIRMED))
    );
    assert!(
        response.extensions().get::<AuditRejection>().is_none(),
        "the handler ran; calling it a pre-handler rejection would misreport it"
    );
}

#[test]
fn the_two_codes_are_distinct_and_stable() {
    assert_eq!(
        codes::AUDIT_INGRESS_UNAVAILABLE,
        "audit_ingress_unavailable"
    );
    assert_eq!(
        codes::AUDIT_COMPLETION_UNCONFIRMED,
        "audit_completion_unconfirmed"
    );
    assert_ne!(
        codes::AUDIT_INGRESS_UNAVAILABLE,
        codes::AUDIT_COMPLETION_UNCONFIRMED
    );
}
