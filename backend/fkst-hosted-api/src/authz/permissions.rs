//! `fkst:*` permission vocabulary and the action-layer guard.
//!
//! This is the FIRST of the two authorization layers (see the `authz` module
//! docs for the full model):
//!
//! 1. **Action layer (here):** may the caller perform this *class* of action at
//!    all? Enforced by [`require_permission`] against the `fkst:*` permission
//!    strings NyxID assigned to the caller (carried in the identity token's
//!    `permissions[]`). Exact-string inclusion — no role→permission matrix,
//!    no local role store (mirrors Ornn's `requirePermission`). fkst-hosted owns
//!    the *vocabulary*; NyxID owns the *assignment*.
//! 2. **Object layer (`authz.rs`):** may the caller act on this *specific*
//!    resource (ownership / org role / share)? Checked after the action layer.
//!
//! Both layers must pass. NyxID must assign these `fkst:*` permissions to its
//! roles to preserve current behaviour (org Admin → all; Member → read + write
//! + trigger; Viewer → read).

use crate::auth::AuthContext;
use crate::error::AppError;

// ---- Permission vocabulary ------------------------------------------------
//
// Each constant gates the route(s) noted in its doc comment. The set covers the
// LIVE route inventory only (sessions, goals, github, catalog, vault); the
// package store was removed (#115) so there is no `fkst:package:*`.

/// Platform-admin escape hatch: bypasses both the action and object layers.
/// NyxID assigns it; it replaces the legacy `fkst:admin` *scope*.
pub const ADMIN: &str = "fkst:admin";

/// Read a session — `GET /api/v1/sessions/:id`.
pub const SESSION_READ: &str = "fkst:session:read";
/// Stop a session — `POST /api/v1/sessions/:id/stop`.
pub const SESSION_STOP: &str = "fkst:session:stop";

/// Read goals — `GET /api/v1/goals`, `GET /api/v1/goals/:id`.
pub const GOAL_READ: &str = "fkst:goal:read";
/// Create a goal — `POST /api/v1/goals`.
pub const GOAL_CREATE: &str = "fkst:goal:create";
/// Update a goal — `PATCH /api/v1/goals/:id`.
pub const GOAL_UPDATE: &str = "fkst:goal:update";
/// Delete a goal — `DELETE /api/v1/goals/:id`.
pub const GOAL_DELETE: &str = "fkst:goal:delete";
/// Trigger a goal (creates a session) — `POST /api/v1/goals/:id/trigger`.
pub const GOAL_TRIGGER: &str = "fkst:goal:trigger";

/// Read the GitHub issues hub — `GET /api/v1/github/accounts`, `.../issues`,
/// `GET .../repos/:owner/:repo/issues/:number[/comments]`.
pub const GITHUB_READ: &str = "fkst:github:read";
/// Write through the GitHub issues hub — `POST`/`PATCH` issue + comment routes.
pub const GITHUB_WRITE: &str = "fkst:github:write";

/// Read the skill catalog — all `GET /api/v1/catalog/*` routes (#114).
pub const CATALOG_READ: &str = "fkst:catalog:read";

/// List/read vault entries — `GET /api/v1/vault/entries` (#100).
pub const VAULT_READ: &str = "fkst:vault:read";
/// Upsert a vault entry — `PUT /api/v1/vault/entries`.
pub const VAULT_WRITE: &str = "fkst:vault:write";
/// Delete a vault entry — `DELETE /api/v1/vault/entries/:id`.
pub const VAULT_DELETE: &str = "fkst:vault:delete";

/// Action-layer guard: require that `ctx` carries `perm` (or the admin
/// permission, which bypasses).
///
/// Returns `403 Forbidden` when neither is present. Exact-string inclusion;
/// no role expansion. This is the FIRST gate every protected handler runs,
/// before any object-level (ownership/org/share) check. A caller with no
/// permissions ("headers mode") is therefore denied at every gated route.
pub fn require_permission(ctx: &AuthContext, perm: &str) -> Result<(), AppError> {
    if ctx.has_permission(ADMIN) || ctx.has_permission(perm) {
        return Ok(());
    }
    tracing::debug!(
        user_id = %ctx.user_id,
        required = perm,
        "permission denied (action layer)"
    );
    Err(AppError::Forbidden(format!(
        "missing required permission: {perm}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn ctx_with(perms: &[&str]) -> AuthContext {
        AuthContext {
            user_id: "u".to_string(),
            email: String::new(),
            display_name: "u".to_string(),
            roles: vec![],
            permissions: perms.iter().map(|p| p.to_string()).collect(),
            groups: vec![],
            user_access_token: Some(SecretString::new("t".into())),
        }
    }

    #[test]
    fn present_permission_proceeds() {
        let ctx = ctx_with(&[GOAL_CREATE]);
        assert!(require_permission(&ctx, GOAL_CREATE).is_ok());
    }

    #[test]
    fn missing_permission_is_forbidden() {
        let ctx = ctx_with(&[GOAL_READ]);
        let err = require_permission(&ctx, GOAL_CREATE).expect_err("must deny");
        assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
    }

    #[test]
    fn admin_permission_bypasses_any_required() {
        let ctx = ctx_with(&[ADMIN]);
        assert!(require_permission(&ctx, GOAL_DELETE).is_ok());
        assert!(require_permission(&ctx, GITHUB_WRITE).is_ok());
        assert!(require_permission(&ctx, VAULT_DELETE).is_ok());
    }

    #[test]
    fn empty_permissions_deny_everything() {
        // "Headers mode" yields an empty permission set: every gated action is
        // denied at the action layer.
        let ctx = ctx_with(&[]);
        assert!(require_permission(&ctx, SESSION_READ).is_err());
        assert!(require_permission(&ctx, CATALOG_READ).is_err());
    }

    #[test]
    fn exact_string_match_only() {
        // A near-miss must not be accepted: no prefix/substring matching.
        let ctx = ctx_with(&["fkst:goal"]);
        assert!(require_permission(&ctx, GOAL_READ).is_err());
    }
}
