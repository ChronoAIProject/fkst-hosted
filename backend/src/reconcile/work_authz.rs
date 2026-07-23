//! Pure, always-on work-issue authority.
//!
//! A routed issue may raise work only for its session creator, a deployment-wide
//! global administrator, or a principal explicitly listed under `### Session
//! Collaborators`. The configured FKST GitHub App is separately trusted as a
//! system principal so workflow materialization can create routed child issues.
//! Repository administrator status is intentionally not a human authority tier.

use crate::access_policy::{entry_matches, AccessPolicy};
use crate::reconcile::creator::is_expected_bot_login;
use crate::reconcile::desired::SessionRegistration;

/// Whether the verified work-issue author may raise work for `reg`.
pub fn is_work_author_allowed(
    reg: &SessionRegistration,
    global_admins: &AccessPolicy,
    author_id: i64,
    author_login: &str,
) -> bool {
    // Human-authored registrations carry the creator's immutable id. App-authored
    // seeded registrations use the sole assignee's login because issue metadata
    // does not expose assignee ids.
    let creator_matches = match reg.creator_id {
        Some(creator_id) => author_id == creator_id,
        None => {
            !reg.creator_login.trim().is_empty()
                && author_login.eq_ignore_ascii_case(&reg.creator_login)
        }
    };
    if creator_matches || global_admins.is_global_admin(author_id, author_login) {
        return true;
    }

    let author_id = author_id.to_string();
    reg.collaborators
        .iter()
        .any(|entry| entry_matches(entry, &author_id, author_login))
}

/// Apply the human authority tiers plus the configured FKST App system principal.
///
/// Routing remains a separate mandatory predicate at every caller, so trusting the
/// App author never permits an unassigned, multiply assigned, or foreign-routed
/// issue to wake or receive acknowledgment from a session.
pub fn is_work_author_allowed_with_bot(
    reg: &SessionRegistration,
    global_admins: &AccessPolicy,
    author_id: i64,
    author_login: &str,
    github_bot_login: Option<&str>,
) -> bool {
    is_expected_bot_login(author_login, github_bot_login)
        || is_work_author_allowed(reg, global_admins, author_id, author_login)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::trigger_parse::PackageRef;
    use crate::models::RepoRef;
    use crate::reconcile::desired::SessionDef;

    fn reg(
        creator_id: Option<i64>,
        creator_login: &str,
        collaborators: &[&str],
    ) -> SessionRegistration {
        SessionRegistration {
            installation_id: 42,
            repo: RepoRef {
                owner: "acme".to_string(),
                name: "site".to_string(),
            },
            trigger_issue: 7,
            trigger_author_id: creator_id.unwrap_or(9000),
            trigger_author_login: "fkst-app[bot]".to_string(),
            creator_login: creator_login.to_string(),
            creator_id,
            def: SessionDef {
                name: "site".to_string(),
                packages: Vec::<PackageRef>::new(),
                manifest_refs: Vec::<PackageRef>::new(),
                work_label: Some("fkst-run".to_string()),
                environment: None,
                output_lang: None,
                engine_config: std::collections::BTreeMap::new(),
                source_branch: None,
                target_branch: None,
            },
            effective_packages: Vec::new(),
            session_id: "sess-abc".to_string(),
            config_hash: "hash123".to_string(),
            auto_merge: false,
            log_access: vec![],
            collaborators: collaborators
                .iter()
                .map(|value| value.to_string())
                .collect(),
        }
    }

    fn access(global_admins: &str) -> AccessPolicy {
        AccessPolicy::from_vars(&[("FKST_GLOBAL_ADMINS".to_string(), global_admins.to_string())])
            .expect("access policy")
    }

    #[test]
    fn creator_is_allowed_by_immutable_id() {
        let reg = reg(Some(583231), "old-login", &[]);
        assert!(is_work_author_allowed(
            &reg,
            &access(""),
            583231,
            "renamed-since"
        ));
        assert!(!is_work_author_allowed(&reg, &access(""), 99, "old-login"));
    }

    #[test]
    fn assignee_derived_creator_is_allowed_by_login_case_insensitively() {
        let reg = reg(None, "Seed-Owner", &[]);
        assert!(is_work_author_allowed(&reg, &access(""), 999, "seed-owner"));
        assert!(!is_work_author_allowed(
            &reg,
            &access(""),
            9000,
            "fkst-app[bot]"
        ));
    }

    #[test]
    fn global_admin_login_entry_is_allowed() {
        let reg = reg(Some(7), "alice", &[]);
        assert!(is_work_author_allowed(
            &reg,
            &access("@Deploy-Admin"),
            500,
            "deploy-admin"
        ));
    }

    #[test]
    fn numeric_global_admin_entry_is_allowed_by_id_only() {
        let reg = reg(Some(7), "alice", &[]);
        let policy = access("4242");
        assert!(is_work_author_allowed(&reg, &policy, 4242, "renamed"));
        assert!(!is_work_author_allowed(&reg, &policy, 999, "4242"));
    }

    #[test]
    fn collaborator_tier_accepts_login_or_numeric_id() {
        let reg = reg(Some(7), "alice", &["Bob", "31337"]);
        assert!(is_work_author_allowed(&reg, &access(""), 100, "bob"));
        assert!(is_work_author_allowed(&reg, &access(""), 31337, "renamed"));
        assert!(!is_work_author_allowed(&reg, &access(""), 100, "31337"));
    }

    #[test]
    fn repo_admin_without_an_explicit_tier_is_rejected() {
        // Repository role is deliberately absent from the predicate. A caller who
        // is only a repo admin is indistinguishable from any other stranger.
        let reg = reg(Some(7), "alice", &["bob"]);
        assert!(!is_work_author_allowed(
            &reg,
            &access("deploy-admin"),
            500,
            "repo-owner"
        ));
    }

    #[test]
    fn blank_collaborator_entries_never_grant() {
        let reg = reg(Some(7), "alice", &["@", "  "]);
        assert!(!is_work_author_allowed(&reg, &access(""), 999, ""));
    }

    #[test]
    fn only_the_configured_app_identity_is_allowed_as_a_system_principal() {
        let reg = reg(Some(7), "alice", &[]);
        for login in ["fkst-app[bot]", "app/FKST-App", "fkst-app"] {
            assert!(is_work_author_allowed_with_bot(
                &reg,
                &access(""),
                9000,
                login,
                Some("fkst-app[bot]")
            ));
        }
        assert!(!is_work_author_allowed_with_bot(
            &reg,
            &access(""),
            9000,
            "fkst-app[bot]",
            None
        ));
        assert!(!is_work_author_allowed_with_bot(
            &reg,
            &access(""),
            9000,
            "other-app[bot]",
            Some("fkst-app[bot]")
        ));
    }
}
