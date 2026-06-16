//! Resource authorization — the OBJECT layer of the two-layer model.
//!
//! fkst-hosted authorizes in two ordered layers (issue #113):
//!
//! 1. **Action layer** ([`permissions::require_permission`]): may the caller
//!    perform this *class* of action at all? Gated by the `fkst:*` permission
//!    strings NyxID assigned (exact-string inclusion; no local role→permission
//!    matrix). Handlers run this FIRST, at entry.
//! 2. **Object layer (this module):** may the caller act on this *specific*
//!    resource? Ownership / org-role visibility, checked AFTER the action layer.
//!
//! Both must pass. This module is unchanged in spirit from before #113 — it
//! still enforces ownership and org-role visibility — but the admin bypass now
//! reads the `fkst:admin` *permission* (NyxID-assigned), not a token *scope*.
//!
//! Object-layer policy rules (ordered):
//! 1. `fkst:admin` permission -> allow everything
//! 2. owner_user_id == caller -> allow everything
//! 3. org member: Viewer -> Read; Member -> Read+Write; Admin -> all
//! 4. owner_user_id == None (legacy pre-auth doc) -> allow everything
//!
//! The async facade (`Authorizer`) only calls NyxID when the pure checks
//! don't already decide, so owner-path requests stay fast and keep working
//! during a NyxID outage. Org-role lookups use the caller's FORWARDED user
//! access token and FAIL SOFT when it is absent (mirrors Ornn): reads degrade
//! to non-member, and writes that need a role cannot be authorized.
//!
//! Package-share predicates were removed with the package store (#115): packages
//! are now repo-scoped, so package access is governed by the goal repo's GitHub
//! permissions, not a `package_shares` collection.
//!
//! Behavioural note for operators: org Members were previously granted Write
//! directly by rule 3. Under #113 the *action* (e.g. `fkst:goal:trigger`) comes
//! from NyxID's `permissions[]`; the org-membership *object* check below stays.
//! NyxID must assign org Admin → all `fkst:*`, Member → read + write + trigger,
//! Viewer → read to preserve current behaviour.

pub mod permissions;

use crate::auth::AuthContext;
use crate::error::AppError;
use crate::nyxid::{NyxIdClient, OrgRole};

/// Action the caller is attempting on a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Read,
    Write,
    Manage,
}

/// Ownership metadata for a resource (package or session).
#[derive(Debug, Clone, Copy)]
pub struct Ownership<'a> {
    pub owner_user_id: Option<&'a str>,
    pub org_id: Option<&'a str>,
}

/// Pure authorization check: no IO, no side effects.
///
/// Returns `true` when `ctx` is allowed to perform `action` on `res`
/// given the caller's `org_role` (if any).
pub fn allows(
    ctx: &AuthContext,
    res: Ownership<'_>,
    org_role: Option<OrgRole>,
    action: Action,
) -> bool {
    // Rule 1: admin permission bypasses everything.
    if ctx.has_permission(permissions::ADMIN) {
        return true;
    }
    // Rule 2: owner has full access.
    if let Some(owner) = res.owner_user_id {
        if owner == ctx.user_id {
            return true;
        }
    }
    // Rule 3: org-based role access.
    if let Some(role) = org_role {
        return match role {
            OrgRole::Admin => true,
            OrgRole::Member => matches!(action, Action::Read | Action::Write),
            OrgRole::Viewer => matches!(action, Action::Read),
        };
    }
    // Rule 4: legacy (no owner) -> allow everything.
    if res.owner_user_id.is_none() {
        return true;
    }
    false
}

/// Async authorization facade that wraps `allows` with NyxID lookups.
#[derive(Clone)]
pub struct Authorizer {
    nyxid: Option<NyxIdClient>,
}

impl Authorizer {
    /// Build an authorizer with an optional NyxID client.
    pub fn new(nyxid: Option<NyxIdClient>) -> Self {
        Self { nyxid }
    }

    /// Authorizer without NyxID (owner-only policy, no org features).
    pub fn disabled() -> Self {
        Self { nyxid: None }
    }

    /// Access the underlying NyxID client (later consumers: exchange,
    /// proxy, etc.).
    pub fn nyxid(&self) -> Option<&NyxIdClient> {
        self.nyxid.as_ref()
    }

    /// Authorize `action` on resource described by `(label, id)`.
    ///
    /// - Read denial -> `AppError::NotFound` (anti-enumeration: same body
    ///   as an absent resource).
    /// - Write/Manage denial -> `AppError::Forbidden`.
    /// - NyxID failure while an org role is needed -> `AppError::Unavailable`
    ///   (fail closed: never silently allow).
    ///
    /// Only calls NyxID when the owner/legacy/admin-scope checks don't
    /// already decide.
    pub async fn authorize(
        &self,
        ctx: &AuthContext,
        res: Ownership<'_>,
        action: Action,
        label: &str,
        id: &str,
    ) -> Result<(), AppError> {
        // Fast-path pure checks that don't need NyxID.
        if ctx.has_permission(permissions::ADMIN) {
            return Ok(());
        }
        if let Some(owner) = res.owner_user_id {
            if owner == ctx.user_id {
                return Ok(());
            }
        }
        if res.owner_user_id.is_none() {
            // Legacy: allow.
            return Ok(());
        }

        // Need to check org role.
        let org_role = match (res.org_id, &self.nyxid) {
            (Some(org_id), Some(client)) => match client.org_role(org_id, &ctx.user_id).await {
                Ok(role) => role,
                Err(error) => {
                    tracing::error!(
                        org_id,
                        user_id = %ctx.user_id,
                        error = %error,
                        "nyxid org-role lookup failed; failing closed"
                    );
                    return Err(AppError::Unavailable(
                        "authorization service unavailable".to_string(),
                    ));
                }
            },
            _ => None,
        };

        if allows(ctx, res, org_role, action) {
            Ok(())
        } else {
            match action {
                Action::Read => {
                    // Anti-enumeration: same response as "not found".
                    Err(AppError::NotFound(format!("{label} not found: {id}")))
                }
                Action::Write | Action::Manage => {
                    Err(AppError::Forbidden("insufficient permissions".to_string()))
                }
            }
        }
    }

    /// Return the org ids visible to `ctx`. Empty when NyxID is not
    /// configured (owner-only mode).
    ///
    /// The org listing is authenticated with the caller's FORWARDED user access
    /// token. When that token is absent ("headers mode" / no bearer forwarded),
    /// the lookup cannot run, so it FAILS SOFT to an empty list (mirrors Ornn's
    /// `nyxidOrgLookupMiddleware`): the caller is treated as a member of no org,
    /// so read visibility degrades to their own resources only — never a hard
    /// error on a read path.
    pub async fn visible_org_ids(&self, ctx: &AuthContext) -> Result<Vec<String>, AppError> {
        let Some(client) = &self.nyxid else {
            return Ok(Vec::new());
        };
        let Some(user_token) = ctx.user_access_token.as_ref() else {
            tracing::debug!(
                user_id = %ctx.user_id,
                "no forwarded user token; org visibility degrades to non-member"
            );
            return Ok(Vec::new());
        };
        match client.user_orgs(&ctx.user_id, user_token).await {
            Ok(orgs) => Ok(orgs.into_iter().map(|o| o.id).collect()),
            Err(error) => {
                tracing::error!(
                    user_id = %ctx.user_id,
                    error = %error,
                    "nyxid user-orgs lookup failed"
                );
                Err(AppError::Unavailable(
                    "authorization service unavailable".to_string(),
                ))
            }
        }
    }

    /// Require that `ctx` has at least Member role in `org_id`.
    /// Returns `Ok(())` when admin or member; `Forbidden` otherwise.
    pub async fn require_org_writer(
        &self,
        ctx: &AuthContext,
        org_id: &str,
    ) -> Result<(), AppError> {
        // Admin permission bypasses.
        if ctx.has_permission(permissions::ADMIN) {
            return Ok(());
        }
        let role = match &self.nyxid {
            Some(client) => match client.org_role(org_id, &ctx.user_id).await {
                Ok(role) => role,
                Err(error) => {
                    tracing::error!(
                        org_id,
                        user_id = %ctx.user_id,
                        error = %error,
                        "nyxid org-role lookup failed during require_org_writer"
                    );
                    return Err(AppError::Unavailable(
                        "authorization service unavailable".to_string(),
                    ));
                }
            },
            None => None,
        };
        match role {
            Some(OrgRole::Admin | OrgRole::Member) => Ok(()),
            _ => Err(AppError::Forbidden(
                "insufficient permissions: org admin or member required".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthContext;
    use secrecy::SecretString;

    fn ctx(user: &str, permissions: &[&str]) -> AuthContext {
        AuthContext {
            user_id: user.to_string(),
            email: String::new(),
            display_name: user.to_string(),
            roles: vec![],
            permissions: permissions.iter().map(|s| s.to_string()).collect(),
            groups: vec![],
            user_access_token: Some(SecretString::new("".into())),
        }
    }

    fn own<'a>(owner: Option<&'a str>, org: Option<&'a str>) -> Ownership<'a> {
        Ownership {
            owner_user_id: owner,
            org_id: org,
        }
    }

    // ---- owner full access ----

    #[test]
    fn owner_has_full_access() {
        let ctx = ctx("alice", &[]);
        let res = own(Some("alice"), None);
        assert!(allows(&ctx, res, None, Action::Read));
        assert!(allows(&ctx, res, None, Action::Write));
        assert!(allows(&ctx, res, None, Action::Manage));
    }

    // ---- stranger denied everything ----

    #[test]
    fn stranger_is_denied_everything() {
        let ctx = ctx("eve", &[]);
        let res = own(Some("alice"), None);
        assert!(!allows(&ctx, res, None, Action::Read));
        assert!(!allows(&ctx, res, None, Action::Write));
        assert!(!allows(&ctx, res, None, Action::Manage));
    }

    // ---- viewer reads only ----

    #[test]
    fn viewer_reads_only() {
        let ctx = ctx("bob", &[]);
        let res = own(Some("alice"), Some("org-1"));
        assert!(allows(&ctx, res, Some(OrgRole::Viewer), Action::Read));
        assert!(!allows(&ctx, res, Some(OrgRole::Viewer), Action::Write));
        assert!(!allows(&ctx, res, Some(OrgRole::Viewer), Action::Manage));
    }

    // ---- member writes, not manages ----

    #[test]
    fn member_writes_not_manages() {
        let ctx = ctx("bob", &[]);
        let res = own(Some("alice"), Some("org-1"));
        assert!(allows(&ctx, res, Some(OrgRole::Member), Action::Read));
        assert!(allows(&ctx, res, Some(OrgRole::Member), Action::Write));
        assert!(!allows(&ctx, res, Some(OrgRole::Member), Action::Manage));
    }

    // ---- admin manages ----

    #[test]
    fn admin_manages() {
        let ctx = ctx("bob", &[]);
        let res = own(Some("alice"), Some("org-1"));
        assert!(allows(&ctx, res, Some(OrgRole::Admin), Action::Read));
        assert!(allows(&ctx, res, Some(OrgRole::Admin), Action::Write));
        assert!(allows(&ctx, res, Some(OrgRole::Admin), Action::Manage));
    }

    // ---- legacy doc open to authenticated ----

    #[test]
    fn legacy_doc_open_to_authenticated() {
        let ctx = ctx("anyone", &[]);
        let res = own(None, None);
        assert!(allows(&ctx, res, None, Action::Read));
        assert!(allows(&ctx, res, None, Action::Write));
        assert!(allows(&ctx, res, None, Action::Manage));
    }

    // ---- admin permission bypass ----

    #[test]
    fn admin_permission_bypasses() {
        // The admin escape hatch is now a NyxID-assigned PERMISSION.
        let ctx = ctx("ops", &[permissions::ADMIN]);
        let res = own(Some("alice"), Some("org-1"));
        assert!(allows(&ctx, res, None, Action::Read));
        assert!(allows(&ctx, res, None, Action::Write));
        assert!(allows(&ctx, res, None, Action::Manage));
    }

    #[test]
    fn non_admin_permission_does_not_bypass() {
        // A non-admin caller with no ownership/org role is denied a stranger's
        // resource regardless of which other permissions they hold: the object
        // layer is independent of the action-layer permission set.
        let ctx = ctx("ops", &["fkst:goal:read", "fkst:goal:create"]);
        let res = own(Some("alice"), Some("org-1"));
        assert!(!allows(&ctx, res, None, Action::Read));
        assert!(!allows(&ctx, res, None, Action::Write));
    }

    // ---- error mapping tests via Authorizer facade ----

    #[tokio::test]
    async fn read_denial_maps_to_not_found() {
        let authz = Authorizer::disabled();
        let ctx = ctx("eve", &[]);
        let res = own(Some("alice"), None);
        let err = authz
            .authorize(&ctx, res, Action::Read, "package", "pkg-1")
            .await
            .expect_err("must deny");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn write_denial_maps_to_forbidden() {
        let authz = Authorizer::disabled();
        let ctx = ctx("eve", &[]);
        let res = own(Some("alice"), None);
        let err = authz
            .authorize(&ctx, res, Action::Write, "package", "pkg-1")
            .await
            .expect_err("must deny");
        assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn nyxid_failure_maps_to_unavailable() {
        // Use a NyxID client pointed at nothing to simulate failure.
        let client = NyxIdClient::new(
            "http://127.0.0.1:1",
            "api-github",
            "sa_test".to_string(),
            SecretString::from("sas_test".to_string()),
            std::time::Duration::from_secs(30),
        )
        .expect("client");
        let authz = Authorizer::new(Some(client));
        let ctx = ctx("bob", &[]);
        let res = own(Some("alice"), Some("org-1"));
        let err = authz
            .authorize(&ctx, res, Action::Read, "package", "pkg-1")
            .await
            .expect_err("must fail");
        assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
    }
}
