//! Unit tests for the pure trigger-issue → registration parse
//! ([`super::parse_registration`]).

use super::*;
use crate::github_app::listing::IssueSummary;
use crate::goals::trigger_parse::PackageRef;
use crate::reconcile::desired::config_hash;
use crate::session_spec::derive_session_id;

const INSTALLATION_ID: i64 = 42;

fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

/// A well-formed `fkst-substrate-trigger` issue body (canonical section order).
fn valid_body() -> String {
    "### Session Name\n\
     demo-session\n\n\
     ### Packages\n\
     acme/tools@main:pkg/demo\n\n\
     ### Work Label\n\
     fkst-demo\n"
        .to_string()
}

fn issue(number: i64, body: &str, user_id: i64) -> IssueSummary {
    IssueSummary {
        number,
        title: "trigger".to_string(),
        body: body.to_string(),
        labels: vec!["fkst-substrate-trigger".to_string()],
        state: "open".to_string(),
        assignees: vec![],
        user_login: "carol".to_string(),
        user_id,
    }
}

#[test]
fn valid_body_yields_a_registration() {
    let issue = issue(7, &valid_body(), 4242);
    let reg = parse_registration(INSTALLATION_ID, &repo(), &issue).expect("valid body parses");

    assert_eq!(reg.installation_id, INSTALLATION_ID);
    assert_eq!(reg.repo, repo());
    assert_eq!(reg.trigger_issue, 7);
    assert_eq!(reg.trigger_author_id, 4242);
    assert_eq!(reg.def.name, "demo-session");
    assert_eq!(
        reg.def.packages,
        vec![PackageRef {
            owner: "acme".to_string(),
            repo: "tools".to_string(),
            git_ref: "main".to_string(),
            path: "pkg/demo".to_string(),
        }]
    );
    assert_eq!(reg.def.work_label, "fkst-demo");
    assert_eq!(reg.def.environment, None);

    // The session id + config hash must match the canonical derivations.
    let want_id = derive_session_id(INSTALLATION_ID, "acme", "site", 7);
    assert_eq!(reg.session_id, want_id);
    let want_hash = config_hash(&reg.def.packages, &reg.def.work_label, None);
    assert_eq!(reg.config_hash, want_hash);
}

#[test]
fn registration_derivations_are_stable_across_calls() {
    let issue = issue(7, &valid_body(), 4242);
    let a = parse_registration(INSTALLATION_ID, &repo(), &issue).expect("parses");
    let b = parse_registration(INSTALLATION_ID, &repo(), &issue).expect("parses");
    assert_eq!(
        a.session_id, b.session_id,
        "session id must be deterministic"
    );
    assert_eq!(a.config_hash, b.config_hash, "config hash must be stable");
    assert_eq!(a, b, "the whole registration is deterministic");
}

#[test]
fn environment_section_is_captured() {
    let body = "### Session Name\n\
                demo\n\n\
                ### Packages\n\
                acme/tools@main:pkg/demo\n\n\
                ### Work Label\n\
                fkst-demo\n\n\
                ### Environment\n\
                staging\n";
    let reg = parse_registration(INSTALLATION_ID, &repo(), &issue(9, body, 1)).expect("parses");
    assert_eq!(reg.def.environment.as_deref(), Some("staging"));
    // The environment participates in the config hash.
    assert_eq!(
        reg.config_hash,
        config_hash(&reg.def.packages, &reg.def.work_label, Some("staging"))
    );
}

#[test]
fn invalid_body_returns_issue_number_and_detail() {
    // A body missing the required `### Packages` section is a 422 whose message
    // names the offending section.
    let body = "### Session Name\n\
                demo\n\n\
                ### Work Label\n\
                fkst-demo\n";
    let err = parse_registration(INSTALLATION_ID, &repo(), &issue(11, body, 1))
        .expect_err("missing Packages must fail");
    assert_eq!(err.0, 11, "the invalid marker carries the issue number");
    assert!(
        err.1.contains("### Packages"),
        "the detail names the offending section: {}",
        err.1
    );
}

#[test]
fn empty_body_returns_an_invalid_marker() {
    let err = parse_registration(INSTALLATION_ID, &repo(), &issue(12, "", 1))
        .expect_err("an empty body cannot parse");
    assert_eq!(err.0, 12);
    assert!(!err.1.is_empty(), "an explanatory detail is present");
}
