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
//!   "packages": [ "ChronoAIProject/fkst-packages@fkst-hosted:packages/workflow-dev", … ] }
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
) -> Result<Vec<PackageRef>, ManifestError> {
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
fn validate_manifest(manifest: FkstManifest) -> Result<Vec<PackageRef>, ManifestError> {
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
    Ok(refs)
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
    let response = http
        .get(&url)
        .query(&[("ref", git_ref.as_str())])
        // `raw` returns the file bytes directly rather than the base64 envelope.
        .header(reqwest::header::ACCEPT, "application/vnd.github.raw")
        .header(reqwest::header::USER_AGENT, "fkst-hosted-api")
        .bearer_auth(token.expose_secret())
        .send()
        .await
        .map_err(|_| ManifestError::Fetch("transport error".to_string()))?;

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

#[cfg(test)]
#[path = "manifest_expand_tests.rs"]
mod tests;
