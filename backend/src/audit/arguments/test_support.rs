//! Shared assertions for the safe-argument DTO tests (never in the binary).
//!
//! Every DTO test goes through [`properties`] rather than through
//! `serde_json::to_value`, because the property map a record actually carries is
//! the one the allowlist filter produced — testing the raw serialization would
//! prove the wrong thing.

use serde_json::{Map, Value};

use super::{allowlisted, BoundedAuditArguments};
use crate::audit::request::policy::{arguments_policy_for, RESERVED_ARGUMENT_POLICIES};

/// The exact property map [`super::record_safe`] would record for `safe`.
pub fn properties<A: BoundedAuditArguments>(safe: &A) -> Map<String, Value> {
    allowlisted(safe)
}

/// The property `key` as a string, or `None` when it was not emitted.
pub fn string(values: &Map<String, Value>, key: &str) -> Option<String> {
    values.get(key)?.as_str().map(str::to_string)
}

/// Prove a DTO is wired to the ONE declared policy for its operation, and that
/// its allowlist is the catalog's list rather than a second copy.
///
/// Called from every DTO's own tests, so a DTO that grows a field without
/// updating [`super::catalog`] fails in the module that changed rather than only
/// in the integration guard.
pub fn assert_policy_matches<A: BoundedAuditArguments>() {
    let declared = arguments_policy_for(A::OPERATION_ID)
        .map(|policy| policy.spec())
        .unwrap_or_else(|| {
            RESERVED_ARGUMENT_POLICIES
                .iter()
                .find(|operation| operation.operation_id == A::OPERATION_ID)
                .map(|operation| operation.arguments.spec())
                .unwrap_or_else(|| {
                    panic!("{} has no declared safe-argument policy", A::OPERATION_ID)
                })
        })
        .unwrap_or_else(|| {
            panic!(
                "{} declares no named DTO for its arguments",
                A::OPERATION_ID
            )
        });
    assert_eq!(
        declared.fields,
        A::ALLOWED_FIELDS,
        "{}'s DTO allowlist must BE the catalog's list, not a copy of it",
        A::OPERATION_ID
    );
}

/// Prove a rendered DTO emitted nothing outside its documented allowlist.
pub fn assert_within_allowlist<A: BoundedAuditArguments>(safe: &A) {
    for key in properties(safe).keys() {
        assert!(
            A::ALLOWED_FIELDS.contains(&key.as_str()),
            "{} emitted the undocumented property {key}",
            A::OPERATION_ID
        );
    }
}

/// Prove none of `canaries` appears anywhere in the rendered properties.
///
/// Serialized whole rather than checked per field, so a canary smuggled into a
/// nested object or an array element is caught too.
pub fn assert_no_canary<A: BoundedAuditArguments>(safe: &A, canaries: &[&str]) {
    let rendered = serde_json::to_string(&properties(safe)).expect("properties serialize");
    for canary in canaries {
        assert!(
            !rendered.contains(canary),
            "{} leaked {canary} into its arguments: {rendered}",
            A::OPERATION_ID
        );
    }
}

/// Re-exported so a DTO test can assert the default status the middleware would
/// apply for its operation without importing the request module itself.
pub fn default_status(operation_id: &str) -> crate::audit::event::ArgumentsParseStatus {
    crate::audit::request::policy::default_arguments_status(operation_id)
}
