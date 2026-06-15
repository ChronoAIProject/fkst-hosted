//! Vault HTTP API for `/api/v1/vault/*`: manage per-scope env variables and
//! secrets. Secrets are **write-only** over HTTP — a secret value is accepted
//! on `PUT` and never returned by any read.
//!
//! Endpoints:
//! - `GET    /api/v1/vault/entries?scope=global|repo&repo=<owner>/<name>` —
//!   redacted list (`value` only for `kind=variable`).
//! - `PUT    /api/v1/vault/entries` — upsert by `(owner, scope, key)`.
//! - `DELETE /api/v1/vault/entries/{id}` — delete (204).
//!
//! This is purely the web edge: wire DTOs, scope parsing, and status mapping.
//! All authoritative validation (key rule, reserved-key denylist, value/entry
//! caps) and the encryption live in `crate::vault::VaultService`. Authorization
//! is the existing owner-or-org default-deny `Authorizer`.

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::auth::AuthContext;
use crate::authz::{Action, Ownership};
use crate::error::AppError;
use crate::routes::extract::AppJson;
use crate::routes::rfc3339;
use crate::state::AppState;
use crate::vault::{EnvKind, EnvScopeRef, RepoRef, VaultEntry, WriteRequest};

/// Request-body cap for the vault routes. The per-value cap is enforced in the
/// service (default 64 KiB); this is a generous wire-size DoS guard with
/// headroom for the JSON framing and `scope`/`key` fields.
const MAX_REQUEST_BODY_BYTES: usize = 256 * 1024;

// ---- DTOs ---------------------------------------------------------------

/// Query parameters for `GET /api/v1/vault/entries`. `scope` is `"global"`
/// (default) or `"repo"`; when `"repo"`, `repo` is required as `owner/name`.
#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
}

/// Wire shape of a scope in a request/response body.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeBody {
    /// `true` for an owner-wide entry.
    #[serde(default)]
    pub global: bool,
    /// `owner/name` for a repo-scoped entry.
    #[serde(default)]
    pub repo: Option<String>,
}

/// Request body for `PUT /api/v1/vault/entries`. Unknown fields are denied so
/// client typos fail loudly. `Debug` is hand-written below to redact `value`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertRequest {
    pub scope: ScopeBody,
    pub key: String,
    pub kind: EnvKind,
    pub value: String,
    /// Optional org to attach the entry to. When present, the caller must be an
    /// org writer.
    #[serde(default)]
    pub org_id: Option<String>,
}

// The Debug for UpsertRequest is derived; `value` would leak. Hand-write it.
impl std::fmt::Debug for UpsertRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpsertRequest")
            .field("scope", &self.scope)
            .field("key", &self.key)
            .field("kind", &self.kind)
            .field("value", &"<redacted>")
            .field("org_id", &self.org_id)
            .finish()
    }
}

/// Redacted view of a vault entry. A secret's `value` is NEVER populated; a
/// variable's `value` is. `masked_hint` is the display-only `"…last4"` for a
/// secret.
#[derive(Debug, Serialize)]
pub struct EntryView {
    pub id: String,
    pub key: String,
    pub kind: EnvKind,
    pub scope: ScopeBody,
    pub masked_hint: Option<String>,
    /// Present only for `kind=variable`. A secret's value is never serialized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub updated_at: String,
}

impl TryFrom<VaultEntry> for EntryView {
    type Error = AppError;

    fn try_from(entry: VaultEntry) -> Result<Self, Self::Error> {
        let updated_at = rfc3339(entry.updated_at)?;
        let value = match entry.kind {
            EnvKind::Variable => entry.value_plain,
            // Hard guarantee: a secret never carries a value field out.
            EnvKind::Secret => None,
        };
        Ok(EntryView {
            id: entry.id.to_string(),
            key: entry.key,
            kind: entry.kind,
            scope: ScopeBody::from(&entry.scope),
            masked_hint: entry.masked_hint,
            value,
            updated_at,
        })
    }
}

impl From<&EnvScopeRef> for ScopeBody {
    fn from(scope: &EnvScopeRef) -> Self {
        ScopeBody {
            global: scope.global && scope.repo.is_none(),
            repo: scope
                .repo
                .as_ref()
                .map(|r| format!("{}/{}", r.owner, r.name)),
        }
    }
}

// ---- Helpers ------------------------------------------------------------

/// Parse an `owner/name` repo string into a [`RepoRef`]. Rejects anything that
/// is not exactly one `owner` + one `name` separated by a single `/`.
fn parse_repo(repo: &str) -> Result<RepoRef, AppError> {
    match repo.split_once('/') {
        Some((owner, name)) if !owner.is_empty() && !name.is_empty() && !name.contains('/') => {
            Ok(RepoRef {
                owner: owner.to_string(),
                name: name.to_string(),
            })
        }
        _ => Err(AppError::Validation(
            "repo must be in the form owner/name".to_string(),
        )),
    }
}

/// Resolve a [`ScopeBody`] (PUT body) into an [`EnvScopeRef`]. Exactly one of
/// `global` or `repo` must be set.
fn scope_from_body(body: &ScopeBody) -> Result<EnvScopeRef, AppError> {
    match (&body.repo, body.global) {
        (Some(repo), false) => Ok(EnvScopeRef {
            repo: Some(parse_repo(repo)?),
            global: false,
        }),
        (None, true) => Ok(EnvScopeRef::global()),
        _ => Err(AppError::Validation(
            "scope must set exactly one of global=true or repo=owner/name".to_string(),
        )),
    }
}

/// Resolve the `?scope`/`?repo` query into an [`EnvScopeRef`]. Default scope is
/// global; `scope=repo` requires a valid `repo`.
fn scope_from_query(query: &ListQuery) -> Result<EnvScopeRef, AppError> {
    match query.scope.as_deref() {
        None | Some("global") => Ok(EnvScopeRef::global()),
        Some("repo") => {
            let repo = query.repo.as_deref().ok_or_else(|| {
                AppError::Validation("scope=repo requires repo=owner/name".to_string())
            })?;
            Ok(EnvScopeRef {
                repo: Some(parse_repo(repo)?),
                global: false,
            })
        }
        Some(other) => Err(AppError::Validation(format!(
            "unknown scope {other:?}: expected global or repo"
        ))),
    }
}

// ---- Handlers -----------------------------------------------------------

/// `GET /api/v1/vault/entries`: list the caller's entries in a scope, redacted.
/// Secret values are never included; variable values are.
async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<EntryView>>, AppError> {
    let scope = scope_from_query(&query)?;
    // Read of an owner's own entries: the owner-path authz fast-path always
    // allows it, but route through the authorizer for uniform default-deny.
    state
        .authz
        .authorize(
            &ctx,
            Ownership {
                owner_user_id: Some(&ctx.user_id),
                org_id: None,
            },
            Action::Read,
            "vault",
            &scope.scope_key(),
        )
        .await?;

    let entries = state.vault.list_in_scope(&ctx.user_id, &scope).await?;
    let views: Vec<EntryView> = entries
        .into_iter()
        .map(EntryView::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    tracing::info!(
        owner = %ctx.user_id,
        scope_key = %scope.scope_key(),
        count = views.len(),
        "vault entries listed"
    );
    Ok(Json(views))
}

/// `PUT /api/v1/vault/entries`: upsert an entry by `(owner, scope, key)`.
/// Returns the redacted entry.
async fn upsert(
    State(state): State<AppState>,
    ctx: AuthContext,
    AppJson(request): AppJson<UpsertRequest>,
) -> Result<Json<EntryView>, AppError> {
    let scope = scope_from_body(&request.scope)?;

    // If an org is named, require the caller to be an org writer; the entry is
    // still owned by the caller (single-principal write in v1).
    if let Some(ref org_id) = request.org_id {
        state.authz.require_org_writer(&ctx, org_id).await?;
    }

    // Never log the value; key/kind/scope only.
    tracing::debug!(
        owner = %ctx.user_id,
        scope_key = %scope.scope_key(),
        key = ?request.key,
        kind = ?request.kind,
        "vault upsert requested"
    );

    let write = WriteRequest {
        owner_user_id: ctx.user_id.clone(),
        org_id: request.org_id,
        scope,
        key: request.key,
        kind: request.kind,
        // Move the value into a zeroizing buffer so the plaintext is wiped once
        // the service has encrypted/stored it.
        value: Zeroizing::new(request.value),
    };
    let stored = state.vault.upsert(write).await?;
    Ok(Json(EntryView::try_from(stored)?))
}

/// `DELETE /api/v1/vault/entries/{id}`: remove an entry. Owner-or-org authz;
/// `404` when the entry does not exist or the caller cannot see it.
async fn delete_one(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let uuid = bson::Uuid::parse_str(&id)
        .map_err(|_| AppError::Validation("invalid entry id: must be a UUID".to_string()))?;

    // Fetch for the authz check (need the entry's ownership fields). An absent
    // entry is a 404 for everyone (anti-enumeration on Read-tier denial).
    let entry = state
        .vault
        .get(uuid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("vault entry not found: {id}")))?;

    state
        .authz
        .authorize(
            &ctx,
            Ownership {
                owner_user_id: Some(&entry.owner_user_id),
                org_id: entry.org_id.as_deref(),
            },
            Action::Manage,
            "vault",
            &id,
        )
        .await?;

    // Delete scoped to the entry's owner (the authz above already proved the
    // caller may manage it; the owner scoping is belt-and-braces).
    let deleted = state.vault.delete(uuid, &entry.owner_user_id).await?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound(format!("vault entry not found: {id}")))
    }
}

// ---- Router -------------------------------------------------------------

/// Vault routes, to be nested under `/api/v1`. The body-limit layer is scoped
/// to these routes only.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vault/entries", get(list).put(upsert))
        .route("/vault/entries/:id", axum::routing::delete(delete_one))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::EncryptedBlob;

    fn secret_entry() -> VaultEntry {
        VaultEntry {
            id: bson::Uuid::new(),
            owner_user_id: "u1".to_string(),
            org_id: None,
            scope: EnvScopeRef::global(),
            scope_key: "global".to_string(),
            key: "OPENAI_API_KEY".to_string(),
            kind: EnvKind::Secret,
            value_plain: None,
            value_enc: Some(EncryptedBlob {
                ciphertext: vec![1, 2, 3],
                nonce: [0u8; 12],
                wrapped_dek: vec![4, 5, 6],
                key_id: "local-v1".to_string(),
                alg: crate::vault::ENVELOPE_ALG.to_string(),
            }),
            masked_hint: Some("…last".to_string()),
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            updated_at: bson::DateTime::from_millis(1_700_000_000_000),
            created_by: "u1".to_string(),
        }
    }

    fn variable_entry() -> VaultEntry {
        VaultEntry {
            kind: EnvKind::Variable,
            value_plain: Some("plain-config".to_string()),
            value_enc: None,
            masked_hint: None,
            key: "LOG_LEVEL".to_string(),
            ..secret_entry()
        }
    }

    #[test]
    fn secret_view_never_serializes_a_value() {
        let view = EntryView::try_from(secret_entry()).expect("view");
        let body = serde_json::to_value(&view).unwrap();
        assert!(body.get("value").is_none(), "secret leaked a value field");
        assert_eq!(body["masked_hint"], "…last");
        assert_eq!(body["kind"], "secret");
    }

    #[test]
    fn variable_view_includes_its_value() {
        let view = EntryView::try_from(variable_entry()).expect("view");
        let body = serde_json::to_value(&view).unwrap();
        assert_eq!(body["value"], "plain-config");
        assert_eq!(body["kind"], "variable");
    }

    #[test]
    fn upsert_request_debug_redacts_value() {
        let req = UpsertRequest {
            scope: ScopeBody {
                global: true,
                repo: None,
            },
            key: "K".to_string(),
            kind: EnvKind::Secret,
            value: "super-secret".to_string(),
            org_id: None,
        };
        let rendered = format!("{req:?}");
        assert!(
            !rendered.contains("super-secret"),
            "value leaked: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn parse_repo_accepts_owner_name() {
        let r = parse_repo("acme/site").expect("ok");
        assert_eq!(r.owner, "acme");
        assert_eq!(r.name, "site");
    }

    #[test]
    fn parse_repo_rejects_malformed() {
        for bad in ["noslash", "a/b/c", "/name", "owner/", ""] {
            assert!(parse_repo(bad).is_err(), "{bad} should reject");
        }
    }

    #[test]
    fn scope_from_body_requires_exactly_one() {
        assert!(scope_from_body(&ScopeBody {
            global: true,
            repo: None
        })
        .is_ok());
        assert!(scope_from_body(&ScopeBody {
            global: false,
            repo: Some("acme/site".to_string())
        })
        .is_ok());
        // both set / neither set => error
        assert!(scope_from_body(&ScopeBody {
            global: true,
            repo: Some("acme/site".to_string())
        })
        .is_err());
        assert!(scope_from_body(&ScopeBody {
            global: false,
            repo: None
        })
        .is_err());
    }

    #[test]
    fn scope_from_query_defaults_to_global() {
        let scope = scope_from_query(&ListQuery::default()).expect("ok");
        assert_eq!(scope.scope_key(), "global");
    }

    #[test]
    fn scope_from_query_repo_requires_repo_param() {
        let err = scope_from_query(&ListQuery {
            scope: Some("repo".to_string()),
            repo: None,
        })
        .expect_err("must require repo");
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
    }

    #[test]
    fn scope_from_query_rejects_unknown_scope() {
        let err = scope_from_query(&ListQuery {
            scope: Some("planet".to_string()),
            repo: None,
        })
        .expect_err("must reject");
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
    }

    #[test]
    fn scope_body_from_env_scope_round_trips() {
        let repo_scope = EnvScopeRef::repo("acme", "site");
        let body = ScopeBody::from(&repo_scope);
        assert_eq!(body.repo.as_deref(), Some("acme/site"));
        assert!(!body.global);

        let global_scope = EnvScopeRef::global();
        let body = ScopeBody::from(&global_scope);
        assert!(body.global);
        assert!(body.repo.is_none());
    }
}
