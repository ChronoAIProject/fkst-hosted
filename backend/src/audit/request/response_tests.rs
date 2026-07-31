//! Unit tests for the typed response markers.

use super::*;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::audit::validate;
use crate::error::AppError;

fn plain(status: StatusCode) -> Response {
    status.into_response()
}

#[test]
fn an_untagged_response_carries_no_markers() {
    let response = plain(StatusCode::OK);
    assert_eq!(error_code_of(&response), None);
    assert!(!is_rejected(&response));
}

#[test]
fn tagging_attaches_the_code_and_the_rejection_marker() {
    let response = with_error_code(plain(StatusCode::REQUEST_TIMEOUT), codes::REQUEST_TIMEOUT);
    assert_eq!(error_code_of(&response), Some(codes::REQUEST_TIMEOUT));
    assert!(!is_rejected(&response));

    let response = with_rejection(
        plain(StatusCode::SERVICE_UNAVAILABLE),
        codes::LEADER_NOT_READY,
    );
    assert_eq!(error_code_of(&response), Some(codes::LEADER_NOT_READY));
    assert!(is_rejected(&response));
}

/// The whole point of the extension: the middleware learns why a call failed
/// without touching a body that may be a multi-megabyte log stream.
#[test]
fn app_error_attaches_its_stable_code_without_the_message() {
    let response =
        AppError::NotFound("session sess-secret does not exist".to_string()).into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(error_code_of(&response), Some("not_found"));
    assert!(!is_rejected(&response));
}

#[test]
fn app_error_marks_identity_and_authorization_answers_as_rejections() {
    for (error, status, code) in [
        (
            AppError::Unauthorized("nope".to_string()),
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ),
        (
            AppError::Forbidden("nope".to_string()),
            StatusCode::FORBIDDEN,
            "forbidden",
        ),
        (
            AppError::ScopeForbidden("nope".to_string()),
            StatusCode::FORBIDDEN,
            "operations_scope_forbidden",
        ),
    ] {
        let response = error.into_response();
        assert_eq!(response.status(), status);
        assert_eq!(error_code_of(&response), Some(code));
        assert!(is_rejected(&response), "{code} must be a rejection");
    }
}

#[test]
fn a_validation_or_dependency_failure_is_not_a_rejection() {
    for error in [
        AppError::Validation("bad".to_string()),
        AppError::Unavailable("down".to_string()),
        AppError::SessionVisibilityUnavailable("cold".to_string()),
    ] {
        let response = error.into_response();
        assert!(
            !is_rejected(&response),
            "only identity/authorization answers are rejections"
        );
        assert!(error_code_of(&response).is_some());
    }
}

/// Every code that can reach a record must satisfy the event contract's stable
/// snake_case rule, or the sink would drop the record it was meant to explain.
#[test]
fn every_declared_code_is_a_valid_stable_error_code() {
    let mut declared = vec![
        codes::REQUEST_TIMEOUT,
        codes::LEADER_NOT_READY,
        codes::ROUTE_NOT_FOUND,
        codes::METHOD_NOT_ALLOWED,
        codes::WEBHOOK_SIGNATURE_INVALID,
        codes::WEBHOOK_NOT_CONFIGURED,
        codes::OAUTH_INVALID_REQUEST,
        codes::OAUTH_UNAUTHORIZED,
        codes::OAUTH_FORBIDDEN,
        codes::OAUTH_NOT_FOUND,
        codes::OAUTH_UNAVAILABLE,
        codes::OAUTH_UPSTREAM,
    ];
    for error in [
        AppError::Validation(String::new()),
        AppError::NotFound(String::new()),
        AppError::Conflict(String::new()),
        AppError::Unauthorized(String::new()),
        AppError::Forbidden(String::new()),
        AppError::ScopeForbidden(String::new()),
        AppError::Unprocessable(String::new()),
        AppError::RateLimited {
            message: String::new(),
            retry_after_secs: 1,
        },
        AppError::Upstream(String::new()),
        AppError::Unavailable(String::new()),
        AppError::SessionVisibilityUnavailable(String::new()),
        AppError::Config(String::new()),
    ] {
        let response = error.into_response();
        declared.push(error_code_of(&response).expect("every AppError tags a code"));
    }

    for code in declared {
        let mut event = crate::audit::test_support::anonymous_event();
        event.error_code = Some(code.to_string());
        validate::validate(&event).unwrap_or_else(|error| panic!("code {code}: {error}"));
    }
}

#[test]
fn browser_statuses_map_onto_bounded_oauth_codes() {
    for (status, expected) in [
        (StatusCode::UNAUTHORIZED, codes::OAUTH_UNAUTHORIZED),
        (StatusCode::FORBIDDEN, codes::OAUTH_FORBIDDEN),
        (StatusCode::NOT_FOUND, codes::OAUTH_NOT_FOUND),
        (StatusCode::SERVICE_UNAVAILABLE, codes::OAUTH_UNAVAILABLE),
        (StatusCode::BAD_GATEWAY, codes::OAUTH_UPSTREAM),
        (StatusCode::BAD_REQUEST, codes::OAUTH_INVALID_REQUEST),
    ] {
        assert_eq!(codes::for_browser_status(status), expected, "{status}");
    }
}
