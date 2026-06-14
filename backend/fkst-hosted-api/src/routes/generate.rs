//! Package generation HTTP edge: `POST /api/v1/packages/generate`.
//!
//! A user sends a natural-language `description`; the endpoint produces a
//! validated fkst package draft via NyxID's LLM gateway, optionally runs an
//! engine conformance dry-run, and optionally persists the draft as the
//! caller's own package. The `LlmGateway` trait is the only LLM seam.
//!
//! Status discipline:
//! - 400 — empty/oversize description, or an explicit invalid package name.
//! - 503 — generation not configured (no gateway) OR the gateway is unreachable.
//! - 409 — `save:true` collided with an existing package name.
//! - 200 — generation ran; the report carries the validation/conformance
//!   verdict (a draft that fails validation or conformance is STILL a 200).
//!
//! Security: the description, the prompt, and the model output are NEVER logged
//! — only byte sizes, file counts, and the conformance status.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;

use crate::auth::AuthContext;
use crate::error::AppError;
use crate::packages::{
    generate_package, is_valid_name, ConformanceStatus, GenerateReport, GenerateRequest,
    NewPackage, PackageError,
};
use crate::routes::extract::AppJson;
use crate::state::AppState;
use std::time::Duration;

/// Max accepted `description` length in bytes. A natural-language request is
/// short; this bounds prompt size and request memory.
const MAX_DESCRIPTION_BYTES: usize = 8192;

/// Request body for `POST /api/v1/packages/generate`. Unknown fields are denied
/// so client typos fail loudly.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratePackageRequest {
    pub description: String,
    /// Explicit package name; when omitted a unique `gen-<hex>` name is minted.
    #[serde(default)]
    pub name: Option<String>,
    /// Persist the draft as the caller's own package when it validates (and
    /// conformance did not fail).
    #[serde(default)]
    pub save: bool,
}

/// Resolve the package name: an explicit name must pass the identity rule; an
/// absent name yields a unique `gen-<8 hex>` (via `bson::Uuid`, no `uuid` dep).
fn resolve_name(requested: Option<String>) -> Result<String, AppError> {
    match requested {
        Some(name) => {
            if !is_valid_name(&name) {
                return Err(AppError::Validation(
                    "invalid package name: must fully match [A-Za-z0-9_-]+".to_string(),
                ));
            }
            Ok(name)
        }
        None => Ok(format!(
            "gen-{}",
            &bson::Uuid::new().to_string().replace('-', "")[..8]
        )),
    }
}

/// `POST /api/v1/packages/generate`.
async fn generate(
    State(state): State<AppState>,
    ctx: AuthContext,
    AppJson(req): AppJson<GeneratePackageRequest>,
) -> Result<(StatusCode, Json<GenerateReport>), AppError> {
    // 1. Description bounds (never log the description itself).
    if req.description.is_empty() || req.description.len() > MAX_DESCRIPTION_BYTES {
        return Err(AppError::Validation(
            "description must be 1..=8192 bytes".to_string(),
        ));
    }

    // 2. Resolve + validate the package name.
    let name = resolve_name(req.name)?;

    // 3. Gateway must be configured (503 when generation is disabled).
    let gateway = state
        .llm
        .as_ref()
        .ok_or_else(|| AppError::Unavailable("package generation is not configured".into()))?;

    tracing::info!(
        description_bytes = req.description.len(),
        save = req.save,
        "package generation requested"
    );

    // 4. Generate (gateway failure bubbles up as a 503).
    let request = GenerateRequest {
        description: req.description,
        name,
        save: req.save,
        max_output_bytes: state.config.llm_max_output_bytes,
    };
    let budget = Duration::from_secs(state.config.request_timeout_secs);
    let mut report = generate_package(gateway.as_ref(), &state.engine, budget, &request).await?;

    // 5. Optional persistence — only a validated, conformance-non-failed draft
    //    is saved; everything else records a `save_error` and stays unsaved.
    if request.save {
        if !report.validation.ok {
            report.save_error = Some("validation failed".into());
        } else if report.conformance.status == ConformanceStatus::Failed {
            report.save_error = Some("conformance failed".into());
        } else {
            let np = NewPackage {
                name: report.package.name.clone(),
                files: report.package.files.clone(),
                composed_deps: report.package.composed_deps.clone(),
            };
            match state.packages.create(np, &ctx.user_id, None).await {
                Ok(_) => report.saved = true,
                Err(PackageError::Duplicate(n)) => {
                    return Err(AppError::Conflict(format!("package already exists: {n}")));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    tracing::info!(
        file_count = report.package.files.len(),
        validation_ok = report.validation.ok,
        conformance = ?report.conformance.status,
        attempts = report.attempts,
        saved = report.saved,
        "package generation completed"
    );

    // 6. ALWAYS 200 when generation ran.
    Ok((StatusCode::OK, Json(report)))
}

/// Router for the generation endpoint.
pub fn router() -> Router<AppState> {
    Router::new().route("/packages/generate", post(generate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_name_accepts_a_valid_explicit_name() {
        assert_eq!(
            resolve_name(Some("My-Pkg_01".to_string())).expect("valid"),
            "My-Pkg_01"
        );
    }

    #[test]
    fn resolve_name_rejects_an_invalid_explicit_name() {
        let err = resolve_name(Some("bad name".to_string())).expect_err("invalid");
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn resolve_name_mints_a_valid_generated_name_when_absent() {
        let name = resolve_name(None).expect("minted");
        assert!(name.starts_with("gen-"), "got {name}");
        assert!(is_valid_name(&name), "minted name must be valid: {name}");
        assert_eq!(name.len(), "gen-".len() + 8);
    }

    #[test]
    fn resolve_name_mints_unique_names() {
        let a = resolve_name(None).expect("a");
        let b = resolve_name(None).expect("b");
        assert_ne!(a, b, "generated names must be unique");
    }
}
