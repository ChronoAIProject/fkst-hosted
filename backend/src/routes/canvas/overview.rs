//! `GET /api/v1/overview` — the whole-account canvas in ONE call: every account
//! (the personal account first, then each org), its repos with App-installation
//! status, and — for installed repos — the live registration-level active
//! session count plus the union of their package references.
//!
//! Computed live on every call (stateless): the USER token drives the
//! account/repo/installation enumeration; per installed repo an APP
//! installation token reads the OPEN trigger issues, each parsed with the SAME
//! reconciler parser the control plane registers sessions with — so "active"
//! here means exactly "would register" (a malformed trigger counts as invalid,
//! not active). A repo whose trigger read fails NEVER fails the whole call: the
//! account is marked `counts_complete: false` and the canvas renders what it has.
//!
//! The per-repo scans are bounded in BOTH dimensions: at most
//! [`REPO_SCAN_CONCURRENCY`] scans run at once (never one sequential GitHub
//! crawl per installed repo), and each scan carries a [`REPO_SCAN_TIMEOUT`]
//! deadline — a hung repo times out and flags its account incomplete instead of
//! stalling the whole canvas.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_app::GithubAppTokens;
use crate::github_identity::GithubUser;
use crate::models::RepoRef;
use crate::reconcile::registry::parse_registration;
use crate::routes::canvas::types::render_package_ref;
use crate::routes::dashboard::{bearer_token, DashboardGithub, UserRepo};
use crate::routes::repos::Viewer;
use crate::state::AppState;

/// The whole-account canvas overview.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OverviewResponse {
    /// The App's slug (install page link base); null when unconfigured.
    pub app_slug: Option<String>,
    /// The signed-in user.
    pub viewer: Viewer,
    /// The personal account first, then every org the user belongs to, sorted.
    pub accounts: Vec<AccountOverview>,
    /// Sums across all accounts.
    pub totals: OverviewTotals,
}

/// One account (the personal account or an org) on the canvas.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountOverview {
    /// The account login.
    pub login: String,
    /// `personal` or `org`.
    pub kind: String,
    /// The caller owns the account: always true for the personal account; for
    /// an org, true when the caller's active membership role is `admin`.
    pub owner: bool,
    /// This App has an installation on the account.
    pub installed: bool,
    /// The installation id; null when not installed.
    pub installation_id: Option<i64>,
    /// `all` or `selected`; null when not installed OR when GitHub omits the
    /// field (never an empty string — the contract union is
    /// `'all' | 'selected' | null`).
    pub repository_selection: Option<String>,
    /// False when any of this account's repo trigger reads failed (the counts
    /// below may undercount; the call itself still succeeds).
    pub counts_complete: bool,
    /// The account's repos the caller can access, sorted by name.
    pub repos: Vec<RepoOverview>,
}

/// One repo on the canvas, with its live session counts.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RepoOverview {
    /// The repo's immutable GitHub id.
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub private: bool,
    /// The caller has admin permission on the repo.
    pub admin: bool,
    /// This App is installed on the repo.
    pub installed: bool,
    /// Open trigger issues whose body parses (registration-level active).
    pub active_sessions: usize,
    /// The union of package references (`owner/repo@ref:path`) across this
    /// repo's active sessions, in first-appearance order.
    pub packages: Vec<String>,
}

/// Whole-canvas sums.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OverviewTotals {
    /// Active sessions across every account.
    pub sessions: usize,
    /// Per-package active-session counts, sorted by count (desc) then name.
    pub packages: Vec<PackageCount>,
}

/// How many active sessions reference one package.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PackageCount {
    /// The package reference (`owner/repo@ref:path`).
    pub package: String,
    /// Active sessions referencing it (once per session, however many times
    /// the session lists it).
    pub count: usize,
}

/// At most this many per-repo trigger scans run concurrently within one call.
const REPO_SCAN_CONCURRENCY: usize = 8;

/// Per-repo scan deadline. Generous enough for a token mint plus a paginated
/// issue listing, small enough that a hung repo degrades (account flagged
/// incomplete) instead of holding the canvas for the transport timeout.
#[cfg(not(test))]
const REPO_SCAN_TIMEOUT: Duration = Duration::from_secs(8);
/// Tests keep the suite fast by scripting the "slow" repo with a delay well
/// beyond this deadline; the timeout mechanism under test is identical. The
/// value is generous enough (1s) that a HEALTHY repo's instant wiremock mocks
/// always complete within it even on a heavily-loaded CI runner — the earlier
/// 300ms was flaky there — while still far below the slow repo's delay.
#[cfg(test)]
const REPO_SCAN_TIMEOUT: Duration = Duration::from_secs(1);

/// One repo's trigger-scan disposition within a single overview call.
enum ScanOutcome {
    /// The repo is not installed — there is nothing to scan.
    NotScanned,
    /// Queued for the bounded concurrent scan; always overwritten by a result
    /// (rendered as incomplete if a bug ever left it in place).
    Pending,
    /// Each parsed-OK open trigger's rendered package refs.
    Sessions(Vec<Vec<String>>),
    /// The scan failed, timed out, or the App was unusable — the owning
    /// account is flagged `counts_complete: false`.
    Incomplete,
}

/// Scan one installed repo's OPEN trigger issues with an App token and return
/// one entry per PARSED-OK trigger: that session's rendered package refs.
/// Malformed triggers are skipped (invalid, not active) — the level-2 sessions
/// endpoint is where their parse error is surfaced.
async fn scan_repo_sessions_packages(
    gh: &DashboardGithub,
    app: &GithubAppTokens,
    installation_id: i64,
    repo: &RepoRef,
    trigger_label: &str,
) -> Result<Vec<Vec<String>>, AppError> {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let inst_token = app.token_for_repo(&owner_repo, None).await?;
    let triggers = gh
        .issues_by_label(&inst_token, &repo.owner, &repo.name, trigger_label, "open")
        .await?;
    let mut sessions = Vec::new();
    for trigger in &triggers {
        if let Ok(reg) = parse_registration(installation_id, repo, &trigger.summary) {
            sessions.push(reg.def.packages.iter().map(render_package_ref).collect());
        }
    }
    Ok(sessions)
}

/// `GET /api/v1/overview` — the whole-account canvas, computed live.
#[utoipa::path(
    get,
    path = "/overview",
    tag = "canvas",
    operation_id = "canvas_overview",
    responses(
        (status = 200, description = "Every account with its repos, installation status, and live session/package counts", body = OverviewResponse),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not allowlisted (FKST_ACCESS_ALLOWED_USERS)", body = ErrorEnvelope),
        (status = 502, description = "GitHub API error", body = ErrorEnvelope),
        (status = 503, description = "GitHub API unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn overview(
    State(state): State<AppState>,
    user: GithubUser,
    headers: HeaderMap,
) -> Result<Json<OverviewResponse>, AppError> {
    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;

    // User-scoped enumeration (these failing fails the call — there is nothing
    // to render without them).
    let all_repos = gh.user_all_repos(&token).await?;
    let mut orgs = gh.user_orgs(&token).await?;
    orgs.sort();
    let admin_orgs: HashSet<String> = gh
        .user_org_memberships(&token)
        .await?
        .into_iter()
        .filter(|m| m.role == "admin")
        .map(|m| m.org.to_ascii_lowercase())
        .collect();
    let installations = gh.user_installations(&token).await?;
    let mut installed_repos: HashSet<String> = HashSet::new();
    for inst in &installations {
        for repo in gh.user_installation_repos(&token, inst.id).await? {
            installed_repos.insert(format!("{}/{}", repo.owner, repo.name).to_ascii_lowercase());
        }
    }

    // Group the accessible repos by owner login (case-insensitive). Repos whose
    // owner is neither the viewer nor one of the viewer's orgs (e.g. a
    // collaborator repo on someone else's personal account) have no account
    // card on the canvas and are deliberately not listed.
    let mut by_owner: HashMap<String, Vec<UserRepo>> = HashMap::new();
    for repo in all_repos {
        by_owner
            .entry(repo.owner.to_ascii_lowercase())
            .or_default()
            .push(repo);
    }

    let app = state.github_app.as_ref();
    let trigger_label = &state.config.reconcile.substrate_trigger_label;

    let mut accounts = Vec::with_capacity(1 + orgs.len());
    let mut total_sessions = 0usize;
    let mut package_counts: BTreeMap<String, usize> = BTreeMap::new();

    // The personal account first, then every org sorted.
    let account_list: Vec<(String, &'static str, bool)> =
        std::iter::once((user.login.clone(), "personal", true))
            .chain(orgs.into_iter().map(|org| {
                let owner = admin_orgs.contains(&org.to_ascii_lowercase());
                (org, "org", owner)
            }))
            .collect();

    for (login, kind, owner) in account_list {
        let installation = installations
            .iter()
            .find(|i| i.account.eq_ignore_ascii_case(&login));
        let mut repos = by_owner
            .remove(&login.to_ascii_lowercase())
            .unwrap_or_default();
        repos.sort_by(|a, b| a.name.cmp(&b.name));

        // Disposition first, reads second: decide synchronously what each repo
        // needs, then run the actual GitHub scans through a bounded stream so
        // one call never becomes an unbounded sequential crawl, and one hung
        // repo only times itself out.
        let mut outcomes: Vec<ScanOutcome> = repos
            .iter()
            .map(|repo| {
                let installed = installed_repos
                    .contains(&format!("{}/{}", repo.owner, repo.name).to_ascii_lowercase());
                if !installed {
                    ScanOutcome::NotScanned
                } else if app.is_none() || installation.is_none() {
                    // Installed repo but no App creds (or no visible
                    // installation) to read triggers with — the count is
                    // unknowable, not zero.
                    tracing::debug!(
                        owner = %repo.owner,
                        name = %repo.name,
                        "canvas overview: repo installed but the GitHub App is not usable; \
                         marking counts incomplete"
                    );
                    ScanOutcome::Incomplete
                } else {
                    ScanOutcome::Pending
                }
            })
            .collect();

        if let (Some(app), Some(inst)) = (app, installation) {
            let gh = &gh;
            let mut scans = Vec::new();
            for (idx, repo) in repos.iter().enumerate() {
                if !matches!(outcomes[idx], ScanOutcome::Pending) {
                    continue;
                }
                let repo_ref = RepoRef {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                };
                scans.push(async move {
                    let scanned = tokio::time::timeout(
                        REPO_SCAN_TIMEOUT,
                        scan_repo_sessions_packages(gh, app, inst.id, &repo_ref, trigger_label),
                    )
                    .await;
                    let outcome = match scanned {
                        Ok(Ok(sessions)) => ScanOutcome::Sessions(sessions),
                        Ok(Err(error)) => {
                            // Degrade, never fail the whole canvas: the repo
                            // renders with zero counts and the account is
                            // flagged incomplete.
                            tracing::warn!(
                                owner = %repo_ref.owner,
                                name = %repo_ref.name,
                                error = %error,
                                "canvas overview: trigger scan failed; marking counts incomplete"
                            );
                            ScanOutcome::Incomplete
                        }
                        Err(_elapsed) => {
                            tracing::warn!(
                                owner = %repo_ref.owner,
                                name = %repo_ref.name,
                                timeout_ms = REPO_SCAN_TIMEOUT.as_millis() as u64,
                                "canvas overview: trigger scan timed out; marking counts incomplete"
                            );
                            ScanOutcome::Incomplete
                        }
                    };
                    (idx, outcome)
                });
            }
            let results: Vec<(usize, ScanOutcome)> = stream::iter(scans)
                .buffer_unordered(REPO_SCAN_CONCURRENCY)
                .collect()
                .await;
            for (idx, outcome) in results {
                outcomes[idx] = outcome;
            }
        }

        let mut counts_complete = true;
        let mut repo_views = Vec::with_capacity(repos.len());
        for (repo, outcome) in repos.into_iter().zip(outcomes) {
            let installed = !matches!(outcome, ScanOutcome::NotScanned);
            let mut active_sessions = 0usize;
            let mut packages: Vec<String> = Vec::new();
            match outcome {
                ScanOutcome::NotScanned => {}
                ScanOutcome::Sessions(sessions) => {
                    active_sessions = sessions.len();
                    total_sessions += sessions.len();
                    for session_packages in sessions {
                        // Count each package ONCE per session, and union into
                        // the repo's list in first-appearance order.
                        let mut seen_in_session: HashSet<&str> = HashSet::new();
                        for package in &session_packages {
                            if seen_in_session.insert(package) {
                                *package_counts.entry(package.clone()).or_default() += 1;
                            }
                            if !packages.contains(package) {
                                packages.push(package.clone());
                            }
                        }
                    }
                }
                ScanOutcome::Incomplete | ScanOutcome::Pending => {
                    counts_complete = false;
                }
            }
            repo_views.push(RepoOverview {
                id: repo.id,
                owner: repo.owner,
                name: repo.name,
                private: repo.private,
                admin: repo.admin,
                installed,
                active_sessions,
                packages,
            });
        }

        accounts.push(AccountOverview {
            login,
            kind: kind.to_string(),
            owner,
            installed: installation.is_some(),
            installation_id: installation.map(|i| i.id),
            // The raw DTO defaults an omitted field to "" — collapse that to
            // null so the wire value stays within the contract's union.
            repository_selection: installation.and_then(|i| {
                let value = i.repository_selection.clone();
                (!value.is_empty()).then_some(value)
            }),
            counts_complete,
            repos: repo_views,
        });
    }

    // Per-package totals sorted by count (desc) then package name (asc) so the
    // canvas chart renders deterministically.
    let mut packages: Vec<PackageCount> = package_counts
        .into_iter()
        .map(|(package, count)| PackageCount { package, count })
        .collect();
    packages.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.package.cmp(&b.package))
    });

    tracing::debug!(
        user_id = user.id,
        accounts = accounts.len(),
        sessions = total_sessions,
        "canvas overview assembled"
    );
    Ok(Json(OverviewResponse {
        app_slug: state
            .github_app
            .as_ref()
            .and_then(|g| g.app_slug().map(str::to_string)),
        viewer: Viewer { login: user.login },
        accounts,
        totals: OverviewTotals {
            sessions: total_sessions,
            packages,
        },
    }))
}

#[cfg(test)]
#[path = "overview_tests.rs"]
mod tests;
