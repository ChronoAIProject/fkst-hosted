//! The per-repository pass: authorization, parse rejection, validation, history
//! trust, and the per-creator cap — all against fakes, no cluster.

use std::collections::HashMap;

use async_trait::async_trait;
use k8s_openapi::chrono::TimeZone;
use secrecy::SecretString;

use crate::github_app::comments::IssueComment;
use crate::github_app::listing::InstallationSummary;
use crate::github_app::GithubAppError;
use crate::goals::package_env::PackageEnv;
use crate::goals::trigger_parse::PackageRef;
use crate::reconcile::desired::SessionDef;
use crate::reconcile::reserved_labels::{CRON_RUNNING_LABEL, SCHEDULE_INVALID_LABEL};
use crate::schedule::{render_marker, RunRecord, RunStatus};

use super::*;

const BOT: &str = "fkst-app[bot]";

fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, day, hour, 0, 0)
        .single()
        .expect("valid timestamp")
}

fn token() -> SecretString {
    SecretString::from("ghs_test".to_string())
}

fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

/// A definition issue authored and assigned to `alice`, the session creator.
fn definition(number: i64, body: &str, labels: &[&str]) -> IssueSummary {
    IssueSummary {
        number,
        title: format!("schedule {number}"),
        body: body.to_string(),
        labels: std::iter::once(SCHEDULED_WORKFLOW_LABEL.to_string())
            .chain(labels.iter().map(|label| (*label).to_string()))
            .collect(),
        state: "open".to_string(),
        assignees: vec!["alice".to_string()],
        user_login: "alice".to_string(),
        user_id: 7,
        created_at: at(27, 0),
    }
}

const HOURLY: &str = "### Workflow\nsourcing\n\n### Run Mode\ncron: 0 * * * *\n";

fn registration(trigger_issue: i64, work_label: Option<&str>) -> SessionRegistration {
    SessionRegistration {
        installation_id: 42,
        repo: repo(),
        trigger_issue,
        trigger_author_id: 7,
        trigger_author_login: "alice".to_string(),
        creator_login: "alice".to_string(),
        creator_id: Some(7),
        def: SessionDef {
            name: "site".to_string(),
            packages: Vec::<PackageRef>::new(),
            manifest_refs: Vec::<PackageRef>::new(),
            work_label: work_label.map(str::to_string),
            environment: None,
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
            source_branch: None,
            target_branch: None,
            package_env: PackageEnv::new(),
        },
        effective_packages: Vec::new(),
        effective_package_env: PackageEnv::new(),
        session_id: format!("sess-{trigger_issue}"),
        config_hash: "hash".to_string(),
        auto_merge: false,
        log_access: Vec::new(),
        collaborators: Vec::new(),
    }
}

struct Listing(Vec<IssueSummary>);

#[async_trait]
impl GithubListing for Listing {
    async fn list_issues_by_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        label: &str,
    ) -> Result<Vec<IssueSummary>, GithubAppError> {
        assert_eq!(
            label, SCHEDULED_WORKFLOW_LABEL,
            "the pass enumerates only definitions"
        );
        Ok(self.0.clone())
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
    ) -> Result<Vec<crate::models::GithubActor>, GithubAppError> {
        Ok(Vec::new())
    }
}

/// Comment history keyed by issue number, each entry `(author, body)`.
#[derive(Default)]
struct Comments(HashMap<i64, Vec<(String, String)>>);

impl Comments {
    fn with(issue: i64, entries: &[(&str, String)]) -> Self {
        Self(HashMap::from([(
            issue,
            entries
                .iter()
                .map(|(author, body)| ((*author).to_string(), body.clone()))
                .collect(),
        )]))
    }
}

#[async_trait]
impl IssueCommentReader for Comments {
    async fn list_recent_issue_comments(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        number: u64,
        _max_pages: u32,
    ) -> Result<Vec<IssueComment>, GithubAppError> {
        Ok(self
            .0
            .get(&(number as i64))
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|(user_login, body)| IssueComment {
                body,
                user_login,
                created_at: at(27, 0),
            })
            .collect())
    }
}

/// A reader that always fails, to prove a read failure never reads as "no history".
struct BrokenComments;

#[async_trait]
impl IssueCommentReader for BrokenComments {
    async fn list_recent_issue_comments(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _max_pages: u32,
    ) -> Result<Vec<IssueComment>, GithubAppError> {
        Err(GithubAppError::Http("transport down".to_string()))
    }
}

fn cfg() -> ReconcileConfig {
    ReconcileConfig {
        github_bot_login: Some(BOT.to_string()),
        ..ReconcileConfig::default()
    }
}

async fn run(
    issues: Vec<IssueSummary>,
    comments: &dyn IssueCommentReader,
    regs: &[SessionRegistration],
    now: DateTime<Utc>,
    cfg: &ReconcileConfig,
) -> Result<Vec<ScheduleEffect>, AppError> {
    plan_repo_schedules(
        &Listing(issues),
        comments,
        &token(),
        &repo(),
        regs,
        &AccessPolicy::from_vars(&[]).expect("open policy"),
        now,
        cfg,
    )
    .await
}

fn detail(effect: &ScheduleEffect) -> &str {
    match effect {
        ScheduleEffect::FlagInvalid { detail, .. } => detail,
        other => panic!("expected an invalid flag, got {other:?}"),
    }
}

// ---- the happy path --------------------------------------------------------

#[tokio::test]
async fn a_due_definition_dispatches_with_the_runners_own_work_label() {
    // The label comes from the RUN MODE, not the session — so a session whose own
    // label is `fkst-dev` still produces a run issue the dev loop will not touch.
    let regs = vec![registration(10, Some("fkst-dev"))];
    let effects = run(
        vec![definition(50, HOURLY, &[])],
        &Comments::default(),
        &regs,
        at(27, 1),
        &cfg(),
    )
    .await
    .expect("the pass succeeds");
    let [ScheduleEffect::Dispatch { request, .. }] = effects.as_slice() else {
        panic!("expected one dispatch, got {effects:?}");
    };
    assert_eq!(request.work_label, "fkst-workflow-scheduled");
    assert_eq!(request.creator_login, "alice");
    assert_eq!(request.schedule_issue, 50);
}

#[test]
fn the_namespace_is_applied_to_the_run_issues_label() {
    // The run issue must carry the DEPLOYMENT-effective label, or it routes to
    // nothing on a namespaced deployment.
    let cfg = ReconcileConfig {
        work_label_namespace: Some("chronoai-fkst".to_string()),
        ..cfg()
    };
    let resolved = run_issue_work_label(&RunMode::Once, &cfg).expect("resolves");
    assert_eq!(resolved, "fkst-workflow-run-chronoai-fkst");
}

/// The two run modes, taken from the same parser production uses rather than
/// hand-built, so a grammar change cannot leave these tests asserting a shape the
/// parser no longer produces.
fn both_run_modes() -> [RunMode; 2] {
    let cron = parse_scheduled_workflow(HOURLY)
        .expect("hourly parses")
        .run_mode;
    [RunMode::Once, cron]
}

#[test]
fn a_cron_run_and_a_one_time_run_carry_different_labels() {
    // Split so a repeating cadence is distinguishable from a one-shot at a glance,
    // in the issue list and in any label-scoped query an operator writes.
    let cfg = cfg();
    let [once, cron] = both_run_modes();
    let once = run_issue_work_label(&once, &cfg).expect("resolves");
    let cron = run_issue_work_label(&cron, &cfg).expect("resolves");
    assert_eq!(once, "fkst-workflow-run");
    assert_eq!(cron, "fkst-workflow-scheduled");
    assert_ne!(once, cron);
}

#[test]
fn a_run_issue_never_carries_a_dev_or_authoring_label() {
    // The regression #5890 is about. `fkst-dev` is admitted by the dev intake,
    // which gates on labels BEFORE it reads a body and so cannot see the dispatch
    // marker that identifies a run — it would triage and implement the run as
    // ordinary development work. `fkst-workflow` is workflow-writer's AUTHORING
    // queue, which would answer a run with a template pull request.
    let cfg = cfg();
    for mode in both_run_modes() {
        let label = run_issue_work_label(&mode, &cfg).expect("resolves");
        assert!(
            !label.starts_with("fkst-dev"),
            "a run issue must never carry a dev label, got {label}"
        );
        assert_ne!(label, "fkst-workflow");
    }
}

#[tokio::test]
async fn a_session_with_several_work_labels_can_still_run_a_schedule() {
    // Before #5890 the run label came from the SESSION and had to resolve to
    // exactly one, so a deployment whose mandatory package set declares three
    // rejected every schedule on every session. The run label no longer consults
    // the session at all, so the count cannot matter.
    let regs = vec![registration(10, None)];
    let effects = run(
        vec![definition(50, HOURLY, &[])],
        &Comments::default(),
        &regs,
        at(27, 1),
        &cfg(),
    )
    .await
    .expect("the pass succeeds");
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, ScheduleEffect::FlagInvalid { .. })),
        "three session labels must not make a schedule invalid: {effects:?}"
    );
}

// ---- authorization + rejection ---------------------------------------------

#[tokio::test]
async fn an_unroutable_definition_is_latched_invalid_rather_than_ignored() {
    let regs = vec![registration(10, Some("fkst-dev"))];
    let mut issue = definition(50, HOURLY, &[]);
    issue.assignees.clear();
    let effects = run(vec![issue], &Comments::default(), &regs, at(27, 1), &cfg())
        .await
        .expect("the pass succeeds");
    assert!(
        detail(&effects[0]).contains("exactly one assignee"),
        "{effects:?}"
    );
}

#[tokio::test]
async fn a_malformed_body_is_latched_invalid_with_the_parser_message() {
    let regs = vec![registration(10, Some("fkst-dev"))];
    let effects = run(
        vec![definition(50, "### Workflow\nsourcing\n", &[])],
        &Comments::default(),
        &regs,
        at(27, 1),
        &cfg(),
    )
    .await
    .expect("the pass succeeds");
    assert!(detail(&effects[0]).contains("### Run Mode"), "{effects:?}");
}

#[tokio::test]
async fn a_too_tight_cadence_is_latched_invalid() {
    let regs = vec![registration(10, Some("fkst-dev"))];
    let body = "### Workflow\nsourcing\n\n### Run Mode\ncron: */5 * * * *\n";
    let effects = run(
        vec![definition(50, body, &[])],
        &Comments::default(),
        &regs,
        at(27, 1),
        &cfg(),
    )
    .await
    .expect("the pass succeeds");
    assert!(
        detail(&effects[0]).contains("minimum is 900s"),
        "{effects:?}"
    );
}

#[tokio::test]
async fn a_single_discovered_label_needs_no_explicit_declaration() {
    let regs = vec![registration(10, None)];
    let effects = run(
        vec![definition(50, HOURLY, &[])],
        &Comments::default(),
        &regs,
        at(27, 1),
        &cfg(),
    )
    .await
    .expect("the pass succeeds");
    assert!(
        matches!(effects[0], ScheduleEffect::Dispatch { .. }),
        "{effects:?}"
    );
}

#[tokio::test]
async fn beyond_the_per_creator_cap_the_lowest_numbered_definitions_keep_running() {
    // Rejecting the whole set would let one accidental burst take down a creator's
    // working schedules.
    let regs = vec![registration(10, Some("fkst-dev"))];
    let cfg = ReconcileConfig {
        cron_max_jobs_per_creator: 2,
        ..cfg()
    };
    let effects = run(
        vec![
            definition(50, HOURLY, &[]),
            definition(51, HOURLY, &[]),
            definition(52, HOURLY, &[]),
        ],
        &Comments::default(),
        &regs,
        at(27, 1),
        &cfg,
    )
    .await
    .expect("the pass succeeds");
    assert!(
        matches!(effects[0], ScheduleEffect::Dispatch { .. }),
        "{effects:?}"
    );
    assert!(
        matches!(effects[1], ScheduleEffect::Dispatch { .. }),
        "{effects:?}"
    );
    assert_eq!(effects[2].schedule_issue(), 52);
    assert!(
        detail(&effects[2]).contains("deployment limit"),
        "{effects:?}"
    );
}

// ---- history trust ---------------------------------------------------------

#[tokio::test]
async fn only_bot_authored_records_are_trusted_as_history() {
    // A forged terminal record from a repository collaborator must not be able to
    // silence a schedule, nor a forged dispatch strand it.
    let regs = vec![registration(10, Some("fkst-dev"))];
    let forged = render_marker(&RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1)));
    let comments = Comments::with(50, &[("mallory", forged)]);
    let effects = run(
        vec![definition(50, HOURLY, &[])],
        &comments,
        &regs,
        at(27, 1),
        &cfg(),
    )
    .await
    .expect("the pass succeeds");
    assert!(
        matches!(effects[0], ScheduleEffect::Dispatch { .. }),
        "the forged record was ignored, so the slot is still due: {effects:?}"
    );
}

#[tokio::test]
async fn a_bot_authored_record_advances_the_cursor() {
    let regs = vec![registration(10, Some("fkst-dev"))];
    let record = render_marker(&RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1)));
    let comments = Comments::with(50, &[(BOT, record)]);
    let effects = run(
        vec![definition(50, HOURLY, &[])],
        &comments,
        &regs,
        at(27, 1),
        &cfg(),
    )
    .await
    .expect("the pass succeeds");
    assert!(effects.is_empty(), "the slot already ran: {effects:?}");
}

#[tokio::test]
async fn the_bot_login_comparison_tolerates_the_bot_suffix() {
    // GitHub renders an App author as `<slug>[bot]` in some contexts and `<slug>`
    // in others; a mismatch here would silently discard the whole history.
    let regs = vec![registration(10, Some("fkst-dev"))];
    let record = render_marker(&RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1)));
    let comments = Comments::with(50, &[("fkst-app", record)]);
    let effects = run(
        vec![definition(50, HOURLY, &[])],
        &comments,
        &regs,
        at(27, 1),
        &cfg(),
    )
    .await
    .expect("the pass succeeds");
    assert!(effects.is_empty(), "{effects:?}");
}

#[tokio::test]
async fn a_history_read_failure_fails_the_pass_rather_than_reading_as_empty() {
    // An empty history from a FAILED read would reset the cursor and re-run a slot
    // that already ran — the one failure mode a scheduler must not have.
    let regs = vec![registration(10, Some("fkst-dev"))];
    assert!(run(
        vec![definition(50, HOURLY, &[])],
        &BrokenComments,
        &regs,
        at(27, 1),
        &cfg(),
    )
    .await
    .is_err());
}

// ---- gating ----------------------------------------------------------------

#[tokio::test]
async fn without_a_configured_app_identity_the_clock_does_not_run() {
    // Nothing could be trusted as a record, so every definition would look as if it
    // had never run and re-fire on every sweep.
    let regs = vec![registration(10, Some("fkst-dev"))];
    let cfg = ReconcileConfig {
        github_bot_login: None,
        ..cfg()
    };
    let effects = run(
        vec![definition(50, HOURLY, &[])],
        &Comments::default(),
        &regs,
        at(27, 1),
        &cfg,
    )
    .await
    .expect("the pass succeeds");
    assert!(effects.is_empty());
}

#[tokio::test]
async fn a_repository_with_no_definitions_costs_one_list_and_reads_nothing_else() {
    let regs = vec![registration(10, Some("fkst-dev"))];
    let effects = run(Vec::new(), &BrokenComments, &regs, at(27, 1), &cfg())
        .await
        .expect("no definitions, no comment reads to fail");
    assert!(effects.is_empty());
}

#[tokio::test]
async fn a_fixed_definition_clears_its_latch_and_runs_the_same_pass() {
    let regs = vec![registration(10, Some("fkst-dev"))];
    let effects = run(
        vec![definition(50, HOURLY, &[SCHEDULE_INVALID_LABEL])],
        &Comments::default(),
        &regs,
        at(27, 1),
        &cfg(),
    )
    .await
    .expect("the pass succeeds");
    assert!(
        matches!(effects[0], ScheduleEffect::ClearInvalid { .. }),
        "{effects:?}"
    );
    assert!(
        matches!(effects[1], ScheduleEffect::Dispatch { .. }),
        "{effects:?}"
    );
}

#[tokio::test]
async fn an_in_flight_definition_skips_rather_than_starting_a_second_run() {
    let regs = vec![registration(10, Some("fkst-dev"))];
    let dispatched = render_marker(&RunRecord::new(at(27, 1), RunStatus::Dispatched, at(27, 1)));
    let comments = Comments::with(50, &[(BOT, dispatched)]);
    // A budget wider than the cadence, so the overlap is observable: with the 1h
    // default the 02:00 slot would arrive exactly as the watchdog fires.
    let cfg = ReconcileConfig {
        cron_max_runtime_secs: 4 * 3600,
        ..cfg()
    };
    let effects = run(
        vec![definition(50, HOURLY, &[CRON_RUNNING_LABEL])],
        &comments,
        &regs,
        at(27, 2),
        &cfg,
    )
    .await
    .expect("the pass succeeds");
    assert_eq!(
        effects,
        vec![ScheduleEffect::RecordSkip {
            schedule_issue: 50,
            slot: at(27, 2)
        }]
    );
}
