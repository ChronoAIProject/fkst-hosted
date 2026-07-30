//! Expand a fkst-manifest reference into its concrete package list (epic #594 I6).
//!
//! A fkst-manifest is a JSON file committed in a repo and referenced — exactly like
//! a package — as `owner/repo@ref:path` (a [`PackageRef`]; see the `### Manifest`
//! section parsed by [`crate::goals::trigger_parse`]). It bundles a package list so a
//! trigger issue can name ONE manifest instead of enumerating every package inline.
//! The authored schema is a small object:
//!
//! ```json
//! { "schemaVersion": 1, "name": "default-workflows", "description": "…",
//!   "packages": [ "ChronoAIProject/fkst-hosted@packages:packages/workflow-dev", … ] }
//! ```
//!
//! This module FETCHES that JSON (a plain authenticated `contents` fetch, mirroring
//! [`crate::reconcile::work_labels`]) and VALIDATES it. Unlike work-label discovery,
//! this is **fail-closed**: a manifest is a required, complete package set, so ANY
//! failure — unreachable, missing, unparseable, wrong schema version, empty, oversized,
//! or a single malformed package reference — is a hard [`ManifestError`], never a
//! silently-empty result. Each `packages` entry is validated with the SAME grammar as
//! a `### Packages` line ([`parse_package_ref`]), so a manifest cannot smuggle in a
//! reference the trigger parser would itself reject.
//!
//! Scope boundary: this PR is the module + its tests only. It is NOT yet wired into
//! `plan_repo`/the reconcile sweep, and it does NOT resolve packages to directories —
//! that is a later PR. Secret hygiene: [`ManifestError`]'s `Display`/`Debug` never
//! carry the token, the URL, or transport detail.

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::goals::trigger_parse::{parse_package_ref, PackageRef};
use crate::reconcile::auth_fallback::should_retry_without_auth;

/// The only fkst-manifest schema version this expander understands. A manifest
/// declaring anything else is rejected rather than best-guessed (fail-closed).
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Hard ceiling on how many packages one manifest may declare. Bounds the fan-out
/// a single manifest reference can trigger (each package is later fetched/resolved),
/// so a hostile or accidental manifest cannot enqueue unbounded work.
const MAX_PACKAGES: usize = 64;

/// The authored fkst-manifest object. `#[serde(default)]` on `name`/`description`
/// (informational only) + NO `deny_unknown_fields` keeps the schema forward-compatible:
/// a future manifest carrying extra keys still parses. `schema_version` and `packages`
/// are REQUIRED — a manifest missing either is malformed and fails to deserialize
/// ([`ManifestError::Parse`]), which is the fail-closed outcome we want.
#[derive(Debug, Deserialize)]
struct FkstManifest {
    /// Schema discriminator; must equal [`SUPPORTED_SCHEMA_VERSION`].
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    /// Human-facing manifest name (e.g. `default-workflows`). Informational.
    #[serde(default)]
    name: String,
    /// Human-facing description. Informational.
    #[serde(default)]
    description: String,
    /// The bundled package references, each an `owner/repo@ref:path` string validated
    /// with [`parse_package_ref`].
    packages: Vec<String>,
    /// OPTIONAL per-package configuration: package name → (KEY → value). Supplies
    /// defaults for every session that uses this manifest, so a fleet-wide package
    /// setting is written once here instead of repeated in every trigger.
    ///
    /// `#[serde(default)]` keeps every existing manifest parsing byte-identically,
    /// and the key is validated with the SAME rules as the trigger's
    /// `### Package Env` section so the two surfaces cannot drift apart.
    #[serde(rename = "packageEnv", default)]
    package_env: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
}

/// Why a fkst-manifest could not be expanded. Every variant's `Display` (and derived
/// `Debug`) is **leak-free**: it never embeds the installation token, the fetch URL,
/// or raw transport detail — only safe context (a status code, the schema version, the
/// package count, or the offending package index + its grammar reason).
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The manifest could not be fetched — a transport failure or a non-404, non-2xx
    /// HTTP status. Carries only a curated, URL/token-free reason.
    #[error("failed to fetch fkst-manifest ({0})")]
    Fetch(String),
    /// The manifest path returned 404 — there is no manifest file at that ref/path.
    #[error("fkst-manifest not found at the referenced path")]
    NotFound,
    /// The fetched body was not valid fkst-manifest JSON (unparseable, or missing a
    /// required field such as `schemaVersion`/`packages`).
    #[error("fkst-manifest body is not valid manifest JSON")]
    Parse,
    /// `schemaVersion` was present but not [`SUPPORTED_SCHEMA_VERSION`].
    #[error("unsupported fkst-manifest schemaVersion {0} (expected {SUPPORTED_SCHEMA_VERSION})")]
    BadSchemaVersion(u32),
    /// The manifest declared an empty `packages` list — a manifest must contribute at
    /// least one package.
    #[error("fkst-manifest declares no packages")]
    Empty,
    /// The manifest declared more than [`MAX_PACKAGES`] packages.
    #[error("fkst-manifest declares {count} packages, exceeding the maximum of {max}")]
    TooMany { count: usize, max: usize },
    /// The `packages` entry at `index` (0-based) is not a valid `owner/repo@ref:path`
    /// reference. `detail` is the grammar reason from [`parse_package_ref`] (author
    /// content only — no token/URL).
    #[error("fkst-manifest package #{index} is an invalid reference: {detail}")]
    BadRef { index: usize, detail: String },
    /// The `packageEnv` block is malformed. `detail` is the same grammar reason a
    /// trigger author would see for the equivalent `### Package Env` mistake — the
    /// two surfaces share one validator so they cannot drift.
    #[error("fkst-manifest packageEnv is invalid: {detail}")]
    BadPackageEnv { detail: String },
}

/// Fetch the fkst-manifest JSON at `manifest_ref` and expand it into its validated
/// package list. FAIL-CLOSED: the returned `Vec` is always the manifest's COMPLETE,
/// well-formed package set — any shortfall is a [`ManifestError`].
///
/// `http` is the shared client, `api_base` the GitHub API root, and `token` a
/// repo-scoped installation (or user) token — mirroring
/// [`crate::reconcile::work_labels::resolve_work_labels`]. `manifest_ref.path` is the
/// JSON file path itself (NOT a directory — there is no `/fkst.toml` suffix, unlike a
/// package reference).
pub async fn expand_manifest(
    http: &reqwest::Client,
    api_base: &str,
    token: &SecretString,
    manifest_ref: &PackageRef,
) -> Result<ExpandedManifest, ManifestError> {
    let base = api_base.trim_end_matches('/');
    let body = fetch_manifest_json(http, base, token, manifest_ref).await?;

    let manifest: FkstManifest = serde_json::from_str(&body).map_err(|_| ManifestError::Parse)?;

    // Read `name`/`description` here so the manifest we fetched is traceable in logs
    // (and the informational fields are genuinely consumed, not dead weight).
    tracing::debug!(
        manifest_name = %manifest.name,
        manifest_description = %manifest.description,
        declared_packages = manifest.packages.len(),
        "fetched fkst-manifest; validating",
    );

    validate_manifest(manifest)
}

/// Validate a parsed manifest and turn its package strings into [`PackageRef`]s.
/// Checks run in a fail-closed order — schema version, then non-empty, then the size
/// ceiling, then each entry (rejecting on the FIRST malformed reference, naming its
/// index) — so the most fundamental defect is the one surfaced.
fn validate_manifest(manifest: FkstManifest) -> Result<ExpandedManifest, ManifestError> {
    if manifest.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(ManifestError::BadSchemaVersion(manifest.schema_version));
    }
    if manifest.packages.is_empty() {
        return Err(ManifestError::Empty);
    }
    if manifest.packages.len() > MAX_PACKAGES {
        return Err(ManifestError::TooMany {
            count: manifest.packages.len(),
            max: MAX_PACKAGES,
        });
    }

    let mut refs = Vec::with_capacity(manifest.packages.len());
    for (index, entry) in manifest.packages.iter().enumerate() {
        let package_ref = parse_package_ref(entry).map_err(|err| ManifestError::BadRef {
            index,
            detail: app_error_detail(err),
        })?;
        refs.push(package_ref);
    }

    // Validated with the trigger's own parser so a manifest can never express a
    // shape a trigger would reject (or vice versa). Rendering the map back to the
    // section's text form is deliberate: it means there is exactly ONE grammar and
    // one set of error messages for per-package configuration.
    let package_env = validate_package_env(&manifest.package_env)?;

    Ok(ExpandedManifest {
        packages: refs,
        package_env,
    })
}

/// A manifest after expansion: its package list plus any per-package configuration
/// it supplies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedManifest {
    pub packages: Vec<PackageRef>,
    pub package_env: crate::goals::package_env::PackageEnv,
}

/// Re-validate the authored `packageEnv` through the trigger-section parser.
///
/// The JSON is rendered back into the `### Package Env` text form and parsed, rather
/// than validated by a second hand-written checker: one grammar, one set of bounds,
/// one set of messages. A divergence here would let a manifest ship configuration a
/// trigger could not express, which is exactly the drift this avoids.
fn validate_package_env(
    authored: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
) -> Result<crate::goals::package_env::PackageEnv, ManifestError> {
    if authored.is_empty() {
        return Ok(crate::goals::package_env::PackageEnv::new());
    }
    let mut rendered = String::new();
    for (package, keys) in authored {
        rendered.push_str("#### ");
        rendered.push_str(package);
        rendered.push('\n');
        for (key, value) in keys {
            rendered.push_str(key);
            rendered.push('=');
            rendered.push_str(value);
            rendered.push('\n');
        }
    }
    crate::goals::package_env::parse_package_env(&rendered).map_err(|err| {
        ManifestError::BadPackageEnv {
            detail: app_error_detail(err),
        }
    })
}

/// Extract the client-safe message from the 422 [`parse_package_ref`] returns. The
/// parser only ever yields [`crate::error::AppError::Unprocessable`]; the fallback
/// keeps this total without leaking anything the other variants would (they never
/// occur here).
fn app_error_detail(err: crate::error::AppError) -> String {
    match err {
        crate::error::AppError::Unprocessable(message) => message,
        other => other.to_string(),
    }
}

/// GET the manifest JSON via the GitHub raw `contents` API. Distinguishes the three
/// fail-closed outcomes: a 404 → [`ManifestError::NotFound`]; any other transport or
/// non-2xx failure → [`ManifestError::Fetch`] carrying only a safe reason; success →
/// the raw body text. The transport error itself is discarded so its embedded URL can
/// never leak.
async fn fetch_manifest_json(
    http: &reqwest::Client,
    base: &str,
    token: &SecretString,
    manifest_ref: &PackageRef,
) -> Result<String, ManifestError> {
    let PackageRef {
        owner,
        repo,
        git_ref,
        path,
    } = manifest_ref;
    // The path IS the .json file — no `/fkst.toml` suffix (that is a package convention).
    let url = format!("{base}/repos/{owner}/{repo}/contents/{path}");
    let mut response = manifest_request(http, &url, git_ref, Some(token)).await?;
    if should_retry_without_auth(response.status()) {
        response = manifest_request(http, &url, git_ref, None).await?;
    }

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(ManifestError::NotFound);
    }
    if !status.is_success() {
        return Err(ManifestError::Fetch(format!("HTTP {}", status.as_u16())));
    }
    response
        .text()
        .await
        .map_err(|_| ManifestError::Fetch("body read error".to_string()))
}

async fn manifest_request(
    http: &reqwest::Client,
    url: &str,
    git_ref: &str,
    token: Option<&SecretString>,
) -> Result<reqwest::Response, ManifestError> {
    let request = http
        .get(url)
        .query(&[("ref", git_ref)])
        // `raw` returns the file bytes directly rather than the base64 envelope.
        .header(reqwest::header::ACCEPT, "application/vnd.github.raw")
        .header(reqwest::header::USER_AGENT, "fkst-hosted-api");
    let request = match token {
        Some(token) => request.bearer_auth(token.expose_secret()),
        None => request,
    };
    request
        .send()
        .await
        .map_err(|_| ManifestError::Fetch("transport error".to_string()))
}

#[cfg(test)]
#[path = "manifest_expand_tests.rs"]
mod tests;
