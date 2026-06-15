//! Catalog HTTP API for `/api/v1/catalog/*`: list the Ornn skills/skillsets a
//! user may attach to a session, and their version lists (issue #114).
//!
//! Endpoints:
//! - `GET /api/v1/catalog/skills?scope=&system=&kind=&tags=&q=&page=`
//! - `GET /api/v1/catalog/skillsets?scope=&kind=&tags=&q=&page=`
//! - `GET /api/v1/catalog/skills/:name/versions`
//! - `GET /api/v1/catalog/skillsets/:name/versions`
//!
//! Pure web edge: it forwards the caller's NyxID token to Ornn's search /
//! versions endpoints (per the requested scope) and re-serializes the result.
//! **fkst-hosted does ZERO permission logic** — Ornn enforces all visibility,
//! and its 4xx/5xx pass through as the authoritative result. When NyxID/Ornn is
//! not wired (auth disabled / no service client) the endpoints answer `503`.

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::AuthContext;
use crate::error::AppError;
use crate::ornn::client::OrnnClient;
use crate::ornn::types::{SearchPage, VersionRow};
use crate::state::AppState;

/// Query parameters shared by the skills/skillsets list endpoints.
///
/// `scope` is the caller-facing scope (`mine` | `shared` | `public`); `system`
/// (`any` | `only` | `exclude`) further filters skills by their system flag
/// (skillsets have no system flag, so `system` is ignored there). `kind`,
/// `tags`, `q`, and `page` are forwarded to Ornn's search verbatim.
#[derive(Debug, Deserialize, Default)]
pub struct CatalogQuery {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub page: Option<String>,
}

/// Aggregated catalog response (per issue #114 §5). One of `skills` /
/// `skillsets` is populated depending on the endpoint; the other is empty.
#[derive(Debug, Serialize, Default)]
pub struct CatalogResponse {
    pub data: CatalogData,
}

/// The `data` envelope of a catalog response.
#[derive(Debug, Serialize, Default)]
pub struct CatalogData {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<crate::ornn::types::SearchRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skillsets: Vec<crate::ornn::types::SearchRow>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

/// Versions response for the lazy picker (`.../:name/versions`).
#[derive(Debug, Serialize)]
pub struct VersionsResponse {
    pub data: VersionsData,
}

#[derive(Debug, Serialize)]
pub struct VersionsData {
    pub name: String,
    pub versions: Vec<VersionRow>,
}

/// Map the caller's `scope`/`system` into the Ornn skill-search query pairs.
///
/// Per the verified Ornn contract: own private → `scope=mine`; shared →
/// `scope=shared-with-me`; public → `scope=public` (+ a `systemFilter`);
/// `system=only` restricts to system skills, `system=exclude` drops them. An
/// unrecognized scope is rejected as a 400 at the edge (Ornn would 4xx anyway,
/// but a clear local message is friendlier).
fn skill_scope_params(query: &CatalogQuery) -> Result<Vec<(String, String)>, AppError> {
    let scope = query.scope.as_deref().unwrap_or("public");
    let ornn_scope = match scope {
        "mine" => "mine",
        "shared" | "shared-with-me" => "shared-with-me",
        "public" => "public",
        other => {
            return Err(AppError::Validation(format!(
                "invalid scope {other:?}: expected mine | shared | public"
            )));
        }
    };
    let mut params = vec![("scope".to_string(), ornn_scope.to_string())];
    // The system filter only applies to skills; default `any`.
    let system = query.system.as_deref().unwrap_or("any");
    let system_filter = match system {
        "any" => "any",
        "only" => "only",
        "exclude" => "exclude",
        other => {
            return Err(AppError::Validation(format!(
                "invalid system filter {other:?}: expected any | only | exclude"
            )));
        }
    };
    params.push(("systemFilter".to_string(), system_filter.to_string()));
    push_common(&mut params, query);
    Ok(params)
}

/// Map the caller's `scope` into the Ornn skillset-search query pairs.
/// Skillsets have no system flag, so `system` is ignored here.
fn skillset_scope_params(query: &CatalogQuery) -> Result<Vec<(String, String)>, AppError> {
    let scope = query.scope.as_deref().unwrap_or("public");
    let ornn_scope = match scope {
        "mine" => "mine",
        "shared" | "shared-with-me" => "shared-with-me",
        "public" => "public",
        other => {
            return Err(AppError::Validation(format!(
                "invalid scope {other:?}: expected mine | shared | public"
            )));
        }
    };
    let mut params = vec![("scope".to_string(), ornn_scope.to_string())];
    push_common(&mut params, query);
    Ok(params)
}

/// Append the search filters common to both endpoints (only when present).
fn push_common(params: &mut Vec<(String, String)>, query: &CatalogQuery) {
    if let Some(kind) = &query.kind {
        params.push(("kind".to_string(), kind.clone()));
    }
    if let Some(tags) = &query.tags {
        params.push(("tags".to_string(), tags.clone()));
    }
    if let Some(q) = &query.q {
        params.push(("q".to_string(), q.clone()));
    }
    if let Some(page) = &query.page {
        params.push(("page".to_string(), page.clone()));
    }
}

/// Borrow the wired Ornn client or answer `503` (catalog needs NyxID/Ornn).
fn require_ornn(state: &AppState) -> Result<&OrnnClient, AppError> {
    state.ornn.as_ref().ok_or_else(|| {
        AppError::Unavailable("skill catalog unavailable: NyxID/Ornn not configured".to_string())
    })
}

/// Convert a `&[(String,String)]` to the `&[(&str,&str)]` the client expects.
fn as_pairs(params: &[(String, String)]) -> Vec<(&str, &str)> {
    params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

/// `GET /api/v1/catalog/skills` — forward to Ornn `skill-search` per scope.
async fn list_skills(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(query): Query<CatalogQuery>,
) -> Result<Json<CatalogResponse>, AppError> {
    let client = require_ornn(&state)?;
    let params = skill_scope_params(&query)?;
    let page = client
        .skill_search(&ctx.raw_token, &as_pairs(&params))
        .await?;
    tracing::debug!(count = page.items.len(), "catalog skills listed");
    Ok(Json(into_skills_response(page)))
}

/// `GET /api/v1/catalog/skillsets` — forward to Ornn `skillset-search` per scope.
async fn list_skillsets(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(query): Query<CatalogQuery>,
) -> Result<Json<CatalogResponse>, AppError> {
    let client = require_ornn(&state)?;
    let params = skillset_scope_params(&query)?;
    let page = client
        .skillset_search(&ctx.raw_token, &as_pairs(&params))
        .await?;
    tracing::debug!(count = page.items.len(), "catalog skillsets listed");
    Ok(Json(into_skillsets_response(page)))
}

/// `GET /api/v1/catalog/skills/:name/versions` — lazy version list.
async fn skill_versions(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(name): Path<String>,
) -> Result<Json<VersionsResponse>, AppError> {
    validate_catalog_name(&name)?;
    let client = require_ornn(&state)?;
    let versions = client.skill_versions(&ctx.raw_token, &name).await?;
    Ok(Json(VersionsResponse {
        data: VersionsData { name, versions },
    }))
}

/// `GET /api/v1/catalog/skillsets/:name/versions` — lazy version list.
async fn skillset_versions(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(name): Path<String>,
) -> Result<Json<VersionsResponse>, AppError> {
    validate_catalog_name(&name)?;
    let client = require_ornn(&state)?;
    let versions = client.skillset_versions(&ctx.raw_token, &name).await?;
    Ok(Json(VersionsResponse {
        data: VersionsData { name, versions },
    }))
}

/// Reject a malformed `:name` path at the edge (the same grammar a pin uses),
/// so a junk path is a clean 400 rather than a forwarded Ornn 4xx.
fn validate_catalog_name(name: &str) -> Result<(), AppError> {
    let pin = crate::ornn::OrnnSkillPin {
        kind: crate::ornn::OrnnPinKind::Skill,
        name: name.to_string(),
        // A throwaway valid version so only the NAME rule is exercised here.
        version: "0.0".to_string(),
    };
    crate::ornn::validate_pin(&pin).map(|_| ())
}

/// Build the skills aggregated response from an Ornn search page.
fn into_skills_response(page: SearchPage) -> CatalogResponse {
    CatalogResponse {
        data: CatalogData {
            skills: page.items,
            skillsets: Vec::new(),
            page: page.page,
            page_size: page.page_size,
            total: page.total,
        },
    }
}

/// Build the skillsets aggregated response from an Ornn search page.
fn into_skillsets_response(page: SearchPage) -> CatalogResponse {
    CatalogResponse {
        data: CatalogData {
            skills: Vec::new(),
            skillsets: page.items,
            page: page.page,
            page_size: page.page_size,
            total: page.total,
        },
    }
}

/// Catalog routes, nested under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/catalog/skills", get(list_skills))
        .route("/catalog/skillsets", get(list_skillsets))
        .route("/catalog/skills/:name/versions", get(skill_versions))
        .route("/catalog/skillsets/:name/versions", get(skillset_versions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(scope: Option<&str>, system: Option<&str>) -> CatalogQuery {
        CatalogQuery {
            scope: scope.map(str::to_string),
            system: system.map(str::to_string),
            ..CatalogQuery::default()
        }
    }

    #[test]
    fn skill_scope_maps_mine_to_ornn_scope() {
        let params = skill_scope_params(&query(Some("mine"), None)).expect("ok");
        assert!(params.contains(&("scope".to_string(), "mine".to_string())));
        assert!(params.contains(&("systemFilter".to_string(), "any".to_string())));
    }

    #[test]
    fn skill_scope_maps_public_system_only() {
        let params = skill_scope_params(&query(Some("public"), Some("only"))).expect("ok");
        assert!(params.contains(&("scope".to_string(), "public".to_string())));
        assert!(params.contains(&("systemFilter".to_string(), "only".to_string())));
    }

    #[test]
    fn skill_scope_maps_shared_alias() {
        let params = skill_scope_params(&query(Some("shared"), None)).expect("ok");
        assert!(params.contains(&("scope".to_string(), "shared-with-me".to_string())));
    }

    #[test]
    fn skill_scope_defaults_to_public_any() {
        let params = skill_scope_params(&query(None, None)).expect("ok");
        assert!(params.contains(&("scope".to_string(), "public".to_string())));
        assert!(params.contains(&("systemFilter".to_string(), "any".to_string())));
    }

    #[test]
    fn skill_scope_rejects_bad_scope_and_system() {
        assert!(skill_scope_params(&query(Some("bogus"), None)).is_err());
        assert!(skill_scope_params(&query(Some("public"), Some("weird"))).is_err());
    }

    #[test]
    fn skillset_scope_ignores_system_filter() {
        let params = skillset_scope_params(&query(Some("mine"), Some("only"))).expect("ok");
        assert!(params.contains(&("scope".to_string(), "mine".to_string())));
        assert!(
            !params.iter().any(|(k, _)| k == "systemFilter"),
            "skillsets have no system flag"
        );
    }

    #[test]
    fn common_filters_are_forwarded_when_present() {
        let q = CatalogQuery {
            scope: Some("public".to_string()),
            kind: Some("tool".to_string()),
            tags: Some("lint,fmt".to_string()),
            q: Some("format".to_string()),
            page: Some("2".to_string()),
            ..CatalogQuery::default()
        };
        let params = skill_scope_params(&q).expect("ok");
        assert!(params.contains(&("kind".to_string(), "tool".to_string())));
        assert!(params.contains(&("tags".to_string(), "lint,fmt".to_string())));
        assert!(params.contains(&("q".to_string(), "format".to_string())));
        assert!(params.contains(&("page".to_string(), "2".to_string())));
    }

    #[test]
    fn into_skills_response_populates_only_skills() {
        let page = SearchPage {
            items: vec![crate::ornn::types::SearchRow {
                name: "fmt".to_string(),
                guid: None,
                description: None,
                tags: vec![],
                is_private: false,
                is_system_skill: true,
                kind: None,
            }],
            page: 1,
            page_size: 20,
            total: 1,
        };
        let resp = into_skills_response(page);
        assert_eq!(resp.data.skills.len(), 1);
        assert!(resp.data.skillsets.is_empty());
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["data"]["total"], 1);
        // Empty skillsets are omitted by skip_serializing_if.
        assert!(json["data"].get("skillsets").is_none());
        assert_eq!(json["data"]["skills"][0]["name"], "fmt");
    }

    #[test]
    fn validate_catalog_name_rejects_junk() {
        assert!(validate_catalog_name("Bad Name").is_err());
        assert!(validate_catalog_name("ok-name").is_ok());
    }
}
