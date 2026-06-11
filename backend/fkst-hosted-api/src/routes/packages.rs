//! Package HTTP API: `POST/GET /api/v1/packages` and
//! `GET /api/v1/packages/{name}`.
//!
//! This is purely the web edge: wire DTOs, the body-size limit, and the
//! status mapping. All authoritative validation (name rule, path-safety
//! security guards, size caps, engine-entry rule) lives in the packages
//! domain (`NewPackage::validate` / `PackageRepository`) and is surfaced
//! here through `From<PackageError> for AppError`.

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::packages::{is_valid_name, NewPackage, Package, PackageFile, MAX_TOTAL_CONTENT_BYTES};
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
            created_at,
            updated_at,
        })
    }
}

/// `POST /api/v1/packages`: validate (domain), insert, answer `201` with a
/// `Location` header. Duplicates are arbitrated solely by the Mongo `_id`
/// uniqueness (no read-then-write): the loser of a race gets a `409`.
async fn create(
    State(state): State<AppState>,
    AppJson(request): AppJson<CreatePackageRequest>,
) -> Result<
    (
        StatusCode,
        [(HeaderName, HeaderValue); 1],
        Json<CreatePackageResponse>,
    ),
    AppError,
> {
    // NEVER log content; paths/sizes/counts only (the repository logs the
    // accepted package at INFO).
    tracing::debug!(
        name = ?request.name,
        files = request.files.len(),
        composed_deps = request.composed_deps.len(),
        "package create requested"
    );
    let created = state
        .packages
        .create(NewPackage {
            name: request.name,
            files: request.files,
            composed_deps: request.composed_deps,
        })
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
/// ascending). Empty store answers `[]`.
async fn list(State(state): State<AppState>) -> Result<Json<Vec<String>>, AppError> {
    let names = state.packages.list().await?;
    tracing::info!(count = names.len(), "packages listed");
    Ok(Json(names))
}

/// `GET /api/v1/packages/{name}`: fetch one package or `404`.
async fn get_one(
    State(state): State<AppState>,
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
            tracing::info!(name = %name, "package fetched");
            Ok(Json(PackageResponse::try_from(package)?))
        }
        None => Err(AppError::NotFound(format!("package not found: {name}"))),
    }
}

/// Package routes, to be nested under `/api/v1`. The body-limit layer is
/// scoped to these routes only (GETs carry no body; the limit is harmless
/// there and keeps the layer wiring simple).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/packages", get(list).post(create))
        .route("/packages/:name", get(get_one))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
}
