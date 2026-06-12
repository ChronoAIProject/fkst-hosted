//! Package HTTP API: CRUD + zip archive upload for `/api/v1/packages`.
//!
//! Endpoints:
//! - `POST   /api/v1/packages`              — create (JSON)
//! - `GET    /api/v1/packages`              — list names
//! - `GET    /api/v1/packages/{name}`       — fetch one
//! - `PUT    /api/v1/packages/{name}`       — update (JSON)
//! - `DELETE /api/v1/packages/{name}`       — delete (204)
//! - `POST   /api/v1/packages/{name}/archive`  — create from zip
//! - `PUT    /api/v1/packages/{name}/archive`   — replace from zip
//!
//! This is purely the web edge: wire DTOs, the body-size limit, and the
//! status mapping. All authoritative validation (name rule, path-safety
//! security guards, size caps, engine-entry rule) lives in the packages
//! domain (`NewPackage::validate` / `PackageRepository`) and is surfaced
//! here through `From<PackageError> for AppError`.
//!
//! # Update semantics (snapshot)
//!
//! Sessions materialize package files **at spawn** — a PUT affects only
//! sessions started afterwards; no running-session invalidation. No engine
//! interaction occurs during update.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::routing::{get, put};
use axum::{Json, Router};
use bson::doc;
use serde::{Deserialize, Serialize};

use crate::auth::AuthContext;
use crate::authz::{Action, Ownership};
use crate::distribution::active_status_bson;
use crate::error::AppError;
use crate::packages::{
    is_valid_name, package_from_zip, NewPackage, Package, PackageFile, MAX_TOTAL_CONTENT_BYTES,
};
use crate::routes::extract::AppJson;
use crate::routes::rfc3339;
use crate::state::AppState;

/// Request-body cap for the packages routes (16 MiB).
///
/// This is a *wire-size* DoS guard, not the authoritative size rule. The
/// domain caps total *decoded* file content at `MAX_TOTAL_CONTENT_BYTES`
/// (12 MiB), and it is that decoded-size cap — not this one — that keeps the
/// stored document under MongoDB's 16 MiB BSON limit. The 4 MiB headroom
/// covers paths (up to 128 KiB), `composed_deps`, the name, and typical JSON
/// framing. Note that JSON escaping can inflate wire size well beyond raw
/// content size (`\n` is 2 wire bytes, `\uXXXX` is 6), so an escape-dense
/// package that is *legal* by the decoded-size rules CAN exceed this cap and
/// be rejected — an accepted v1 trade-off in exchange for bounding request
/// memory before deserialization. Over-limit bodies surface as `400 "request
/// body too large"` via [`AppJson`] (deliberately not `413`).
pub const MAX_REQUEST_BODY_BYTES: usize = MAX_TOTAL_CONTENT_BYTES + 4 * 1024 * 1024;

// ---- DTOs ---------------------------------------------------------------

/// Request body for `POST /api/v1/packages`. Unlike the forgiving domain
/// `NewPackage`, the API edge denies unknown fields so client typos (e.g.
/// `"file"` for `"files"`) fail loudly with a `400`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePackageRequest {
    pub name: String,
    pub files: Vec<PackageFile>,
    #[serde(default)]
    pub composed_deps: Vec<String>,
    /// Optional org to attach the package to. When present, the caller
    /// must be an admin or member of that org.
    #[serde(default)]
    pub org_id: Option<String>,
}

/// Request body for `PUT /api/v1/packages/{name}`. The name comes from the
/// URL path only (not the body), so body typos on the name field are
/// structurally impossible. Unknown fields are denied.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePackageRequest {
    pub files: Vec<PackageFile>,
    #[serde(default)]
    pub composed_deps: Vec<String>,
}

/// Response body for `POST /api/v1/packages` (201).
#[derive(Debug, Serialize)]
pub struct CreatePackageResponse {
    pub name: String,
}

/// Response body for `GET /api/v1/packages/{name}` (200). Timestamps are
/// RFC3339 UTC strings with a trailing `Z`.
#[derive(Debug, Serialize)]
pub struct PackageResponse {
    pub name: String,
    pub files: Vec<PackageFile>,
    pub composed_deps: Vec<String>,
    /// Owner user ID (explicit null for legacy packages).
    pub owner_user_id: Option<String>,
    /// Organization ID (explicit null for personal packages).
    pub org_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl TryFrom<Package> for PackageResponse {
    type Error = AppError;

    fn try_from(package: Package) -> Result<Self, Self::Error> {
        let created_at = rfc3339(package.created_at)?;
        let updated_at = rfc3339(package.updated_at)?;
        Ok(PackageResponse {
            name: package.name,
            files: package.files,
            composed_deps: package.composed_deps,
            owner_user_id: package.owner_user_id,
            org_id: package.org_id,
            created_at,
            updated_at,
        })
    }
}

// ---- Helpers ------------------------------------------------------------

/// Validate a package name from a URL path segment. Returns `AppError::Validation`
/// on failure.
fn validate_path_name(name: &str) -> Result<(), AppError> {
    if !is_valid_name(name) {
        tracing::warn!(name = ?name, "package name rejected");
        return Err(AppError::Validation(
            "invalid package name: must fully match [A-Za-z0-9_-]+".to_string(),
        ));
    }
    Ok(())
}

/// Verify that the Content-Type header is `application/zip`.
fn require_zip_content_type(headers: &HeaderMap) -> Result<(), AppError> {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    // Compare only the media type, ignoring parameters like charset/boundary.
    let media_type = ct.split(';').next().unwrap_or("").trim();
    if media_type != "application/zip" {
        return Err(AppError::Validation(
            "Content-Type must be application/zip".to_string(),
        ));
    }
    Ok(())
}

/// Check whether a package has active sessions or a live lease.
/// Returns `Ok(())` if clear, `Err(AppError::Conflict)` otherwise.
///
/// The check-then-delete TOCTOU window is accepted: the engine driver already
/// handles "package disappeared before start" gracefully, and the reaper
/// fails active sessions whose package vanished.
async fn check_active_usage(state: &AppState, name: &str) -> Result<(), AppError> {
    // Check for live lease.
    let lease_filter = doc! {
        "_id": name,
        "expires_at": { "$gt": bson::DateTime::now() }
    };
    let has_live_lease = state
        .db
        .leases()
        .find_one(lease_filter)
        .await
        .map_err(AppError::Mongo)?
        .is_some();

    if has_live_lease {
        return Err(AppError::Conflict(format!(
            "package {name} has an active session or live lease"
        )));
    }

    // Check for active sessions.
    let session_filter = doc! {
        "package_name": name,
        "status": { "$in": active_status_bson() }
    };
    let active_count = state
        .db
        .sessions()
        .count_documents(session_filter)
        .await
        .map_err(AppError::Mongo)?;

    if active_count > 0 {
        return Err(AppError::Conflict(format!(
            "package {name} has an active session or live lease"
        )));
    }

    Ok(())
}

// ---- Handlers -----------------------------------------------------------

/// `POST /api/v1/packages`: validate (domain), insert, answer `201` with a
/// `Location` header. Duplicates are arbitrated solely by the Mongo `_id`
/// uniqueness (no read-then-write): the loser of a race gets a `409`.
async fn create(
    State(state): State<AppState>,
    ctx: AuthContext,
    AppJson(request): AppJson<CreatePackageRequest>,
) -> Result<
    (
        StatusCode,
        [(HeaderName, HeaderValue); 1],
        Json<CreatePackageResponse>,
    ),
    AppError,
> {
    // If body has org_id, require the caller to be an org writer.
    if let Some(ref org_id) = request.org_id {
        state.authz.require_org_writer(&ctx, org_id).await?;
    }

    // NEVER log content; paths/sizes/counts only (the repository logs the
    // accepted package at INFO).
    tracing::debug!(
        name = ?request.name,
        files = request.files.len(),
        composed_deps = request.composed_deps.len(),
        org_id = ?request.org_id,
        "package create requested"
    );
    let created = state
        .packages
        .create(
            NewPackage {
                name: request.name,
                files: request.files,
                composed_deps: request.composed_deps,
            },
            &ctx.user_id,
            request.org_id.as_deref(),
        )
        .await?;

    // The name passed domain validation ([A-Za-z0-9_-]+), so it is ASCII and
    // header-safe by construction; failure here is unreachable.
    let location = HeaderValue::try_from(format!("/api/v1/packages/{}", created.name))
        .expect("validated package name is ASCII and header-safe");
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(CreatePackageResponse { name: created.name }),
    ))
}

/// `GET /api/v1/packages`: flat JSON array of names (repository order:
/// ascending). Empty store answers `[]`. Filters to visible packages
/// based on caller identity and org memberships.
async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<String>>, AppError> {
    let org_ids = state.authz.visible_org_ids(&ctx).await?;
    let names = state.packages.list_visible(&ctx.user_id, &org_ids).await?;
    tracing::info!(count = names.len(), "packages listed");
    Ok(Json(names))
}

/// `GET /api/v1/packages/{name}`: fetch one package or `404`.
async fn get_one(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(name): Path<String>,
) -> Result<Json<PackageResponse>, AppError> {
    // Axum percent-decodes the segment before `Path<String>` sees it, and the
    // anchored name rule forbids `/`, `.`, `$`, and whitespace — so a decoded
    // traversal (`..%2Fetc` -> "../etc") or operator injection fails here and
    // never reaches Mongo. Do not weaken this guard: the name is used
    // directly as a Mongo `_id` lookup.
    if !is_valid_name(&name) {
        tracing::warn!(name = ?name, "package name rejected");
        return Err(AppError::Validation(
            "invalid package name: must fully match [A-Za-z0-9_-]+".to_string(),
        ));
    }
    match state.packages.get(&name).await? {
        Some(package) => {
            let ownership = Ownership {
                owner_user_id: package.owner_user_id.as_deref(),
                org_id: package.org_id.as_deref(),
            };
            state
                .authz
                .authorize(&ctx, ownership, Action::Read, "package", &name)
                .await?;
            tracing::info!(name = %name, "package fetched");
            Ok(Json(PackageResponse::try_from(package)?))
        }
        None => Err(AppError::NotFound(format!("package not found: {name}"))),
    }
}

/// `PUT /api/v1/packages/{name}`: atomically replace files and composed_deps.
///
/// The name comes from the path; `created_at` and ownership fields are
/// untouched. Requires write permission on the package. Snapshot semantics:
/// only sessions started after this call see the new files.
async fn update(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(name): Path<String>,
    AppJson(body): AppJson<UpdatePackageRequest>,
) -> Result<Json<PackageResponse>, AppError> {
    validate_path_name(&name)?;

    // Fetch existing for authz check.
    let existing = state
        .packages
        .get(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("package not found: {name}")))?;

    let ownership = Ownership {
        owner_user_id: existing.owner_user_id.as_deref(),
        org_id: existing.org_id.as_deref(),
    };
    state
        .authz
        .authorize(&ctx, ownership, Action::Write, "package", &name)
        .await?;

    tracing::debug!(
        name = %name,
        files = body.files.len(),
        composed_deps = body.composed_deps.len(),
        "package update requested"
    );

    let new_package = NewPackage {
        name,
        files: body.files,
        composed_deps: body.composed_deps,
    };
    let updated = state.packages.replace(new_package).await?;
    Ok(Json(PackageResponse::try_from(updated)?))
}

/// `DELETE /api/v1/packages/{name}`: remove a package.
///
/// Returns `409` when the package has an active session or a live lease.
/// Requires manage permission on the package.
async fn delete_one(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(name): Path<String>,
) -> Result<StatusCode, AppError> {
    validate_path_name(&name)?;

    // Fetch existing for authz check.
    let existing = state
        .packages
        .get(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("package not found: {name}")))?;

    let ownership = Ownership {
        owner_user_id: existing.owner_user_id.as_deref(),
        org_id: existing.org_id.as_deref(),
    };
    state
        .authz
        .authorize(&ctx, ownership, Action::Manage, "package", &name)
        .await?;

    // Active-usage guard. The check-then-delete TOCTOU window is accepted:
    // the engine driver handles "package disappeared before start" and the
    // reaper fails sessions whose package vanished.
    check_active_usage(&state, &name).await?;

    let deleted = state.packages.delete(&name).await?;
    if deleted {
        tracing::info!(name = %name, "package deleted");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound(format!("package not found: {name}")))
    }
}

/// `POST /api/v1/packages/{name}/archive`: create a package from a zip archive.
///
/// The body must be raw `application/zip` bytes. Stamps ownership exactly
/// like JSON create (honors the `org_id` convention; archive create has no
/// body fields, so org attachment is a non-goal for this path).
async fn archive_create(
    State(state): State<AppState>,
    ctx: AuthContext,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<
    (
        StatusCode,
        [(HeaderName, HeaderValue); 1],
        Json<CreatePackageResponse>,
    ),
    AppError,
> {
    validate_path_name(&name)?;
    require_zip_content_type(&headers)?;

    tracing::debug!(name = %name, size = body.len(), "package archive create requested");

    let new_package = package_from_zip(&name, &body).map_err(AppError::Validation)?;

    let created = state
        .packages
        .create(new_package, &ctx.user_id, None)
        .await?;

    let location = HeaderValue::try_from(format!("/api/v1/packages/{}", created.name))
        .expect("validated package name is ASCII and header-safe");
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(CreatePackageResponse { name: created.name }),
    ))
}

/// `PUT /api/v1/packages/{name}/archive`: replace a package from a zip archive.
///
/// The body must be raw `application/zip` bytes. Requires write permission
/// on the package. Snapshot semantics: only sessions started after this call
/// see the new files.
async fn archive_replace(
    State(state): State<AppState>,
    ctx: AuthContext,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Json<PackageResponse>, AppError> {
    validate_path_name(&name)?;
    require_zip_content_type(&headers)?;

    // Fetch existing for authz check.
    let existing = state
        .packages
        .get(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("package not found: {name}")))?;

    let ownership = Ownership {
        owner_user_id: existing.owner_user_id.as_deref(),
        org_id: existing.org_id.as_deref(),
    };
    state
        .authz
        .authorize(&ctx, ownership, Action::Write, "package", &name)
        .await?;

    tracing::debug!(name = %name, size = body.len(), "package archive replace requested");

    let new_package = package_from_zip(&name, &body).map_err(AppError::Validation)?;

    let updated = state.packages.replace(new_package).await?;
    Ok(Json(PackageResponse::try_from(updated)?))
}

// ---- Router -------------------------------------------------------------

/// Package routes, to be nested under `/api/v1`. The body-limit layer is
/// scoped to these routes only (GETs carry no body; the limit is harmless
/// there and keeps the layer wiring simple).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/packages", get(list).post(create))
        .route(
            "/packages/:name",
            get(get_one).put(update).delete(delete_one),
        )
        .route(
            "/packages/:name/archive",
            put(archive_replace).post(archive_create),
        )
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_response_serializes_to_the_documented_shape() {
        let body = serde_json::to_value(CreatePackageResponse {
            name: "billing-pipeline".to_string(),
        })
        .unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "name": "billing-pipeline"
            })
        );
    }

    #[test]
    fn package_response_emits_explicit_nulls_for_ownership() {
        let package = Package {
            name: "demo".to_string(),
            files: vec![],
            composed_deps: vec![],
            owner_user_id: None,
            org_id: None,
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            updated_at: bson::DateTime::from_millis(1_700_000_000_000),
        };
        let view = PackageResponse::try_from(package).expect("view");
        let body = serde_json::to_value(&view).unwrap();
        assert!(body["owner_user_id"].is_null(), "must be explicit null");
        assert!(body["org_id"].is_null(), "must be explicit null");
    }
}
