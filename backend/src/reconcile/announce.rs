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

use std::sync::OnceLock;

use regex::Regex;

/// Render the announcement comment body for a newly-registered session.
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
pub fn announce_session_comment(
    session_name: &str,
    work_label: Option<&str>,
    packages: &[String],
    environment: Option<&str>,
    auto_merge: bool,
    log_url: Option<&str>,
    full_config_hash: &str,
) -> String {
    let mut body = format!("🟢 **fkst session `{session_name}` registered.**\n\n");

    // A label-less session's work is auto-discovered from its packages' declared
    // labels; there is no single trigger-side label to advertise here.
    if let Some(work_label) = work_label {
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
        let body = announce_session_comment(
            "mysession",
            Some("fkst-run"),
            &packages,
            Some("prod"),
            true,
            Some("https://fkst.example/api/v1/logs/sess-abc"),
            "abc123",
        );

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
    fn renders_zero_packages_no_environment_and_auto_merge_off() {
        let body =
            announce_session_comment("solo", Some("run"), &[], None, false, None, "deadbeef");

        assert!(body.contains("**Packages:** 0"));
        // No bullets when there are no packages.
        assert!(!body.contains("\n- `"));
        assert!(body.contains("**Environment:** none"));
        assert!(body.contains("**Auto-merge:** `off`"));
        // The auto-merge note only appears when it is ON.
        assert!(!body.contains("auto-merged to the default"));
        // The marker is always appended, regardless of the visible metadata.
        assert!(body.contains("<!-- fkst-config-hash: deadbeef -->"));
    }

    #[test]
    fn the_rendered_marker_round_trips_through_the_parser() {
        // The marker the renderer writes is exactly what the parser reads back — the
        // load-bearing contract for the immutability check.
        let body = announce_session_comment("s", Some("wl"), &[], None, false, None, "0a1b2c3d");
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
        let body = announce_session_comment("solo", Some("run"), &[], None, false, None, "cfghash");
        assert!(
            !body.contains("Logs:"),
            "the log line must be omitted when no URL is configured: {body}"
        );
    }
}

#[cfg(test)]
mod optional_work_label_tests {
    use super::announce_session_comment;

    #[test]
    fn present_label_shows_the_work_label_line() {
        let body = announce_session_comment("s", Some("fkst-x"), &[], None, false, None, "h");
        assert!(body.contains("**Work label:** `fkst-x`"), "{body}");
    }

    #[test]
    fn absent_label_omits_the_work_label_line() {
        let body = announce_session_comment("s", None, &[], None, false, None, "h");
        assert!(!body.contains("**Work label:**"), "{body}");
        // The rest of the announce still renders.
        assert!(body.contains("fkst session `s` registered"), "{body}");
    }
}
