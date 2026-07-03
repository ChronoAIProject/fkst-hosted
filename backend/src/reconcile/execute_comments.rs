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

pub(super) fn config_rejected_comment() -> String {
    "⚠️ **Config changes are not allowed after a session trigger exists.** Your edit \
     has been ignored and will not be accepted. To change packages, environment, the \
     log-streaming flag, or any other setting, **close this issue and open a new \
     session.**"
        .to_string()
}

pub(super) fn flag_invalid_comment(detail: &str) -> String {
    format!(
        "⚠️ fkst couldn't parse this trigger issue: {detail}\n\nExpected the \
         `fkst-substrate-trigger` body with `### Session Name`, `### Packages` (one \
         `owner/repo@ref:path` per line), `### Work Label`, and an optional `### Environment`. \
         Fix the issue body and the reconciler will retry."
    )
}
