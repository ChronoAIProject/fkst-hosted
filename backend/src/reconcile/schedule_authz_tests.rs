use k8s_openapi::chrono::TimeZone;

use crate::goals::package_env::PackageEnv;
use crate::goals::trigger_parse::PackageRef;
use crate::models::RepoRef;
use crate::reconcile::desired::SessionDef;

use super::*;

/// A body that would FAIL to parse. Used to prove the denial paths never look at
/// it: if any predicate read the body first, these tests would report a parse
/// error instead of the authorization reason they assert.
const UNPARSEABLE_BODY: &str = "### Workflow\n### Workflow\nnot a definition at all";

fn created() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 27, 8, 0, 0)
        .single()
        .expect("valid timestamp")
}

/// The creator's immutable GitHub id. Authority matches a creator by id when one
/// is known, so a fixture that only shares the login would be denied — which is
/// the behaviour, not a bug.
const CREATOR_ID: i64 = 7;
const STRANGER_ID: i64 = 4242;

fn issue(number: i64, author: &str, author_id: i64, assignees: &[&str]) -> IssueSummary {
    IssueSummary {
        number,
        title: "nightly sourcing".to_string(),
        body: UNPARSEABLE_BODY.to_string(),
        labels: vec![crate::reconcile::reserved_labels::SCHEDULED_WORKFLOW_LABEL.to_string()],
        state: "open".to_string(),
        assignees: assignees.iter().map(|value| value.to_string()).collect(),
        user_login: author.to_string(),
        user_id: author_id,
        created_at: created(),
    }
}

/// The common case: the session's own creator opening a schedule.
fn by_creator(number: i64, creator: &str, assignees: &[&str]) -> IssueSummary {
    issue(number, creator, CREATOR_ID, assignees)
}

fn registration(trigger_issue: i64, creator_login: &str) -> SessionRegistration {
    SessionRegistration {
        installation_id: 42,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        trigger_issue,
        trigger_author_id: 7,
        trigger_author_login: creator_login.to_string(),
        creator_login: creator_login.to_string(),
        creator_id: Some(7),
        def: SessionDef {
            name: "site".to_string(),
            packages: Vec::<PackageRef>::new(),
            manifest_refs: Vec::<PackageRef>::new(),
            work_label: Some("fkst-dev".to_string()),
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

fn open_access() -> AccessPolicy {
    AccessPolicy::from_vars(&[]).expect("open access policy")
}

#[test]
fn a_routed_issue_from_the_creator_is_authorized_and_names_its_session() {
    let regs = vec![registration(10, "alice")];
    let (authorized, owner) = authorize_schedule_issue(
        &by_creator(50, "alice", &["alice"]),
        &regs,
        &open_access(),
        None,
    )
    .expect("the creator may schedule for their own session");
    assert_eq!(owner.trigger_issue, 10);
    assert_eq!(authorized.number(), 50);
    assert_eq!(authorized.created_at(), created());
    assert_eq!(authorized.body(), UNPARSEABLE_BODY);
    assert!(authorized
        .labels()
        .contains(&crate::reconcile::reserved_labels::SCHEDULED_WORKFLOW_LABEL.to_string()));
}

#[test]
fn assignee_matching_is_case_insensitive_like_work_routing() {
    let regs = vec![registration(10, "Alice")];
    assert!(authorize_schedule_issue(
        &by_creator(50, "alice", &["ALICE"]),
        &regs,
        &open_access(),
        None
    )
    .is_ok());
}

#[test]
fn zero_or_several_assignees_are_unrouted_without_reading_the_body() {
    for assignees in [vec![], vec!["alice", "bob"]] {
        let regs = vec![registration(10, "alice")];
        let denial = authorize_schedule_issue(
            &by_creator(50, "alice", &assignees),
            &regs,
            &open_access(),
            None,
        )
        .expect_err("ambiguous routing is refused");
        assert!(matches!(denial, ScheduleDenial::Unrouted(_)), "{denial:?}");
        assert!(
            denial.detail().contains("exactly one assignee"),
            "{denial:?}"
        );
    }
}

#[test]
fn an_assignee_with_no_session_is_refused_naming_the_fix() {
    let regs = vec![registration(10, "alice")];
    let denial = authorize_schedule_issue(
        &by_creator(50, "alice", &["bob"]),
        &regs,
        &open_access(),
        None,
    )
    .expect_err("nobody runs sessions for bob");
    assert!(matches!(denial, ScheduleDenial::NoSession(_)), "{denial:?}");
    assert!(denial.detail().contains("bob"), "{denial:?}");
    assert!(
        denial.detail().contains("fkst-substrate-trigger"),
        "states the remedy: {denial:?}"
    );
}

#[test]
fn an_unauthorized_author_is_refused_even_when_routing_succeeds() {
    // Routed to alice's session, but authored by a stranger. Authority is decided
    // on the AUTHOR, from metadata, exactly as the pending gate decides it.
    let regs = vec![registration(10, "alice")];
    let denial = authorize_schedule_issue(
        &issue(50, "stranger", STRANGER_ID, &["alice"]),
        &regs,
        &open_access(),
        None,
    )
    .expect_err("a stranger may not schedule work for alice's session");
    assert!(
        matches!(denial, ScheduleDenial::Unauthorized(_)),
        "{denial:?}"
    );
    assert!(
        denial.detail().contains("#10"),
        "names the session: {denial:?}"
    );
}

#[test]
fn a_listed_collaborator_may_schedule() {
    let mut reg = registration(10, "alice");
    reg.collaborators = vec!["bob".to_string()];
    let regs = vec![reg];
    assert!(
        authorize_schedule_issue(
            &issue(50, "bob", STRANGER_ID, &["alice"]),
            &regs,
            &open_access(),
            None
        )
        .is_ok(),
        "a `### Session Collaborators` login carries work authority"
    );
}

#[test]
fn the_configured_app_is_accepted_as_a_system_principal() {
    let regs = vec![registration(10, "alice")];
    assert!(authorize_schedule_issue(
        &issue(50, "fkst-app[bot]", STRANGER_ID, &["alice"]),
        &regs,
        &open_access(),
        Some("fkst-app[bot]"),
    )
    .is_ok());
    // ...and only when the deployment actually configured that identity.
    assert!(authorize_schedule_issue(
        &issue(50, "fkst-app[bot]", STRANGER_ID, &["alice"]),
        &regs,
        &open_access(),
        None,
    )
    .is_err());
}

#[test]
fn the_lowest_trigger_issue_owns_the_schedule_when_a_creator_runs_several_sessions() {
    // Deliberately supplied out of order: ownership must not depend on input order.
    let regs = vec![
        registration(31, "alice"),
        registration(12, "alice"),
        registration(20, "alice"),
    ];
    let (_, owner) = authorize_schedule_issue(
        &by_creator(50, "alice", &["alice"]),
        &regs,
        &open_access(),
        None,
    )
    .expect("routed");
    assert_eq!(owner.trigger_issue, 12);
}

#[test]
fn authorization_never_depends_on_body_content() {
    // The same metadata with two wildly different bodies must reach the same
    // decision — the structural guarantee behind the carve-out.
    let regs = vec![registration(10, "alice")];
    let mut valid_body = issue(50, "stranger", STRANGER_ID, &["alice"]);
    valid_body.body = "### Workflow\nsourcing\n\n### Run Mode\nonce\n".to_string();
    let with_valid = authorize_schedule_issue(&valid_body, &regs, &open_access(), None);
    let with_garbage = authorize_schedule_issue(
        &issue(50, "stranger", STRANGER_ID, &["alice"]),
        &regs,
        &open_access(),
        None,
    );
    assert_eq!(with_valid.err(), with_garbage.err());
}
