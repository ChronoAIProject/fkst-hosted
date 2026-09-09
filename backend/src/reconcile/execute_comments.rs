//! Pure feedback-comment bodies the executor posts back to a trigger issue, split out
//! of [`crate::reconcile::execute`] to keep it under the file-size budget. Every body
//! is PUBLIC metadata only — the parser's message, the offending refs, a named
//! environment — never the minted token or any environment secret VALUE.

pub(super) fn env_not_ready_comment(name: &str) -> String {
    format!(
        "⚠️ fkst couldn't start this session: environment `{name}` was not found in your account \
         (or isn't ready). Create it first with `PUT /api/v1/users/me/environments/{name}`, then \
         re-trigger. Omit the `### Environment` section to run with no environment."
    )
}

pub(super) fn env_verify_failed_comment(name: &str) -> String {
    format!(
        "⚠️ fkst couldn't verify environment `{name}` right now (a transient error reading your \
         environments). Please re-trigger in a moment."
    )
}

pub(super) fn invalid_refs_comment(failures: &[(String, String)]) -> String {
    let mut body = String::from(
        "⚠️ fkst couldn't start this session: one or more `### Packages` refs are not reachable \
         on public GitHub.\n\n",
    );
    for (r, reason) in failures {
        body.push_str(&format!("- `{r}` — {reason}\n"));
    }
    body.push_str(
        "\nEach ref must be `owner/repo@ref:path/to/package` in a PUBLIC repo with an `fkst.toml` \
         at that path. Fix the refs and re-trigger.",
    );
    body
}

/// Feedback for a package tree that resolves fine but ships no session-isolation
/// rule. Deliberately NOT `invalid_refs_comment`: that one tells the author their
/// ref is unreachable and to check the repo is public with an `fkst.toml`, and both
/// are false here — the ref resolved, it just cannot honour issue routing. Sending
/// the author after a reachability problem that does not exist wastes their time.
pub(super) fn missing_isolation_comment(failures: &[(String, String)]) -> String {
    let mut body = String::from(
        "⚠️ fkst couldn't start this session: its package tree does not implement \
         session isolation.\n\n\
         A session that cannot route work by assignee would act on issues belonging to \
         OTHER people's sessions — redriving their in-progress work, and even \
         implementing and merging their issues onto this session's branch. So the \
         session is refused rather than started.\n\n",
    );
    for (tree, reason) in failures {
        body.push_str(&format!("- `{tree}` — {reason}\n"));
    }
    body.push_str(
        "\nPoint the affected `### Packages` / `### Manifest` refs at a package tree \
         that carries the rule, then re-trigger. This deployment's own catalog \
         (`ChronoAIProject/fkst-hosted@packages`) carries it.",
    );
    body
}

pub(super) fn config_rejected_comment() -> String {
    "⚠️ **Config changes are not allowed after a session trigger exists.** Your edit \
     has been ignored and will not be accepted. To change packages, environment, the \
     log-streaming flag, or any other setting, **close this issue and open a new \
     session.**"
        .to_string()
}

pub(super) fn flag_invalid_comment(detail: &str) -> String {
    // Covers both a parse failure and a work-label collision demotion. The lead is
    // phrased so either reason reads correctly, while the guidance describes the
    // current optional sections and creator-scoped collision rule.
    format!(
        "⚠️ fkst can't run this trigger issue as a session: {detail}\n\nA valid trigger needs \
         `### Session Name` plus at least one package source (`### Packages` lines or a \
         `### Manifest` reference); `### Work Label`, `### Source Branch`, `### Target Branch`, \
         and `### Environment` are optional. Its effective work labels must not overlap another \
         of **your** active sessions on this repo. Fix the issue and the reconciler will retry."
    )
}

pub(super) fn trigger_unauthorized_comment(detail: &str) -> String {
    format!(
        "🚫 **This trigger issue was not accepted: {detail}.**\n\nOnly an fkst deployment \
         administrator or a user with **admin or maintain** permission on this repository can \
         start an fkst session. The issue body has not been read. If the required role is \
         granted (or a deployment admin adds the creator to `FKST_GLOBAL_ADMINS`), the \
         reconciler will pick this trigger up automatically."
    )
}

#[cfg(test)]
mod tests {
    use super::{flag_invalid_comment, missing_isolation_comment};

    #[test]
    fn invalid_trigger_guidance_describes_the_current_contract() {
        assert_eq!(
            flag_invalid_comment("the effective labels overlap"),
            "⚠️ fkst can't run this trigger issue as a session: the effective labels overlap\n\n\
             A valid trigger needs `### Session Name` plus at least one package source (`### Packages` \
             lines or a `### Manifest` reference); `### Work Label`, `### Source Branch`, `### Target \
             Branch`, and `### Environment` are optional. Its effective work labels must not overlap \
             another of **your** active sessions on this repo. Fix the issue and the reconciler will retry."
        );
    }

    /// The refusal must describe the ACTUAL failure. Reusing `invalid_refs_comment`
    /// told the author their ref was unreachable and to check the repo is public with
    /// an `fkst.toml` -- both false here, and both send them chasing a problem that
    /// does not exist.
    #[test]
    fn missing_isolation_comment_does_not_claim_unreachability() {
        let body = missing_isolation_comment(&[(
            "o/old@legacy".to_string(),
            "ships claims.lua WITHOUT issue_owned_by_session".to_string(),
        )]);
        assert!(
            body.contains("does not implement session isolation"),
            "{body}"
        );
        assert!(body.contains("o/old@legacy"), "{body}");
        assert!(body.contains("issue_owned_by_session"), "{body}");
        assert!(!body.contains("not reachable"), "{body}");
        assert!(!body.contains("PUBLIC repo"), "{body}");
    }

    #[test]
    fn missing_isolation_comment_names_every_offending_tree_and_the_remedy() {
        let body = missing_isolation_comment(&[
            ("o/a@x".to_string(), "reason a".to_string()),
            ("o/b@y".to_string(), "reason b".to_string()),
        ]);
        assert!(body.contains("o/a@x"), "{body}");
        assert!(body.contains("o/b@y"), "{body}");
        assert!(
            body.contains("ChronoAIProject/fkst-hosted@packages"),
            "{body}"
        );
    }
}
