//! The one-time session-registration announcement comment (issue #359 UX follow-up).
//!
//! A `fkst-substrate-trigger` issue is otherwise silent after it is opened, so the
//! author has no signal that a session was registered or how to drive it. When a
//! session FIRST registers (a valid trigger), the reconciler posts this friendly
//! metadata comment and latches [`crate::reconcile::SUBSTRATE_ANNOUNCED_LABEL`] so
//! it is posted exactly once (the label is durable, surviving CP restarts).
//!
//! This module is the PURE renderer + marker parser — no I/O, exhaustively
//! unit-testable. The effectful post + latch lives in [`crate::reconcile::execute`],
//! mirroring the invalid-flag path. The body carries only PUBLIC metadata (session
//! name, work label, rendered package refs, environment name, auto-merge state) —
//! never the minted token or any environment secret VALUE.
//!
//! CONFIG IMMUTABILITY: the announcement ALSO carries a hidden HTML-comment marker
//! recording the registration's original `full_config_hash`. Because the announce is
//! posted exactly once, that marker durably records the config the session was
//! registered with — with zero new storage. A later reconcile reads it back (via
//! `list_issue_comments` + [`parse_config_hash_marker`]) to detect, and reject, any
//! edit to an already-triggered issue.
//!
//! ONBOARDING (issue #3379): beyond the metadata block, the comment walks the
//! author through driving the session — how to queue a work issue (label + the
//! session creator as the SOLE assignee, the routing invariant), who may author
//! work, what feedback to expect (`fkst-picked-up`, one PR per issue into the
//! target branch, keep-alive/idle-down), and the fkst dashboard URL when
//! `FKST_FRONTEND_URL` is configured. The comment is NOT parsed by the trigger
//! parser (only issue BODIES are), so its formatting is unconstrained.

use std::sync::OnceLock;

use regex::Regex;

/// Render the announcement comment body for a newly-registered session.
///
/// `detected_work_labels` is the session's FULL effective work-label set (its explicit
/// `### Work Label` ∪ the labels its packages auto-declare) — the set that actually
/// wakes it. The comment lists EVERY label in it, so a label-less/auto-detect session
/// (no `### Work Label`, wake labels discovered from its packages) still gets concrete
/// labeling guidance. When the set is empty (shouldn't happen — a zero-label session is
/// rejected upstream), it falls back to the single explicit `work_label` (rendered only
/// when `Some`), matching the pre-multi-label behavior.
///
/// `packages` are the refs already rendered to `owner/repo@ref:path` (author order);
/// `environment` is the named environment or `None`; `auto_merge` reflects the
/// trigger's `### Auto-merge` opt-in. `log_url`, when `Some`, is the identity-gated
/// download endpoint for this session's redacted logs (built from the configured
/// public base URL + the session id); when `None` (no `FKST_PUBLIC_BASE_URL`) the log
/// line is omitted. The URL is STATIC and safe to post because the endpoint itself is
/// identity-gated (GitHub login or a Bearer token) — the link grants nothing on its
/// own. All values are public metadata safe to display; no token or secret appears.
///
/// `full_config_hash` is appended as a hidden marker (see [`config_hash_marker`]) so
/// the original config is durably latched on the issue for the immutability check.
// Each parameter is a distinct piece of PUBLIC announcement metadata rendered into the
// comment; they are not a cohesive struct worth introducing for a single call site.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn announce_session_comment_with_defaults(
    session_name: &str,
    work_label: Option<&str>,
    detected_work_labels: &[String],
    packages: &[String],
    environment: Option<&str>,
    auto_merge: bool,
    log_url: Option<&str>,
    full_config_hash: &str,
) -> String {
    announce_session_comment(
        session_name,
        work_label,
        detected_work_labels,
        packages,
        environment,
        None,
        crate::reconcile::branches::DEFAULT_TARGET_BRANCH,
        auto_merge,
        "the-creator",
        None,
        log_url,
        full_config_hash,
    )
}

/// Branch-aware announcement renderer used by the reconciler. `creator_login` is
/// the session's effective creator — the login every work issue must carry as its
/// SOLE assignee to route here; `frontend_url` (the configured
/// `FKST_FRONTEND_URL`) renders the dashboard block when `Some`.
#[allow(clippy::too_many_arguments)]
pub fn announce_session_comment(
    session_name: &str,
    work_label: Option<&str>,
    detected_work_labels: &[String],
    packages: &[String],
    environment: Option<&str>,
    source_branch: Option<&str>,
    target_branch: &str,
    auto_merge: bool,
    creator_login: &str,
    frontend_url: Option<&str>,
    log_url: Option<&str>,
    full_config_hash: &str,
) -> String {
    let mut body = format!("🟢 **fkst session `{session_name}` registered.**\n\n");

    // List the session's FULL effective work-label set (explicit `### Work Label` ∪ its
    // packages' auto-declared labels) — the set that actually wakes it. This surfaces the
    // auto-DISCOVERED labels of a label-less session, which the old single-`work_label`
    // rendering omitted entirely (leaving an auto-detect session with no labeling
    // guidance). Trimmed + deduped defensively, first-occurrence order preserved.
    let effective: Vec<&str> = detected_work_labels.iter().fold(Vec::new(), |mut acc, l| {
        let trimmed = l.trim();
        if !trimmed.is_empty() && !acc.contains(&trimmed) {
            acc.push(trimmed);
        }
        acc
    });
    // The inline backticked label list, reused by the metadata block and the
    // queue-work steps below. Empty-set fallback: the single explicit label.
    let labels_inline = if !effective.is_empty() {
        Some(
            effective
                .iter()
                .map(|l| format!("`{l}`"))
                .collect::<Vec<_>>()
                .join(", "),
        )
    } else {
        work_label.map(|l| format!("`{l}`"))
    };
    if !effective.is_empty() {
        let rendered = labels_inline.as_deref().unwrap_or_default();
        body.push_str(&format!(
            "**Work label(s):** {rendered} — open an issue with any of these labels in this \
             repo to queue work for this session.\n\n"
        ));
    } else if let Some(work_label) = work_label {
        // Fallback for the (upstream-rejected) zero-label case: the pre-multi-label
        // single-label rendering. A label-less session with no discovered labels advertises
        // nothing, exactly as before.
        body.push_str(&format!(
            "**Work label:** `{work_label}` — open issues with this label in this repo to \
             queue work for this session.\n\n"
        ));
    }

    body.push_str(&format!("**Packages:** {}\n", packages.len()));
    for r in packages {
        body.push_str(&format!("- `{r}`\n"));
    }
    body.push('\n');

    match environment {
        Some(name) => body.push_str(&format!("**Environment:** `{name}`\n\n")),
        None => body.push_str(
            "**Environment:** default — no named environment profile, no extra \
             configuration or software installations.\n\n",
        ),
    }

    body.push_str(&format!(
        "**Source branch:** `{}`\n\n",
        source_branch.unwrap_or("(repo default)")
    ));
    body.push_str(&format!(
        "**Target branch:** `{target_branch}` — all of this session's pull requests merge here.\n\n"
    ));

    if auto_merge {
        body.push_str(
            "**Auto-merge:** `on` — the App bot's PRs will be auto-merged to the target \
             branch when mergeable.\n\n",
        );
    } else {
        body.push_str("**Auto-merge:** `off`\n\n");
    }

    // --- Onboarding walkthrough (issue #3379) --------------------------------
    // How to queue work: the three routing requirements, spelled out. The sole-
    // assignee rule is the one users most often miss — a work issue routes ONLY
    // when its single assignee equals this session's effective creator.
    body.push_str("---\n\n**📋 How to queue work**\n\n");
    body.push_str(
        "1. Open a **new issue** in this repository describing one task — what to \
         change and how to verify it.\n",
    );
    match &labels_inline {
        Some(labels) => body.push_str(&format!(
            "2. Add one of this session's work labels: {labels}.\n"
        )),
        None => body.push_str("2. Add one of this session's work labels.\n"),
    }
    body.push_str(&format!(
        "3. Assign the issue to **@{creator_login}** as its **only assignee** — work \
         routes by label plus that single assignee. A missing, different, or extra \
         assignee leaves the issue marked `fkst-unrouted` until corrected, after \
         which it is picked up automatically.\n\n"
    ));
    body.push_str(&format!(
        "Work issues may be authored by **@{creator_login}**, the logins under this \
         trigger's `### Session Collaborators`, or a deployment admin; issues from \
         anyone else stay unworked and are marked `fkst-unauthorized`.\n\n"
    ));

    // What to expect once a work issue routes.
    body.push_str("**🔁 What to expect**\n\n");
    body.push_str(
        "- The session claims each routed issue and labels it `fkst-picked-up` — \
         usually within seconds via webhook, otherwise on the next periodic sweep.\n",
    );
    body.push_str(&format!(
        "- Every open work issue is worked in parallel as its own pull request into \
         `{target_branch}`, with progress reported back on the issue.\n"
    ));
    if auto_merge {
        body.push_str("- Session pull requests auto-merge once they are mergeable.\n");
    } else {
        body.push_str(
            "- Auto-merge is off for this session — review and merge each session \
             pull request yourself.\n",
        );
    }
    body.push_str(
        "- Open work issues keep this session's pod running; merge or close them to \
         let the session idle down.\n\n",
    );

    // The dashboard block — rendered only when a frontend URL is configured.
    if let Some(url) = frontend_url {
        body.push_str(&format!(
            "**🖥️ Dashboard**\n\n{url} — sign in with GitHub to browse your \
             repositories and sessions, watch live session activity, review work \
             items and outcomes, download session logs, and manage named environment \
             profiles.\n\n"
        ));
    }

    // The identity-gated log-download link. Static + safe to post: the endpoint
    // authorizes every request (the session owner, the `### Log Access Allowlist` list, or an
    // admin), so the bare URL grants nothing. Omitted when no public base URL is set.
    if let Some(url) = log_url {
        body.push_str(&format!(
            "📥 **Logs:** {url} — authorized users only (session owner, listed users, or \
             admins). Open in a browser (GitHub login) or pass `Authorization: Bearer \
             <github-token>` for API/agent access.\n\n"
        ));
    }

    body.push_str(
        "🔒 **Config frozen.** This session's configuration is now immutable — editing \
         this issue's body is rejected (`fkst-config-rejected`); to change anything, \
         close this issue and open a new trigger.\n\n",
    );
    body.push_str("Close this issue to retire the session.");
    // The hidden immutability marker is appended LAST so it trails the visible body.
    body.push_str(&config_hash_marker(full_config_hash));
    body
}

/// Render the hidden HTML-comment marker that latches a registration's original
/// `full_config_hash` in the announcement comment. GitHub does not render HTML
/// comments, so it is invisible in the issue thread. The leading blank line separates
/// it from the visible body; [`parse_config_hash_marker`] recovers the hash back out.
fn config_hash_marker(full_config_hash: &str) -> String {
    format!("\n\n<!-- fkst-config-hash: {full_config_hash} -->")
}

/// Recover the FIRST latched `full_config_hash` from a trigger issue's comment bodies,
/// scanning in author order for the hidden `<!-- fkst-config-hash: <hex> -->` marker.
///
/// Returns `None` when no comment carries a well-formed marker — e.g. an issue whose
/// announcement predates this check, or a body with a malformed marker. The hash is a
/// lowercase SHA-256 hex digest, so the capture is anchored to `[0-9a-f]+`; a marker
/// with any other payload (uppercase, empty, non-hex) does not match and is ignored.
pub fn parse_config_hash_marker(comments: &[String]) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"<!-- fkst-config-hash: ([0-9a-f]+) -->")
            .expect("static config-hash marker regex")
    });
    comments
        .iter()
        .find_map(|body| re.captures(body).map(|c| c[1].to_string()))
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
        let body = announce_session_comment_with_defaults(
            "mysession",
            Some("fkst-run"),
            &["fkst-run".to_string()],
            &packages,
            Some("prod"),
            true,
            Some("https://fkst.example/api/v1/logs/sess-abc"),
            "abc123",
        );

        // Session name headline.
        assert!(body.contains("fkst session `mysession` registered."));
        // The effective work-label set, verbatim in backticks (a single-label set here).
        assert!(body.contains("**Work label(s):** `fkst-run`"));
        assert!(body.contains("open an issue with any of these labels"));
        // Package count + each rendered ref bulleted.
        assert!(body.contains("**Packages:** 2"));
        assert!(body.contains("- `ChronoAIProject/fkst-packages@dev:packages/github-devloop`"));
        assert!(body.contains("- `acme/pkgs@main:packages/proxy`"));
        // Environment name.
        assert!(body.contains("**Environment:** `prod`"));
        // Auto-merge ON carries the explanatory note.
        assert!(body.contains("**Auto-merge:** `on`"));
        assert!(body.contains("auto-merged to the target"));
        // The identity-gated log link is rendered with the passed URL.
        assert!(body.contains("📥 **Logs:** https://fkst.example/api/v1/logs/sess-abc"));
        assert!(body.contains("authorized users only"));
        assert!(body.contains("Authorization: Bearer"));
        // Closing guidance.
        assert!(body.contains("Close this issue to retire the session."));
        // The hidden immutability marker is present with the passed hash.
        assert!(body.contains("<!-- fkst-config-hash: abc123 -->"));
    }

    #[test]
    fn onboarding_names_the_creator_as_sole_assignee_and_lists_the_labels() {
        // The queue-work steps must spell out the routing invariant: the work
        // issue's ONLY assignee is the session creator, plus one of the labels.
        let body = announce_session_comment_with_defaults(
            "s",
            Some("fkst-x"),
            &["fkst-x".to_string(), "fkst-y".to_string()],
            &[],
            None,
            true,
            None,
            "h",
        );
        assert!(body.contains("**📋 How to queue work**"), "{body}");
        assert!(
            body.contains("Add one of this session's work labels: `fkst-x`, `fkst-y`."),
            "{body}"
        );
        // The defaults helper passes creator "the-creator".
        assert!(
            body.contains("Assign the issue to **@the-creator** as its **only assignee**"),
            "{body}"
        );
        assert!(body.contains("`fkst-unrouted`"), "{body}");
        // Authorized-author guidance names the creator, collaborators, and admins.
        assert!(body.contains("`### Session Collaborators`"), "{body}");
        assert!(body.contains("`fkst-unauthorized`"), "{body}");
        // Expectations: claim label, per-issue PRs, keep-alive/idle-down.
        assert!(body.contains("`fkst-picked-up`"), "{body}");
        assert!(body.contains("as its own pull request"), "{body}");
        assert!(body.contains("idle down"), "{body}");
        // Auto-merge ON renders the auto-merge expectation, not the review-yourself one.
        assert!(
            body.contains("Session pull requests auto-merge once they are mergeable."),
            "{body}"
        );
        assert!(!body.contains("review and merge each session"), "{body}");
    }

    #[test]
    fn dashboard_block_renders_only_when_a_frontend_url_is_configured() {
        let with_url = announce_session_comment(
            "s",
            Some("wl"),
            &["wl".to_string()],
            &[],
            None,
            None,
            "fkst-hosted-default",
            false,
            "octocat",
            Some("https://fkst.chrono-ai.fun"),
            None,
            "h",
        );
        assert!(with_url.contains("**🖥️ Dashboard**"), "{with_url}");
        assert!(
            with_url.contains("https://fkst.chrono-ai.fun"),
            "{with_url}"
        );
        assert!(with_url.contains("environment profiles"), "{with_url}");
        assert!(
            with_url.contains("**@octocat**"),
            "the real creator login is rendered: {with_url}"
        );

        // No frontend URL configured → the dashboard block is omitted entirely.
        let without = announce_session_comment_with_defaults(
            "s",
            Some("wl"),
            &["wl".to_string()],
            &[],
            None,
            false,
            None,
            "h",
        );
        assert!(!without.contains("Dashboard"), "{without}");
    }

    #[test]
    fn renders_zero_packages_no_environment_and_auto_merge_off() {
        let body = announce_session_comment_with_defaults(
            "solo",
            Some("run"),
            &["run".to_string()],
            &[],
            None,
            false,
            None,
            "deadbeef",
        );

        assert!(body.contains("**Packages:** 0"));
        // No bullets when there are no packages.
        assert!(!body.contains("\n- `"));
        // A profile-less session reads as the DEFAULT environment (issue #3379),
        // spelling out that nothing extra is configured or installed.
        assert!(body.contains(
            "**Environment:** default — no named environment profile, no extra \
             configuration or software installations."
        ));
        assert!(body.contains("**Auto-merge:** `off`"));
        // Auto-merge OFF renders the review-it-yourself expectation line.
        assert!(body.contains("review and merge each session"));
        // The auto-merge note only appears when it is ON.
        assert!(!body.contains("auto-merged to the default"));
        // The marker is always appended, regardless of the visible metadata.
        assert!(body.contains("<!-- fkst-config-hash: deadbeef -->"));
    }

    #[test]
    fn the_rendered_marker_round_trips_through_the_parser() {
        // The marker the renderer writes is exactly what the parser reads back — the
        // load-bearing contract for the immutability check.
        let body = announce_session_comment_with_defaults(
            "s",
            Some("wl"),
            &["wl".to_string()],
            &[],
            None,
            false,
            None,
            "0a1b2c3d",
        );
        assert_eq!(
            parse_config_hash_marker(&[body]).as_deref(),
            Some("0a1b2c3d"),
            "the parser recovers the exact hash the renderer latched"
        );
    }

    #[test]
    fn parse_returns_the_first_markers_hash() {
        // The announcement is the first comment; its marker wins even if a later
        // comment carries another marker (which never happens in practice).
        let comments = vec![
            "plain comment, no marker".to_string(),
            "announce\n\n<!-- fkst-config-hash: aaaa1111 -->".to_string(),
            "later\n\n<!-- fkst-config-hash: bbbb2222 -->".to_string(),
        ];
        assert_eq!(
            parse_config_hash_marker(&comments).as_deref(),
            Some("aaaa1111")
        );
    }

    #[test]
    fn parse_absent_marker_is_none() {
        let comments = vec![
            "just a normal comment".to_string(),
            "another one with no marker".to_string(),
        ];
        assert_eq!(parse_config_hash_marker(&comments), None);
        // The empty set is trivially markerless.
        assert_eq!(parse_config_hash_marker(&[]), None);
    }

    #[test]
    fn parse_malformed_marker_is_none() {
        // A marker whose payload is not lowercase hex (uppercase / empty / spaces)
        // does not match the anchored capture and is treated as absent.
        let comments = vec![
            "<!-- fkst-config-hash: NOTHEX -->".to_string(),
            "<!-- fkst-config-hash:  -->".to_string(),
            "<!-- fkst-config-hash -->".to_string(),
            "<!-- some-other-marker: abcd -->".to_string(),
        ];
        assert_eq!(parse_config_hash_marker(&comments), None);
    }

    #[test]
    fn omits_the_log_line_when_no_url_is_configured() {
        // No public base URL => `None` => the log line is absent entirely (no bare
        // "Logs:" label, no dangling link).
        let body = announce_session_comment_with_defaults(
            "solo",
            Some("run"),
            &["run".to_string()],
            &[],
            None,
            false,
            None,
            "cfghash",
        );
        assert!(
            !body.contains("Logs:"),
            "the log line must be omitted when no URL is configured: {body}"
        );
    }
}

#[cfg(test)]
mod work_label_set_tests {
    use super::{announce_session_comment, announce_session_comment_with_defaults};

    // Convenience: a `Vec<String>` from string literals for the detected-set arg.
    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn explicit_label_lists_that_single_label_with_guidance() {
        // An explicit `### Work Label` session: its detected set is exactly that one
        // label, so the announce lists it plus the labeling instruction.
        let body = announce_session_comment_with_defaults(
            "s",
            Some("fkst-x"),
            &labels(&["fkst-x"]),
            &[],
            None,
            false,
            None,
            "h",
        );
        assert!(body.contains("**Work label(s):** `fkst-x`"), "{body}");
        assert!(
            body.contains("open an issue with any of these labels in this repo"),
            "{body}"
        );
    }

    #[test]
    fn discovered_only_label_less_session_lists_the_discovered_labels() {
        // The headline I5 case: a label-less/auto-detect session (no `### Work Label`,
        // so `work_label` is None) whose wake labels are auto-discovered from its
        // packages. The old rendering omitted the block entirely — leaving no labeling
        // guidance; now the discovered labels + the instruction are shown.
        let body = announce_session_comment_with_defaults(
            "auto",
            None,
            &labels(&["pkg-alpha", "pkg-beta"]),
            &[],
            None,
            false,
            None,
            "h",
        );
        assert!(
            body.contains("**Work label(s):** `pkg-alpha`, `pkg-beta`"),
            "the label-less session must now list its DISCOVERED labels: {body}"
        );
        assert!(
            body.contains("open an issue with any of these labels in this repo"),
            "{body}"
        );
        // The rest of the announce still renders.
        assert!(body.contains("fkst session `auto` registered"), "{body}");
    }

    #[test]
    fn multi_label_session_lists_the_full_effective_set() {
        // Explicit `### Work Label` ∪ package-discovered: every label in the effective
        // set is listed, in the order the set carries.
        let body = announce_session_comment_with_defaults(
            "multi",
            Some("explicit"),
            &labels(&["explicit", "disc-one", "disc-two"]),
            &[],
            None,
            false,
            None,
            "h",
        );
        assert!(
            body.contains("**Work label(s):** `explicit`, `disc-one`, `disc-two`"),
            "{body}"
        );
    }

    #[test]
    fn blank_and_duplicate_labels_are_dropped_and_deduped() {
        // Defensive: blank tokens are dropped and duplicates collapsed, first-occurrence
        // order preserved — the rendered list never carries an empty `` `` `` entry.
        let body = announce_session_comment_with_defaults(
            "s",
            None,
            &labels(&["a", "", "  ", "a", "b"]),
            &[],
            None,
            false,
            None,
            "h",
        );
        assert!(body.contains("**Work label(s):** `a`, `b`"), "{body}");
        assert!(!body.contains("``"), "no empty backtick pair: {body}");
    }

    #[test]
    fn empty_detected_set_falls_back_to_the_single_explicit_label() {
        // A zero-label detected set shouldn't happen (rejected upstream), but if it does
        // the renderer falls back to the pre-multi-label single-`work_label` rendering.
        let body = announce_session_comment_with_defaults(
            "s",
            Some("only"),
            &[],
            &[],
            None,
            false,
            None,
            "h",
        );
        assert!(body.contains("**Work label:** `only`"), "{body}");
    }

    #[test]
    fn no_labels_at_all_omits_the_work_label_line() {
        // Empty detected set AND no explicit label: nothing to advertise — the block is
        // omitted, exactly as before. The rest of the announce still renders.
        let body =
            announce_session_comment_with_defaults("s", None, &[], &[], None, false, None, "h");
        assert!(!body.contains("**Work label"), "{body}");
        assert!(body.contains("fkst session `s` registered"), "{body}");
    }

    #[test]
    fn renders_explicit_source_and_resolved_target_branches() {
        let body = announce_session_comment(
            "s",
            None,
            &[],
            &[],
            None,
            Some("release/v1"),
            "feature-x",
            true,
            "octocat",
            None,
            None,
            "h",
        );
        assert!(body.contains("**Source branch:** `release/v1`"));
        assert!(body.contains("**Target branch:** `feature-x`"));
        assert!(body.contains("auto-merged to the target branch"));
        // The what-to-expect PR line names the resolved target branch too.
        assert!(body.contains("as its own pull request into `feature-x`"));
        assert!(body.ends_with("<!-- fkst-config-hash: h -->"));
    }
}
