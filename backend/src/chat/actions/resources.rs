//! Proposals for the resources a session runs *on*: repositories, named environment
//! profiles, and the App installation itself.
//!
//! These complete the concierge's write surface, so every mutation the dashboard can
//! perform is also reachable by asking for it in words. They obey the same confirm gate
//! as the session proposals — the model drafts, the user reviews, the SPA executes with
//! the user's own token.
//!
//! ## Where a secret value lives, and where it does not
//!
//! An environment profile is the one drafted resource with a secret dimension.
//! [`propose_save_environment_profile`] therefore accepts secret KEY NAMES and nothing
//! else: the model may say "this profile needs `NPM_TOKEN`", and the confirmation card
//! collects the value from the user directly. A secret value never reaches the model, the
//! SSE event, or a server-side log — the draft type has nowhere to put one.

use super::{
    clean_list, optional, required, ActionProposal, ActionTarget, EnvVarDraft, ProposalError,
};
use crate::environment_validation::valid_env_key;
use crate::reserved_env::{is_reserved_env_key, LLM_ENV_KEY};

/// Repository-name rule, mirroring `routes::repos::create_repo` exactly so a draft can
/// never be accepted here and rejected on confirm.
const MAX_REPO_NAME_CHARS: usize = 100;
/// Repository-description cap. Display-only here; GitHub's own limit is larger, and the
/// bound exists so a runaway draft cannot be streamed to the browser.
const MAX_REPO_DESCRIPTION_CHARS: usize = 350;
/// Environment-profile name cap, mirroring `routes::environments::MAX_NAME_LEN`.
const MAX_ENV_NAME_CHARS: usize = 40;
/// Install-command count cap for a DRAFT. The deployment's configured cap
/// (`FKST_ENV_INSTALL_MAX_COMMANDS`) is authoritative and is enforced on confirm; this is
/// a sanity bound so an obviously-runaway draft fails with a clear message instead of
/// rendering a hundred-line card.
const MAX_DRAFT_INSTALL_COMMANDS: usize = 50;
/// Per-command cap for a DRAFT, same rationale as the count.
const MAX_DRAFT_INSTALL_COMMAND_CHARS: usize = 4096;
/// Non-secret variable count cap for a DRAFT.
const MAX_DRAFT_VARIABLES: usize = 50;
/// Non-secret variable VALUE cap for a DRAFT.
const MAX_DRAFT_VARIABLE_VALUE_CHARS: usize = 2048;
/// Secret-key count cap for a DRAFT.
const MAX_DRAFT_SECRET_KEYS: usize = 25;
/// Uninstall-reason cap; display-only on the card.
const MAX_UNINSTALL_REASON_CHARS: usize = 500;

/// True when the name obeys the environment store's DNS-1123-ish rule
/// (`^[a-z0-9]([a-z0-9-]*[a-z0-9])?$`).
///
/// Hand-checked rather than regex-matched because the rule is three conditions and the
/// dependency-free form keeps this module's validation readable next to its neighbours.
fn valid_env_profile_name(name: &str) -> bool {
    let ok_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    !name.is_empty() && name.chars().all(ok_char) && !name.starts_with('-') && !name.ends_with('-')
}

/// Validate one environment-variable NAME against the same rules the endpoint applies.
///
/// A reserved name is rejected here rather than at confirm because the model can fix it
/// within the turn, and a card promising to set `LLM_API_KEY` would be a card that always
/// fails.
fn check_env_key(key: &str, kind: &str) -> Result<(), ProposalError> {
    if !valid_env_key(key) {
        return Err(ProposalError::new(format!(
            "{kind} name {key:?} is invalid: it must match ^[A-Za-z_][A-Za-z0-9_]*$"
        )));
    }
    if is_reserved_env_key(key) || key == LLM_ENV_KEY {
        return Err(ProposalError::new(format!(
            "{kind} name {key:?} is reserved by the platform and cannot be set"
        )));
    }
    Ok(())
}

/// Build a create-repository proposal.
///
/// `owner` is the ORGANIZATION to create under; `None` (or the viewer's own login, which
/// the endpoint normalizes) means their personal account. Whether the user may create in
/// that organization is GitHub's decision and is made on confirm with their own token.
pub fn propose_create_repository(
    owner: Option<String>,
    name: &str,
    private: bool,
    description: Option<String>,
) -> Result<ActionProposal, ProposalError> {
    let name = required(name, "the repository name")?;
    let valid = name.chars().count() <= MAX_REPO_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid {
        return Err(ProposalError::new(format!(
            "the repository name must be 1-{MAX_REPO_NAME_CHARS} characters of [A-Za-z0-9._-]"
        )));
    }
    let owner = optional(owner);
    let description = optional(description);
    if let Some(description) = &description {
        if description.chars().count() > MAX_REPO_DESCRIPTION_CHARS {
            return Err(ProposalError::new(format!(
                "the repository description must be at most {MAX_REPO_DESCRIPTION_CHARS} characters"
            )));
        }
    }

    let where_ = match &owner {
        Some(org) => format!("in {org}"),
        None => "on your personal account".to_string(),
    };
    let summary = format!(
        "Create the {} repository `{}` {}",
        if private { "private" } else { "public" },
        name,
        where_
    );
    Ok(ActionProposal::CreateRepository {
        target: ActionTarget {
            method: "POST".to_string(),
            path: "/api/v1/repos".to_string(),
        },
        owner,
        name,
        private,
        description,
        summary,
    })
}

/// Build a save-environment-profile proposal (create or replace).
///
/// `replaces_existing` is supplied by the tool, which looks the profile up first, so the
/// card can say "replace" rather than "create". It is `None` when that lookup could not
/// run — the card then states the ambiguity instead of guessing.
///
/// What is deliberately NOT checked here: the deployment's configured entry/byte caps and
/// whether the install commands actually succeed. The endpoint runs the real install in an
/// isolated validation pod on confirm, which is the only place that answer exists.
pub fn propose_save_environment_profile(
    profile_name: &str,
    install: Vec<String>,
    variables: Vec<(String, String)>,
    secret_keys: Vec<String>,
    replaces_existing: Option<bool>,
) -> Result<ActionProposal, ProposalError> {
    let profile_name = required(profile_name, "the environment name")?;
    if profile_name.chars().count() > MAX_ENV_NAME_CHARS || !valid_env_profile_name(&profile_name) {
        return Err(ProposalError::new(format!(
            "the environment name must match ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ \
             and be 1-{MAX_ENV_NAME_CHARS} characters"
        )));
    }

    let install = clean_list(install);
    // A saved profile's PUT validates an installation runtime, so it requires at least one
    // command — the same rule `environment_validation::validate_install(.., true)` applies.
    if install.is_empty() {
        return Err(ProposalError::new(
            "an environment profile needs at least one install command",
        ));
    }
    if install.len() > MAX_DRAFT_INSTALL_COMMANDS {
        return Err(ProposalError::new(format!(
            "a drafted environment may list at most {MAX_DRAFT_INSTALL_COMMANDS} install commands"
        )));
    }
    if let Some(command) = install
        .iter()
        .find(|c| c.chars().count() > MAX_DRAFT_INSTALL_COMMAND_CHARS)
    {
        return Err(ProposalError::new(format!(
            "install command {:?} exceeds {MAX_DRAFT_INSTALL_COMMAND_CHARS} characters",
            command.chars().take(60).collect::<String>()
        )));
    }

    if variables.len() > MAX_DRAFT_VARIABLES {
        return Err(ProposalError::new(format!(
            "a drafted environment may list at most {MAX_DRAFT_VARIABLES} variables"
        )));
    }
    let mut drafted_variables = Vec::with_capacity(variables.len());
    for (key, value) in variables {
        let key = key.trim().to_string();
        check_env_key(&key, "variable")?;
        if value.chars().count() > MAX_DRAFT_VARIABLE_VALUE_CHARS {
            return Err(ProposalError::new(format!(
                "the value for {key:?} exceeds {MAX_DRAFT_VARIABLE_VALUE_CHARS} characters"
            )));
        }
        drafted_variables.push(EnvVarDraft { key, value });
    }

    let secret_keys = clean_list(secret_keys);
    if secret_keys.len() > MAX_DRAFT_SECRET_KEYS {
        return Err(ProposalError::new(format!(
            "a drafted environment may declare at most {MAX_DRAFT_SECRET_KEYS} secret names"
        )));
    }
    for key in &secret_keys {
        check_env_key(key, "secret")?;
    }
    // A name used for both a plaintext variable and a secret would be ambiguous at
    // confirm-time (the endpoint stores them in separate maps and the session merges them),
    // so the collision is rejected while the model can still fix it.
    if let Some(collision) = secret_keys
        .iter()
        .find(|key| drafted_variables.iter().any(|entry| &&entry.key == key))
    {
        return Err(ProposalError::new(format!(
            "{collision:?} is declared as BOTH a variable and a secret; pick one"
        )));
    }

    let verb = match replaces_existing {
        Some(true) => "Replace",
        _ => "Create",
    };
    let summary = format!(
        "{verb} the environment profile `{profile_name}` ({} install command{}, {} variable{}, {} secret{})",
        install.len(),
        if install.len() == 1 { "" } else { "s" },
        drafted_variables.len(),
        if drafted_variables.len() == 1 { "" } else { "s" },
        secret_keys.len(),
        if secret_keys.len() == 1 { "" } else { "s" },
    );
    Ok(ActionProposal::SaveEnvironmentProfile {
        target: ActionTarget {
            method: "PUT".to_string(),
            path: format!("/api/v1/users/me/environment-profiles/{profile_name}"),
        },
        profile_name,
        replaces_existing,
        install,
        variables: drafted_variables,
        secret_keys,
        summary,
    })
}

/// Build a delete-environment-profile proposal.
pub fn propose_delete_environment_profile(
    profile_name: &str,
) -> Result<ActionProposal, ProposalError> {
    let profile_name = required(profile_name, "the environment name")?;
    if profile_name.chars().count() > MAX_ENV_NAME_CHARS || !valid_env_profile_name(&profile_name) {
        return Err(ProposalError::new(format!(
            "the environment name must match ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ \
             and be 1-{MAX_ENV_NAME_CHARS} characters"
        )));
    }
    let summary = format!("Delete the environment profile `{profile_name}`");
    Ok(ActionProposal::DeleteEnvironmentProfile {
        target: ActionTarget {
            method: "DELETE".to_string(),
            path: format!("/api/v1/users/me/environment-profiles/{profile_name}"),
        },
        profile_name,
        summary,
    })
}

/// Build an uninstall-App proposal.
///
/// The reason is required for the same reason the stop-session reason is: uninstalling
/// takes the App off every repository of that account at once, so the user must see the
/// rationale before confirming.
pub fn propose_uninstall_app(owner: &str, reason: &str) -> Result<ActionProposal, ProposalError> {
    let owner = required(owner, "owner")?;
    let reason = required(reason, "the uninstall reason")?;
    if reason.chars().count() > MAX_UNINSTALL_REASON_CHARS {
        return Err(ProposalError::new(format!(
            "the uninstall reason must be at most {MAX_UNINSTALL_REASON_CHARS} characters"
        )));
    }
    let summary = format!("Uninstall the fkst GitHub App from {owner}");
    Ok(ActionProposal::UninstallApp {
        target: ActionTarget {
            method: "DELETE".to_string(),
            path: format!("/api/v1/installations/{owner}"),
        },
        owner,
        reason,
        summary,
    })
}

#[cfg(test)]
#[path = "resources_tests.rs"]
mod tests;
