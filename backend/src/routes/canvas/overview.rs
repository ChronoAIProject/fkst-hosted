//! `GET /api/v1/overview` — the whole-account canvas in ONE call: every account
//! (the personal account first, then each org), its repos with App-installation
//! status, and — for installed repos — the live registration-level active
//! session count plus the union of their package references.
//!
//! Computed live on every call (stateless). For an ordinary caller the USER token
//! drives account/repo/installation enumeration. For a verified
//! `FKST_GLOBAL_ADMINS` caller, the APP JWT enumerates every installation and an
//! installation-wide token enumerates every covered repo, including private repos
//! outside the caller's personal visibility. Per installed repo an APP installation
//! token reads the OPEN trigger issues, each parsed with the SAME reconciler parser
//! the control plane registers sessions with — so "active" here means exactly
//! "would register" (a malformed trigger counts as invalid, not active). A repo
//! whose trigger read fails NEVER fails the whole call: the account is marked
//! `counts_complete: false` and the canvas renders what it has.
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
use crate::routes::canvas::overview_broader::resolve_enumeration_token;
use crate::routes::canvas::types::render_package_ref;
use crate::routes::dashboard::{bearer_token, DashboardGithub, InstallationRef, UserRepo};
use crate::routes::repos::Viewer;
use crate::state::AppState;

/// The whole-account canvas overview.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OverviewResponse {
    /// The App's slug (install page link base); null when unconfigured.
    pub app_slug: Option<String>,
    /// The signed-in user.
    pub viewer: Viewer,
    /// True when the verified viewer matched `FKST_GLOBAL_ADMINS` and this
    /// response therefore spans every installation of the configured GitHub App.
    pub global_admin: bool,
    /// The personal account first, then every org the user belongs to, sorted.
    pub accounts: Vec<AccountOverview>,
    /// Sums across all accounts.
    pub totals: OverviewTotals,
    /// Whether this deployment offers the broader-visibility OAuth connect flow
    /// (`GET /api/v1/auth/github/broader`) — true iff the broader OAuth pair is
    /// configured (issue #572). The SPA surfaces the "connect for full repo/org
    /// visibility" action only when this is true.
    pub broader_oauth_available: bool,
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

/// At most this many installation-wide repository enumerations run concurrently
/// for a global-admin overview.
const INSTALLATION_SCAN_CONCURRENCY: usize = 4;

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

/// Internal enumeration result for one repository. `installed` stays explicit
/// because an ordinary caller's overview also includes repos where the App is not
/// installed, while a global-admin overview contains installed repos only.
struct RepoInput {
    repo: UserRepo,
    installed: bool,
}

/// Internal enumeration result for one account, independent of whether it came
/// from the caller's user token or the App-wide global-admin path.
struct AccountInput {
    login: String,
    kind: String,
    owner: bool,
    installation: Option<InstallationRef>,
    repos: Vec<RepoInput>,
    counts_complete: bool,
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

/// Enumerate the accounts/repositories visible to an ordinary caller. This is
/// the pre-global-admin behavior: the optional broader OAuth token widens only
/// the caller's own GitHub visibility, and the App user token determines which
/// of those repositories are installed.
async fn user_account_inputs(
    state: &AppState,
    gh: &DashboardGithub,
    user: &GithubUser,
    user_token: &secrecy::SecretString,
    headers: &HeaderMap,
) -> Result<Vec<AccountInput>, AppError> {
    let enum_token = resolve_enumeration_token(state, user, user_token, headers).await;
    let all_repos = gh.user_all_repos(&enum_token).await?;

    // GitHub App user-to-server tokens leave `/user/orgs` empty, while active
    // membership reads remain populated. Union both so classic broader tokens
    // retain their full organization visibility too.
    let memberships = gh.user_org_memberships(&enum_token).await?;
    let admin_orgs: HashSet<String> = memberships
        .iter()
        .filter(|membership| membership.role == "admin")
        .map(|membership| membership.org.to_ascii_lowercase())
        .collect();
    let mut seen_orgs = HashSet::new();
    let mut orgs = Vec::new();
    for org in memberships
        .into_iter()
        .map(|membership| membership.org)
        .chain(gh.user_orgs(&enum_token).await?)
    {
        if seen_orgs.insert(org.to_ascii_lowercase()) {
            orgs.push(org);
        }
    }
    orgs.sort();

    let installations = gh.user_installations(user_token).await?;
    let mut installed_repos = HashSet::new();
    for installation in &installations {
        for repo in gh
            .user_installation_repos(user_token, installation.id)
            .await?
        {
            installed_repos.insert(format!("{}/{}", repo.owner, repo.name).to_ascii_lowercase());
        }
    }

    let mut by_owner: HashMap<String, Vec<UserRepo>> = HashMap::new();
    for repo in all_repos {
        by_owner
            .entry(repo.owner.to_ascii_lowercase())
            .or_default()
            .push(repo);
    }

    let account_list = std::iter::once((user.login.clone(), "personal".to_string(), true)).chain(
        orgs.into_iter().map(|org| {
            let owner = admin_orgs.contains(&org.to_ascii_lowercase());
            (org, "org".to_string(), owner)
        }),
    );

    Ok(account_list
        .map(|(login, kind, owner)| {
            let installation = installations
                .iter()
                .find(|candidate| candidate.account.eq_ignore_ascii_case(&login))
                .cloned();
            let mut repos = by_owner
                .remove(&login.to_ascii_lowercase())
                .unwrap_or_default();
            repos.sort_by(|a, b| a.name.cmp(&b.name));
            let repos = repos
                .into_iter()
                .map(|repo| {
                    let installed = installed_repos
                        .contains(&format!("{}/{}", repo.owner, repo.name).to_ascii_lowercase());
                    RepoInput { repo, installed }
                })
                .collect();
            AccountInput {
                login,
                kind,
                owner,
                installation,
                repos,
                counts_complete: true,
            }
        })
        .collect())
}

/// Enumerate every installation and covered repository of the configured App.
/// The top-level installation list is authoritative; a failure within one
/// installation degrades that account instead of hiding every other account.
async fn global_admin_account_inputs(
    state: &AppState,
    gh: &DashboardGithub,
    user: &GithubUser,
) -> Result<Vec<AccountInput>, AppError> {
    let app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unavailable("the github app is not configured on this deployment".to_string())
    })?;
    let app_jwt = app.app_jwt()?;
    let installations = gh.app_installations(&app_jwt).await?;

    let mut accounts: Vec<AccountInput> =
        stream::iter(installations.into_iter().map(|installation| async move {
            let login = installation.account.clone();
            let kind = installation.account_kind.clone();
            let owner = kind == "personal" && login.eq_ignore_ascii_case(&user.login);
            let repos_result = match app.installation_wide_token(installation.id).await {
                Ok(token) => gh.installation_repos(&token).await,
                Err(error) => Err(AppError::from(error)),
            };
            let (mut repos, counts_complete) = match repos_result {
                Ok(repos) => (repos, true),
                Err(error) => {
                    tracing::warn!(
                        installation_id = installation.id,
                        account = %login,
                        error = %error,
                        "global-admin overview: installation repo enumeration failed"
                    );
                    (Vec::new(), false)
                }
            };
            repos.sort_by(|a, b| a.name.cmp(&b.name));
            AccountInput {
                login,
                kind,
                owner,
                installation: Some(installation),
                repos: repos
                    .into_iter()
                    .map(|repo| RepoInput {
                        repo,
                        installed: true,
                    })
                    .collect(),
                counts_complete,
            }
        }))
        .buffer_unordered(INSTALLATION_SCAN_CONCURRENCY)
        .collect()
        .await;

    // Stable presentation independent of request completion order: personal
    // installations first, then organization installations, each by login.
    accounts.sort_by(|a, b| {
        let a_org = a.kind == "org";
        let b_org = b.kind == "org";
        a_org.cmp(&b_org).then_with(|| {
            a.login
                .to_ascii_lowercase()
                .cmp(&b.login.to_ascii_lowercase())
        })
    });
    Ok(accounts)
}

/// Resolve registration/package counts for already-enumerated accounts and
/// render the public response. Both user-scoped and App-wide paths share this
/// function, so session parsing, timeouts, totals, and partial-failure behavior
/// cannot drift between roles.
async fn assemble_overview(
    state: &AppState,
    gh: &DashboardGithub,
    user: GithubUser,
    global_admin: bool,
    account_inputs: Vec<AccountInput>,
) -> OverviewResponse {
    let app = state.github_app.as_ref();
    let trigger_label = &state.config.reconcile.substrate_trigger_label;
    let mut accounts = Vec::with_capacity(account_inputs.len());
    let mut total_sessions = 0usize;
    let mut package_counts: BTreeMap<String, usize> = BTreeMap::new();

    for input in account_inputs {
        let installation = input.installation.as_ref();
        let mut outcomes: Vec<ScanOutcome> = input
            .repos
            .iter()
            .map(|repo| {
                if !repo.installed {
                    ScanOutcome::NotScanned
                } else if app.is_none() || installation.is_none() {
                    tracing::debug!(
                        owner = %repo.repo.owner,
                        name = %repo.repo.name,
                        "canvas overview: installed repo cannot be scanned; marking counts incomplete"
                    );
                    ScanOutcome::Incomplete
                } else {
                    ScanOutcome::Pending
                }
            })
            .collect();

        if let (Some(app), Some(installation)) = (app, installation) {
            let mut scans = Vec::new();
            for (idx, repo) in input.repos.iter().enumerate() {
                if !matches!(outcomes[idx], ScanOutcome::Pending) {
                    continue;
                }
                let repo_ref = RepoRef {
                    owner: repo.repo.owner.clone(),
                    name: repo.repo.name.clone(),
                };
                scans.push(async move {
                    let scanned = tokio::time::timeout(
                        REPO_SCAN_TIMEOUT,
                        scan_repo_sessions_packages(
                            gh,
                            app,
                            installation.id,
                            &repo_ref,
                            trigger_label,
                        ),
                    )
                    .await;
                    let outcome = match scanned {
                        Ok(Ok(sessions)) => ScanOutcome::Sessions(sessions),
                        Ok(Err(error)) => {
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

        let mut counts_complete = input.counts_complete;
        let mut repo_views = Vec::with_capacity(input.repos.len());
        for (repo_input, outcome) in input.repos.into_iter().zip(outcomes) {
            let installed = repo_input.installed;
            let repo = repo_input.repo;
            let mut active_sessions = 0usize;
            let mut packages = Vec::new();
            match outcome {
                ScanOutcome::NotScanned => {}
                ScanOutcome::Sessions(sessions) => {
                    active_sessions = sessions.len();
                    total_sessions += sessions.len();
                    for session_packages in sessions {
                        let mut seen_in_session = HashSet::new();
                        for package in &session_packages {
                            if seen_in_session.insert(package.as_str()) {
                                *package_counts.entry(package.clone()).or_default() += 1;
                            }
                            if !packages.contains(package) {
                                packages.push(package.clone());
                            }
                        }
                    }
                }
                ScanOutcome::Incomplete | ScanOutcome::Pending => counts_complete = false,
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
            login: input.login,
            kind: input.kind,
            owner: input.owner,
            installed: installation.is_some(),
            installation_id: installation.map(|value| value.id),
            repository_selection: installation.and_then(|value| {
                let selection = value.repository_selection.clone();
                (!selection.is_empty()).then_some(selection)
            }),
            counts_complete,
            repos: repo_views,
        });
    }

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
        global_admin,
        accounts = accounts.len(),
        sessions = total_sessions,
        "canvas overview assembled"
    );
    OverviewResponse {
        app_slug: state
            .github_app
            .as_ref()
            .and_then(|github| github.app_slug().map(str::to_string)),
        viewer: Viewer { login: user.login },
        global_admin,
        accounts,
        totals: OverviewTotals {
            sessions: total_sessions,
            packages,
        },
        // Connecting a broader user token cannot widen an already App-wide
        // administrator view, so suppress the inert connect affordance.
        broader_oauth_available: !global_admin && state.config.log.broader_oauth().is_some(),
    }
}

/// `GET /api/v1/overview` — the whole-account canvas, computed live.
#[utoipa::path(
    get,
    path = "/overview",
    tag = "canvas",
    operation_id = "canvas_overview",
    params(
        ("X-Github-Broader-Token" = Option<String>, Header, description = "Optional broader-visibility OAuth token (issue #572); when it verifies to the same GitHub id as the Bearer identity it drives repo/org enumeration so repos/orgs where the App is not installed still appear. Ignored on mismatch/verify failure."),
    ),
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
    let global_admin = state.config.access.is_global_admin(user.id, &user.login);
    let accounts = if global_admin {
        global_admin_account_inputs(&state, &gh, &user).await?
    } else {
        user_account_inputs(&state, &gh, &user, &token, &headers).await?
    };
    Ok(Json(
        assemble_overview(&state, &gh, user, global_admin, accounts).await,
    ))
}

#[cfg(test)]
#[path = "overview_tests.rs"]
mod tests;
