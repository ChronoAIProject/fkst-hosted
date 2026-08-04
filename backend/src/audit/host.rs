//! Validation for `FKST_POSTHOG_HOST`, shared by every process that reads it.
//!
//! Both deployables take the same variable and must judge it the same way: the
//! control plane for its capture sink and its activity query, and the
//! `fkst-audit-relay` binary for capture and verification. Two copies of this
//! rule would eventually disagree, and the disagreement would be silent — a
//! relay that accepted `http://…` while the control plane refused it ships the
//! project capture token in cleartext on every batch, and a relay that accepted
//! `https://svc:<token>@posthog.example` puts a credential in the ConfigMap
//! whose whole purpose is to hold no credential (epic `OPS-02`).
//!
//! Two entry points, because a host has two lifecycles:
//!
//! - [`normalize`] is the full check, applied whenever the host will actually be
//!   dialled: parseable `http`/`https` URL, a host component, no userinfo, and
//!   TLS unless the deployment explicitly names itself `test`/`local`.
//! - [`stage`] is for a host that is configured but inert (the control plane
//!   with `FKST_POSTHOG_ENABLED=false`). It judges nothing about shape, so a
//!   half-prepared rollout cannot fail an unrelated deploy — except userinfo,
//!   which is never acceptable at rest because the retained value is copied into
//!   `Debug` output and every config dump.

use crate::error::AppError;

/// The deployment environments in which a plaintext `http://` PostHog host is
/// tolerated. Everywhere else the request carries a credential, so TLS is
/// mandatory. Matched ASCII-case-insensitively after trimming.
pub(crate) const PLAINTEXT_ENVIRONMENTS: [&str; 2] = ["test", "local"];

/// One message for every host path, so an operator sees the same instruction
/// whichever check caught the credential.
pub(crate) const USERINFO_REJECTED: &str = "FKST_POSTHOG_HOST must not embed userinfo credentials";

/// Keep a staged (feature-off) host without judging its shape, minus the one
/// thing that is never acceptable: embedded userinfo.
pub(crate) fn stage(raw: &str) -> Result<String, AppError> {
    let staged = raw.trim_end_matches('/').to_string();
    if authority(&staged).contains('@') {
        return Err(AppError::Config(USERINFO_REJECTED.to_string()));
    }
    Ok(staged)
}

/// The authority component of a host value: everything after `scheme://` and
/// before the path. Deliberately textual rather than URL-parsed — a staged value
/// that does not parse at all can still carry a credential, and that is exactly
/// the case a parsed check would wave through.
fn authority(host: &str) -> &str {
    let after_scheme = host.split_once("://").map_or(host, |(_, rest)| rest);
    let end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    &after_scheme[..end]
}

/// Normalize + validate a host that will be dialled: strip trailing slashes,
/// require a parseable `http`/`https` URL with a host, forbid embedded userinfo,
/// and require HTTPS outside a `test`/`local` deployment environment.
pub(crate) fn normalize(raw: &str, environment: &str) -> Result<String, AppError> {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::Config(
            "FKST_POSTHOG_HOST must not be blank".to_string(),
        ));
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|e| AppError::Config(format!("FKST_POSTHOG_HOST must be a valid URL ({e})")))?;
    if url.host_str().is_none() {
        return Err(AppError::Config(
            "FKST_POSTHOG_HOST must include a host".to_string(),
        ));
    }
    // A credential embedded in the URL would be copied into every reqwest error,
    // proxy access log, and metric label derived from the host.
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Config(USERINFO_REJECTED.to_string()));
    }
    let plaintext_allowed = PLAINTEXT_ENVIRONMENTS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(environment));
    match url.scheme() {
        "https" => {}
        "http" if plaintext_allowed => {}
        "http" => {
            return Err(AppError::Config(format!(
                "FKST_POSTHOG_HOST must use https outside a {} deployment \
                 (FKST_DEPLOYMENT_ENVIRONMENT={environment:?}); a project or query \
                 credential rides every request to it",
                PLAINTEXT_ENVIRONMENTS.join("/")
            )))
        }
        other => {
            return Err(AppError::Config(format!(
                "FKST_POSTHOG_HOST must use http(s) (got scheme {other:?})"
            )))
        }
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
