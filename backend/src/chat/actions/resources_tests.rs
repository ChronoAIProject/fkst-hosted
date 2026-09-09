//! Tests for the resource proposals (sibling `#[path]` module).
//!
//! The rule these all serve: a draft that validates HERE must be one the real endpoint
//! accepts. Every rejection case below mirrors a rule the endpoint enforces, so a card can
//! never promise something that fails the moment the user confirms it.

use super::*;

fn install() -> Vec<String> {
    vec!["npm ci".to_string()]
}

// ---- create repository ---------------------------------------------------

#[test]
fn create_repository_drafts_a_personal_private_repo() {
    let proposal =
        propose_create_repository(None, "site-builder", true, None).expect("valid draft");
    let ActionProposal::CreateRepository {
        owner,
        name,
        private,
        description,
        summary,
        target,
    } = proposal
    else {
        panic!("expected a CreateRepository proposal");
    };
    assert_eq!(owner, None);
    assert_eq!(name, "site-builder");
    assert!(private);
    assert_eq!(description, None);
    assert!(summary.contains("private"), "summary was {summary:?}");
    assert!(
        summary.contains("personal account"),
        "summary was {summary:?}"
    );
    assert_eq!(target.method, "POST");
    assert_eq!(target.path, "/api/v1/repos");
}

#[test]
fn create_repository_keeps_the_org_and_description() {
    let proposal = propose_create_repository(
        Some("  acme  ".to_string()),
        "docs",
        false,
        Some("  Public docs  ".to_string()),
    )
    .expect("valid draft");
    let ActionProposal::CreateRepository {
        owner,
        description,
        summary,
        ..
    } = proposal
    else {
        panic!("expected a CreateRepository proposal");
    };
    // Trimmed, because a model that pads a value should not produce a different repo.
    assert_eq!(owner.as_deref(), Some("acme"));
    assert_eq!(description.as_deref(), Some("Public docs"));
    assert!(summary.contains("in acme"), "summary was {summary:?}");
    assert!(summary.contains("public"), "summary was {summary:?}");
}

#[test]
fn create_repository_rejects_a_blank_name() {
    let error = propose_create_repository(None, "   ", false, None).expect_err("blank name");
    assert!(error.to_string().contains("must not be empty"));
}

#[test]
fn create_repository_rejects_illegal_name_characters() {
    let error = propose_create_repository(None, "my repo!", false, None).expect_err("bad name");
    assert!(
        error.to_string().contains("[A-Za-z0-9._-]"),
        "error was {error}"
    );
}

#[test]
fn create_repository_rejects_an_over_long_name() {
    let name = "a".repeat(MAX_REPO_NAME_CHARS + 1);
    let error = propose_create_repository(None, &name, false, None).expect_err("long name");
    assert!(error.to_string().contains("1-100"), "error was {error}");
}

#[test]
fn create_repository_rejects_an_over_long_description() {
    let description = "d".repeat(MAX_REPO_DESCRIPTION_CHARS + 1);
    let error = propose_create_repository(None, "ok", false, Some(description))
        .expect_err("long description");
    assert!(
        error.to_string().contains("description"),
        "error was {error}"
    );
}

// ---- save environment profile --------------------------------------------

#[test]
fn save_environment_profile_drafts_install_variables_and_secret_names() {
    let proposal = propose_save_environment_profile(
        "node-ci",
        vec!["npm ci".to_string(), "npm run build".to_string()],
        vec![("NODE_ENV".to_string(), "production".to_string())],
        vec!["NPM_TOKEN".to_string()],
        Some(false),
    )
    .expect("valid draft");
    let ActionProposal::SaveEnvironmentProfile {
        profile_name,
        replaces_existing,
        install,
        variables,
        secret_keys,
        summary,
        target,
    } = proposal
    else {
        panic!("expected a SaveEnvironmentProfile proposal");
    };
    assert_eq!(profile_name, "node-ci");
    assert_eq!(replaces_existing, Some(false));
    assert_eq!(install.len(), 2);
    assert_eq!(
        variables,
        vec![EnvVarDraft {
            key: "NODE_ENV".to_string(),
            value: "production".to_string(),
        }]
    );
    assert_eq!(secret_keys, vec!["NPM_TOKEN".to_string()]);
    assert!(summary.starts_with("Create"), "summary was {summary:?}");
    assert_eq!(target.method, "PUT");
    assert_eq!(target.path, "/api/v1/users/me/environment-profiles/node-ci");
}

#[test]
fn save_environment_profile_says_replace_when_the_name_already_exists() {
    let proposal =
        propose_save_environment_profile("node-ci", install(), Vec::new(), Vec::new(), Some(true))
            .expect("valid draft");
    assert!(
        proposal.summary().starts_with("Replace"),
        "summary was {:?}",
        proposal.summary()
    );
}

#[test]
fn save_environment_profile_reads_as_create_when_existence_is_unknown() {
    // `None` must not silently claim a replacement — the safer wording wins.
    let proposal =
        propose_save_environment_profile("node-ci", install(), Vec::new(), Vec::new(), None)
            .expect("valid draft");
    assert!(proposal.summary().starts_with("Create"));
}

#[test]
fn save_environment_profile_rejects_a_name_the_store_would_refuse() {
    for name in ["Node-CI", "-lead", "trail-", "under_score", ""] {
        let error = propose_save_environment_profile(name, install(), Vec::new(), Vec::new(), None)
            .expect_err("invalid name");
        assert!(
            !error.to_string().is_empty(),
            "expected a message for {name:?}"
        );
    }
}

#[test]
fn save_environment_profile_requires_at_least_one_install_command() {
    // Matches the endpoint's `validate_install(.., require_one = true)`.
    let error = propose_save_environment_profile(
        "node-ci",
        vec!["   ".to_string()],
        Vec::new(),
        Vec::new(),
        None,
    )
    .expect_err("no install commands");
    assert!(
        error.to_string().contains("at least one install command"),
        "error was {error}"
    );
}

#[test]
fn save_environment_profile_rejects_a_reserved_variable_name() {
    let error = propose_save_environment_profile(
        "node-ci",
        install(),
        vec![(LLM_ENV_KEY.to_string(), "x".to_string())],
        Vec::new(),
        None,
    )
    .expect_err("reserved key");
    assert!(error.to_string().contains("reserved"), "error was {error}");
}

#[test]
fn save_environment_profile_rejects_a_malformed_variable_name() {
    let error = propose_save_environment_profile(
        "node-ci",
        install(),
        vec![("2BAD".to_string(), "x".to_string())],
        Vec::new(),
        None,
    )
    .expect_err("bad key");
    assert!(
        error.to_string().contains("must match"),
        "error was {error}"
    );
}

#[test]
fn save_environment_profile_rejects_a_malformed_secret_name() {
    let error = propose_save_environment_profile(
        "node-ci",
        install(),
        Vec::new(),
        vec!["has-dash".to_string()],
        None,
    )
    .expect_err("bad secret key");
    assert!(error.to_string().contains("secret"), "error was {error}");
}

#[test]
fn save_environment_profile_rejects_a_name_used_as_both_variable_and_secret() {
    let error = propose_save_environment_profile(
        "node-ci",
        install(),
        vec![("TOKEN".to_string(), "plain".to_string())],
        vec!["TOKEN".to_string()],
        None,
    )
    .expect_err("collision");
    assert!(error.to_string().contains("BOTH"), "error was {error}");
}

#[test]
fn save_environment_profile_rejects_runaway_drafts() {
    let many = (0..MAX_DRAFT_INSTALL_COMMANDS + 1)
        .map(|i| format!("echo {i}"))
        .collect();
    let error = propose_save_environment_profile("node-ci", many, Vec::new(), Vec::new(), None)
        .expect_err("too many commands");
    assert!(error.to_string().contains("at most"), "error was {error}");

    let long = vec!["x".repeat(MAX_DRAFT_INSTALL_COMMAND_CHARS + 1)];
    let error = propose_save_environment_profile("node-ci", long, Vec::new(), Vec::new(), None)
        .expect_err("command too long");
    assert!(error.to_string().contains("exceeds"), "error was {error}");
}

// ---- delete environment profile ------------------------------------------

#[test]
fn delete_environment_profile_drafts_the_delete() {
    let proposal = propose_delete_environment_profile("node-ci").expect("valid draft");
    let ActionProposal::DeleteEnvironmentProfile {
        profile_name,
        summary,
        target,
    } = proposal
    else {
        panic!("expected a DeleteEnvironmentProfile proposal");
    };
    assert_eq!(profile_name, "node-ci");
    assert!(summary.contains("node-ci"), "summary was {summary:?}");
    assert_eq!(target.method, "DELETE");
    assert_eq!(target.path, "/api/v1/users/me/environment-profiles/node-ci");
}

#[test]
fn delete_environment_profile_rejects_an_invalid_name() {
    let error = propose_delete_environment_profile("Node CI").expect_err("invalid name");
    assert!(
        error.to_string().contains("must match"),
        "error was {error}"
    );
}

// ---- uninstall app -------------------------------------------------------

#[test]
fn uninstall_app_drafts_the_uninstall_with_its_reason() {
    let proposal =
        propose_uninstall_app("acme", "moving to a different account").expect("valid draft");
    let ActionProposal::UninstallApp {
        owner,
        reason,
        summary,
        target,
    } = proposal
    else {
        panic!("expected an UninstallApp proposal");
    };
    assert_eq!(owner, "acme");
    assert_eq!(reason, "moving to a different account");
    assert!(summary.contains("acme"), "summary was {summary:?}");
    assert_eq!(target.method, "DELETE");
    assert_eq!(target.path, "/api/v1/installations/acme");
}

#[test]
fn uninstall_app_requires_a_reason() {
    // Same rationale as the stop-session reason: the user must see WHY before confirming
    // something that affects every repository on the account.
    let error = propose_uninstall_app("acme", "  ").expect_err("blank reason");
    assert!(error.to_string().contains("must not be empty"));
}

#[test]
fn uninstall_app_rejects_an_over_long_reason() {
    let reason = "r".repeat(MAX_UNINSTALL_REASON_CHARS + 1);
    let error = propose_uninstall_app("acme", &reason).expect_err("long reason");
    assert!(error.to_string().contains("at most"), "error was {error}");
}

#[test]
fn uninstall_app_rejects_a_blank_owner() {
    let error = propose_uninstall_app("", "why").expect_err("blank owner");
    assert!(error.to_string().contains("owner"), "error was {error}");
}
