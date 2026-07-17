//! Canvas-specific GitHub calls, added onto [`DashboardGithub`] as a sibling
//! `impl` block: the org-membership role read the overview's owner detection
//! needs. Follows the dashboard client's raw-DTO + paged `get_page` pattern
//! (same transport, same error mapping); lives in its own file so neither
//! module grows past the file-size budget.

use secrecy::SecretString;
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::dashboard::DashboardGithub;

#[derive(Deserialize)]
struct RawOrgLogin {
    login: String,
}

/// One element of the bare-array `GET /user/memberships/orgs` response.
#[derive(Deserialize)]
struct RawMembership {
    /// `admin` (org owner) or `member`.
    #[serde(default)]
    role: String,
    organization: RawOrgLogin,
}

/// The caller's active membership in one organization: the org login plus the
/// caller's role (`admin` = org owner; anything else = plain member).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrgMembership {
    pub org: String,
    pub role: String,
}

impl DashboardGithub {
    /// `GET /user/memberships/orgs?state=active` (user token), paginated — the
    /// caller's ACTIVE org memberships with their role. The overview uses
    /// `role == "admin"` as the org-owner signal; `state=active` excludes
    /// pending invitations (an invitee is not an owner of anything yet).
    pub(crate) async fn user_org_memberships(
        &self,
        user_token: &SecretString,
    ) -> Result<Vec<OrgMembership>, AppError> {
        let mut url = format!("{}/user/memberships/orgs", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> =
            Some(vec![("state", "active"), ("per_page", "100")]);
        let mut out = Vec::new();
        loop {
            let (page, next): (Vec<RawMembership>, _) = self
                .get_page(&url, user_token, query.as_deref(), "user_org_memberships")
                .await?;
            out.extend(page.into_iter().map(|m| OrgMembership {
                org: m.organization.login,
                role: m.role,
            }));
            match next {
                Some(next_url) => {
                    url = next_url;
                    query = None;
                }
                None => return Ok(out),
            }
        }
    }
}

#[cfg(test)]
#[path = "github_tests.rs"]
mod tests;
