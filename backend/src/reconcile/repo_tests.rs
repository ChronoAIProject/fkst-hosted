use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use k8s_openapi::chrono::Utc;
use secrecy::SecretString;

use super::*;
use crate::github_app::listing::InstallationSummary;
use crate::github_app::GithubAppError;
use crate::models::GithubActor;
use crate::reconcile::desired::{KillReason, LivePod, PodLiveness, ReconcileAction};
use crate::reconcile_config::ReconcileConfig;
use crate::session_spec::derive_session_id;

enum Reply {
    Role(Option<String>),
    Error,
}

struct FakeListing {
    reply: Reply,
    calls: AtomicUsize,
}

impl FakeListing {
    fn role(role: Option<&str>) -> Self {
        Self {
            reply: Reply::Role(role.map(str::to_string)),
            calls: AtomicUsize::new(0),
        }
    }

    fn error() -> Self {
        Self {
            reply: Reply::Error,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl GithubListing for FakeListing {
    async fn list_issues_by_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _label: &str,
    ) -> Result<Vec<IssueSummary>, GithubAppError> {
        Ok(Vec::new())
    }

    async fn count_open_issues_with_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _label: &str,
    ) -> Result<u64, GithubAppError> {
        Ok(0)
    }

    async fn get_collaborator_role(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _username: &str,
    ) -> Result<Option<String>, GithubAppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.reply {
            Reply::Role(role) => Ok(role.clone()),
            Reply::Error => Err(GithubAppError::Http("temporary failure".to_string())),
        }
    }

    async fn list_installations(
        &self,
        _app_jwt: &SecretString,
    ) -> Result<Vec<InstallationSummary>, GithubAppError> {
        Ok(Vec::new())
    }

    async fn list_installation_repos(
        &self,
        _token: &SecretString,
    ) -> Result<Vec<RepoRef>, GithubAppError> {
        Ok(Vec::new())
    }

    async fn list_repo_admins(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<Vec<GithubActor>, GithubAppError> {
        Ok(Vec::new())
    }
}

fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

fn token() -> SecretString {
    SecretString::from("token".to_string())
}

fn valid_body() -> String {
    "### Session Name\ndemo\n\n### Packages\nacme/tools@main:pkg/demo\n\n### Work Label\nfkst-run\n"
        .to_string()
}

fn issue(body: &str, labels: &[&str]) -> IssueSummary {
    IssueSummary {
        number: 7,
        title: "trigger".to_string(),
        body: body.to_string(),
        labels: labels.iter().map(|label| label.to_string()).collect(),
        state: "open".to_string(),
        assignees: Vec::new(),
        user_login: "alice".to_string(),
        user_id: 4242,
    }
}

async fn classify(
    listing: &FakeListing,
    issue: IssueSummary,
    announced: &HashSet<i64>,
) -> ClassifiedTriggers {
    classify_triggers(
        42,
        &repo(),
        &[issue],
        listing,
        &token(),
        &AccessPolicy::default(),
        Some("fkst-app"),
        announced,
    )
    .await
}

fn plan_runtime(
    classified: &ClassifiedTriggers,
    live: &[LivePod],
    announced: &HashSet<i64>,
) -> Vec<ReconcileAction> {
    let labels: HashMap<String, Vec<String>> = classified
        .registrations
        .iter()
        .map(|reg| (reg.session_id.clone(), vec!["fkst-run".to_string()]))
        .collect();
    let pending: HashMap<String, bool> = classified
        .registrations
        .iter()
        .map(|reg| (reg.session_id.clone(), false))
        .collect();
    plan_repo(
        &classified.registrations,
        &labels,
        &classified.invalid,
        live,
        &pending,
        &HashSet::new(),
        announced,
        &HashMap::new(),
        &HashSet::new(),
        Utc::now(),
        &ReconcileConfig::default(),
    )
}

#[tokio::test]
async fn blocked_effective_creator_of_a_bot_authored_trigger_is_silently_dropped() {
    // Denylist enforcement must reach the EFFECTIVE creator, not just the issue
    // author: an App-authored (seeded) trigger's author is the bot, which a
    // denylist always admits — its sole ASSIGNEE is the creator. A blocked
    // assignee-creator must drop the trigger silently (no registration, no
    // unauthorized marker, no parse), which also revokes an already-running
    // seeded session on the next reconcile (issue #3376 review).
    let denylist = AccessPolicy::from_vars(&[
        ("FKST_AUTH_MODEL".to_string(), "denylist".to_string()),
        (
            "FKST_ACCESS_BLOCKED_USERS".to_string(),
            "mallory".to_string(),
        ),
    ])
    .expect("denylist policy parses");
    let bot_trigger = |assignee: &str| IssueSummary {
        assignees: vec![assignee.to_string()],
        user_login: "fkst-app[bot]".to_string(),
        user_id: 302043618,
        ..issue(&valid_body(), &[])
    };

    // The repo listing would grant admin — but the access gate runs first.
    let listing = FakeListing::role(Some("admin"));
    let classified = classify_triggers(
        42,
        &repo(),
        &[bot_trigger("Mallory")],
        &listing,
        &token(),
        &denylist,
        Some("fkst-app"),
        &HashSet::new(),
    )
    .await;
    assert!(classified.registrations.is_empty(), "must not register");
    assert!(classified.invalid.is_empty(), "must not be parsed");
    assert!(classified.unauthorized.is_empty(), "silent, not flagged");
    assert!(classified.authorized_issues.is_empty());

    // A NON-blocked assignee-creator on the same bot-authored trigger registers.
    let classified = classify_triggers(
        42,
        &repo(),
        &[bot_trigger("alice")],
        &listing,
        &token(),
        &denylist,
        Some("fkst-app"),
        &HashSet::new(),
    )
    .await;
    assert_eq!(classified.registrations.len(), 1, "non-blocked registers");
}

#[tokio::test]
async fn unauthorized_trigger_is_flagged_once_without_parsing_its_body() {
    // An empty body would enter the invalid-parser path if the authorization gate
    // accidentally ran after parsing. It must appear only in the auth marker set.
    let listing = FakeListing::role(Some("write"));
    let classified = classify(&listing, issue("", &[]), &HashSet::new()).await;
    assert!(classified.registrations.is_empty());
    assert!(
        classified.invalid.is_empty(),
        "unauthorized body was parsed"
    );
    assert_eq!(classified.unauthorized.len(), 1);

    let first = plan_trigger_authorization(
        &classified.unauthorized,
        &classified.authorized_issues,
        &HashSet::new(),
    );
    assert!(matches!(
        first.as_slice(),
        [ReconcileAction::FlagTriggerUnauthorized {
            trigger_issue: 7,
            ..
        }]
    ));
    assert!(plan_trigger_authorization(
        &classified.unauthorized,
        &classified.authorized_issues,
        &HashSet::from([7]),
    )
    .is_empty());
}

#[tokio::test]
async fn granting_maintain_registers_and_clears_the_rejection() {
    let listing = FakeListing::role(Some("maintain"));
    let classified = classify(&listing, issue(&valid_body(), &[]), &HashSet::new()).await;
    assert_eq!(classified.registrations.len(), 1);
    assert!(classified.authorized_issues.contains(&7));
    assert!(plan_runtime(&classified, &[], &HashSet::new())
        .iter()
        .any(|action| matches!(
            action,
            ReconcileAction::AnnounceSession {
                trigger_issue: 7,
                ..
            }
        )));
    assert_eq!(
        plan_trigger_authorization(
            &classified.unauthorized,
            &classified.authorized_issues,
            &HashSet::from([7]),
        ),
        vec![ReconcileAction::ClearTriggerUnauthorized { trigger_issue: 7 }]
    );
}

#[tokio::test]
async fn deferred_unlatched_trigger_has_no_registration_or_feedback() {
    let classified = classify(
        &FakeListing::error(),
        issue(&valid_body(), &[]),
        &HashSet::new(),
    )
    .await;
    assert!(classified.registrations.is_empty());
    assert!(classified.invalid.is_empty());
    assert!(classified.unauthorized.is_empty());
    assert!(classified.authorized_issues.is_empty());
}

#[tokio::test]
async fn deferred_announced_trigger_stays_desired_and_live_pod_is_not_orphaned() {
    let announced = HashSet::from([7]);
    let classified = classify(
        &FakeListing::error(),
        issue(&valid_body(), &[SUBSTRATE_ANNOUNCED_LABEL]),
        &announced,
    )
    .await;
    let reg = classified
        .registrations
        .first()
        .expect("announced registration is preserved");
    let live = [LivePod {
        session_id: reg.session_id.clone(),
        trigger_issue: 7,
        liveness: PodLiveness::Live,
        created_at: Utc::now(),
        last_pending_at: None,
        config_hash: Some(reg.config_hash.clone()),
        work_labels: vec!["fkst-run".to_string()],
    }];
    assert!(!plan_runtime(&classified, &live, &announced)
        .iter()
        .any(|action| matches!(
            action,
            ReconcileAction::Kill {
                reason: KillReason::TriggerClosed,
                ..
            }
        )));
    assert!(
        classified.authorized_issues.is_empty(),
        "deferred is not a clear decision"
    );
}

#[tokio::test]
async fn definitive_unauthorized_announced_trigger_orphans_and_retires_live_session() {
    let announced = HashSet::from([7]);
    let classified = classify(
        &FakeListing::role(Some("write")),
        issue(
            &valid_body(),
            &[SUBSTRATE_ANNOUNCED_LABEL, TRIGGER_UNAUTHORIZED_LABEL],
        ),
        &announced,
    )
    .await;
    assert!(classified.registrations.is_empty());

    let live = [LivePod {
        session_id: derive_session_id(42, "acme", "site", 7),
        trigger_issue: 7,
        liveness: PodLiveness::Live,
        created_at: Utc::now(),
        last_pending_at: None,
        config_hash: None,
        work_labels: vec!["fkst-run".to_string()],
    }];
    let actions = plan_runtime(&classified, &live, &announced);
    assert!(actions.iter().any(|action| matches!(
        action,
        ReconcileAction::Kill {
            reason: KillReason::TriggerClosed,
            ..
        }
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        ReconcileAction::RetireWorkIssues { work_labels }
            if work_labels == &["fkst-run".to_string()]
    )));
}
