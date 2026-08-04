//! Safe arguments for the named-environment profile API.
//!
//! A profile is, by design, a bag of secrets: install commands that may embed a
//! registry credential, variable names that leak infrastructure topology, and
//! secret keys and values that are the credentials themselves. So the record
//! keeps the environment's NAME — the only value the store validates into a
//! bounded, non-secret identifier — plus three counts.
//!
//! The spec is explicit that even non-secret variable VALUES are not valid audit
//! properties despite the product API returning them, and that nothing may be
//! hashed: a guessable hash of a short value is the value. Neither appears here.
//! The validation Pod's stdout/stderr, which the `422` body surfaces to the
//! caller, is likewise never an argument.

use serde::Serialize;

use super::bounds::safe_environment_name;
use super::catalog;
use super::{sealed::Sealed, BoundedAuditArguments, ToSafeAuditArguments};
use crate::audit::event::ArgumentsParseStatus;

/// `put_user_environment_profile` — validate and persist one environment.
#[derive(Clone, Debug, Serialize)]
pub struct SafePutEnvironmentProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    environment_name: Option<String>,
    install_command_count: usize,
    variable_count: usize,
    secret_count: usize,
}

impl BoundedAuditArguments for SafePutEnvironmentProfile {
    const OPERATION_ID: &'static str = "put_user_environment_profile";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::PUT_USER_ENVIRONMENT_PROFILE_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.environment_name.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

/// The input view for `put_user_environment_profile`, built from the already
/// shape-validated name and the spec's collection sizes.
pub struct PutEnvironmentProfileInput<'a> {
    pub environment_name: &'a str,
    pub install_command_count: usize,
    pub variable_count: usize,
    pub secret_count: usize,
}

impl Sealed for PutEnvironmentProfileInput<'_> {}

impl ToSafeAuditArguments for PutEnvironmentProfileInput<'_> {
    type Safe = SafePutEnvironmentProfile;

    fn to_safe_audit_arguments(&self) -> Self::Safe {
        SafePutEnvironmentProfile {
            environment_name: safe_environment_name(self.environment_name),
            install_command_count: self.install_command_count,
            variable_count: self.variable_count,
            secret_count: self.secret_count,
        }
    }
}

/// The name-only shape shared by the read and delete operations.
///
/// They are separate types rather than one reused DTO because
/// [`BoundedAuditArguments::OPERATION_ID`] binds a DTO to exactly one operation —
/// that binding is what lets the coverage guard prove no operation has two
/// policies and no policy covers two operations.
macro_rules! environment_name_arguments {
    ($ty:ident, $operation:literal, $fields:path, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Serialize)]
        pub struct $ty {
            #[serde(skip_serializing_if = "Option::is_none")]
            environment_name: Option<String>,
        }

        impl $ty {
            pub fn new(environment_name: &str) -> Self {
                Self {
                    environment_name: safe_environment_name(environment_name),
                }
            }
        }

        impl BoundedAuditArguments for $ty {
            const OPERATION_ID: &'static str = $operation;
            const ALLOWED_FIELDS: &'static [&'static str] = $fields;

            fn parse_status(&self) -> ArgumentsParseStatus {
                if self.environment_name.is_some() {
                    ArgumentsParseStatus::Parsed
                } else {
                    ArgumentsParseStatus::Invalid
                }
            }
        }
    };
}

environment_name_arguments!(
    SafeGetEnvironmentProfile,
    "get_user_environment_profile",
    catalog::GET_USER_ENVIRONMENT_PROFILE_FIELDS,
    "`get_user_environment_profile` — read one environment (never its values)."
);

environment_name_arguments!(
    SafeDeleteEnvironmentProfile,
    "delete_user_environment_profile",
    catalog::DELETE_USER_ENVIRONMENT_PROFILE_FIELDS,
    "`delete_user_environment_profile` — remove one environment."
);

#[cfg(test)]
#[path = "environments_tests.rs"]
mod tests;
