//! Shared GitHub contents auth-fallback policy.
//!
//! GitHub can hide an otherwise-public repository from an installation/user token
//! that lacks access by returning 401, 403, or 404. The reconciler has three
//! contents-read paths (manifest expansion, work-label discovery, and package
//! reachability); all must apply the same authenticated-to-anonymous retry rule so
//! a public package outside the target repository installation is not misclassified
//! as missing.

/// Whether an authenticated GitHub contents response should be retried without
/// authentication. A final anonymous 404 remains a genuine not-found.
pub(crate) fn should_retry_without_auth(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED
            | reqwest::StatusCode::FORBIDDEN
            | reqwest::StatusCode::NOT_FOUND
    )
}
