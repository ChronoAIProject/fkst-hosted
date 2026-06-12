//! Resource authorization: pure policy (`allows`) + async facade (`Authorizer`).
//!
//! Policy rules (ordered):
//! 1. `fkst:admin` scope -> allow everything
//! 2. owner_user_id == caller -> allow everything
//! 3. org member: Viewer -> Read; Member -> Read+Write; Admin -> all
//! 4. owner_user_id == None (legacy pre-auth doc) -> allow everything
//!
//! Share-aware predicates (`can_read_package`, `can_use_package`) additionally
//! consult the `package_shares` collection after the owner/org/legacy checks
//! fail: a `read`- or `use`-level share grants read; only a `use`-level share
//! grants use (required for session create).
//!
//! The async facade (`Authorizer`) only calls NyxID when the pure checks
//! don't already decide, so owner-path requests stay fast and keep working
//! during a NyxID outage.

use crate::auth::AuthContext;
use crate::error::AppError;
use crate::nyxid::{NyxIdClient, OrgRole};
use crate::packages::ShareRepo;

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

/// Operator escape-hatch scope.
pub const ADMIN_SCOPE: &str = "fkst:admin";

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
    // Rule 1: admin scope bypasses everything.
    if ctx.has_scope(ADMIN_SCOPE) {
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
    shares: Option<ShareRepo>,
}

impl Authorizer {
    /// Build an authorizer with an optional NyxID client and optional shares
    /// repository.
    pub fn new(nyxid: Option<NyxIdClient>) -> Self {
        Self {
            nyxid,
            shares: None,
        }
    }

    /// Build an authorizer with NyxID client and shares repository.
    pub fn with_shares(nyxid: Option<NyxIdClient>, shares: ShareRepo) -> Self {
        Self {
            nyxid,
            shares: Some(shares),
        }
    }

    /// Authorizer without NyxID (owner-only policy, no org features).
    pub fn disabled() -> Self {
        Self {
            nyxid: None,
            shares: None,
        }
    }

    /// Attach the shares repository after construction (for test convenience
    /// and staged wiring).
    pub fn set_shares(&mut self, shares: ShareRepo) {
        self.shares = Some(shares);
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
        if ctx.has_scope(ADMIN_SCOPE) {
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
    pub async fn visible_org_ids(&self, ctx: &AuthContext) -> Result<Vec<String>, AppError> {
        let Some(client) = &self.nyxid else {
            return Ok(Vec::new());
        };
        match client.user_orgs(&ctx.user_id, &ctx.raw_token).await {
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
        // Admin scope bypasses.
        if ctx.has_scope(ADMIN_SCOPE) {
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

    /// Check whether `ctx` can read `package_name` (owner, org visibility, or
    /// share grant at Read or Use level). The `package_owner` and `package_org`
    /// come from the fetched Package document.
    ///
    /// Returns `true` when any access path grants read. Does NOT fail closed
    /// on share-repo errors -- falls back to the owner/org/legacy checks only.
    pub async fn can_read_package(
        &self,
        ctx: &AuthContext,
        package_name: &str,
        package_owner: Option<&str>,
        package_org: Option<&str>,
    ) -> bool {
        // Fast paths: admin, owner, legacy.
        if ctx.has_scope(ADMIN_SCOPE) {
            return true;
        }
        if let Some(owner) = package_owner {
            if owner == ctx.user_id {
                return true;
            }
        } else {
            // Legacy (no owner) -> allow.
            return true;
        }

        // Org visibility check.
        if let Some(org_id) = package_org {
            if let Some(client) = &self.nyxid {
                if let Ok(Some(_role)) = client.org_role(org_id, &ctx.user_id).await {
                    return true;
                }
            }
        }

        // Share grant check.
        if let Some(shares) = &self.shares {
            let org_ids = self.visible_org_ids_raw(ctx).await;
            match shares
                .has_read_share(package_name, &ctx.user_id, &org_ids)
                .await
            {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(
                        package = package_name,
                        error = %error,
                        "share read check failed; falling back to owner/org"
                    );
                }
            }
        }

        false
    }

    /// Check whether `ctx` can *use* `package_name` (required for session
    /// create). Requires owner/org-write access OR a `use`-level share grant.
    /// A `read`-level share does NOT grant use.
    ///
    /// Returns `true` when any access path grants use. Does NOT fail closed
    /// on share-repo errors.
    pub async fn can_use_package(
        &self,
        ctx: &AuthContext,
        package_name: &str,
        package_owner: Option<&str>,
        package_org: Option<&str>,
    ) -> bool {
        // Fast paths: admin, owner, legacy.
        if ctx.has_scope(ADMIN_SCOPE) {
            return true;
        }
        if let Some(owner) = package_owner {
            if owner == ctx.user_id {
                return true;
            }
        } else {
            // Legacy (no owner) -> allow.
            return true;
        }

        // Org write access (member or admin).
        if let Some(org_id) = package_org {
            if let Some(client) = &self.nyxid {
                if let Ok(Some(role)) = client.org_role(org_id, &ctx.user_id).await {
                    match role {
                        OrgRole::Admin | OrgRole::Member => return true,
                        OrgRole::Viewer => {}
                    }
                }
            }
        }

        // Use-level share grant check.
        if let Some(shares) = &self.shares {
            let org_ids = self.visible_org_ids_raw(ctx).await;
            match shares
                .has_use_share(package_name, &ctx.user_id, &org_ids)
                .await
            {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(
                        package = package_name,
                        error = %error,
                        "share use check failed; falling back to owner/org"
                    );
                }
            }
        }

        false
    }

    /// Get visible org ids without error propagation (returns empty on
    /// failure). For use inside the can_read/can_use predicates that should
    /// not fail closed on org lookup errors.
    async fn visible_org_ids_raw(&self, ctx: &AuthContext) -> Vec<String> {
        self.visible_org_ids(ctx).await.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthContext;
    use secrecy::SecretString;

    fn ctx(user: &str, scopes: &[&str]) -> AuthContext {
        AuthContext {
            user_id: user.to_string(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            roles: vec![],
            groups: vec![],
            permissions: vec![],
            session_id: None,
            is_service_account: false,
            delegation: None,
            raw_token: SecretString::new("".into()),
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

    // ---- admin scope bypass ----

    #[test]
    fn admin_scope_bypasses() {
        let ctx = ctx("ops", &["fkst:admin"]);
        let res = own(Some("alice"), Some("org-1"));
        assert!(allows(&ctx, res, None, Action::Read));
        assert!(allows(&ctx, res, None, Action::Write));
        assert!(allows(&ctx, res, None, Action::Manage));
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
