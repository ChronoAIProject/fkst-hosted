//! The one-time session-registration announcement comment (issue #359 UX follow-up).
//!
//! A `fkst-substrate-trigger` issue is otherwise silent after it is opened, so the
//! author has no signal that a session was registered or how to drive it. When a
//! session FIRST registers (a valid trigger), the reconciler posts this friendly
//! metadata comment and latches [`crate::reconcile::SUBSTRATE_ANNOUNCED_LABEL`] so
//! it is posted exactly once (the label is durable, surviving CP restarts).
//!
//! This module is the PURE renderer — no I/O, exhaustively unit-testable. The
//! effectful post + latch lives in [`crate::reconcile::execute`], mirroring the
//! invalid-flag path. The body carries only PUBLIC metadata (session name, work
//! label, rendered package refs, environment name, auto-merge state) — never the
//! minted token or any environment secret VALUE.

/// Render the announcement comment body for a newly-registered session.
///
/// `packages` are the refs already rendered to `owner/repo@ref:path` (author order);
/// `environment` is the named environment or `None`; `auto_merge` reflects the
/// trigger's `### Auto-merge` opt-in. All values are public metadata safe to display.
pub fn announce_session_comment(
    session_name: &str,
    work_label: &str,
    packages: &[String],
    environment: Option<&str>,
    auto_merge: bool,
) -> String {
    let mut body = format!("🟢 **fkst session `{session_name}` registered.**\n\n");

    body.push_str(&format!(
        "**Work label:** `{work_label}` — open issues with this label in this repo to \
         queue work for this session.\n\n"
    ));

    body.push_str(&format!("**Packages:** {}\n", packages.len()));
    for r in packages {
        body.push_str(&format!("- `{r}`\n"));
    }
    body.push('\n');

    match environment {
        Some(name) => body.push_str(&format!("**Environment:** `{name}`\n\n")),
        None => body.push_str("**Environment:** none\n\n"),
    }

    if auto_merge {
        body.push_str(
            "**Auto-merge:** `on` — the App bot's PRs will be auto-merged to the default \
             branch when mergeable.\n\n",
        );
    } else {
        body.push_str("**Auto-merge:** `off`\n\n");
    }

    body.push_str("Close this issue to retire the session.");
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_all_metadata_with_packages_and_auto_merge_on() {
        let packages = vec![
            "ChronoAIProject/fkst-packages@dev:packages/github-devloop".to_string(),
            "acme/pkgs@main:packages/proxy".to_string(),
        ];
        let body = announce_session_comment("mysession", "fkst-run", &packages, Some("prod"), true);

        // Session name headline.
        assert!(body.contains("fkst session `mysession` registered."));
        // Work label, verbatim in backticks.
        assert!(body.contains("**Work label:** `fkst-run`"));
        // Package count + each rendered ref bulleted.
        assert!(body.contains("**Packages:** 2"));
        assert!(body.contains("- `ChronoAIProject/fkst-packages@dev:packages/github-devloop`"));
        assert!(body.contains("- `acme/pkgs@main:packages/proxy`"));
        // Environment name.
        assert!(body.contains("**Environment:** `prod`"));
        // Auto-merge ON carries the explanatory note.
        assert!(body.contains("**Auto-merge:** `on`"));
        assert!(body.contains("auto-merged to the default"));
        // Closing guidance.
        assert!(body.contains("Close this issue to retire the session."));
    }

    #[test]
    fn renders_zero_packages_no_environment_and_auto_merge_off() {
        let body = announce_session_comment("solo", "run", &[], None, false);

        assert!(body.contains("**Packages:** 0"));
        // No bullets when there are no packages.
        assert!(!body.contains("\n- `"));
        assert!(body.contains("**Environment:** none"));
        assert!(body.contains("**Auto-merge:** `off`"));
        // The auto-merge note only appears when it is ON.
        assert!(!body.contains("auto-merged to the default"));
    }
}
