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
pub fn announce_session_comment(
    session_name: &str,
    work_label: Option<&str>,
    detected_work_labels: &[String],
    packages: &[String],
    environment: Option<&str>,
    auto_merge: bool,
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
    if !effective.is_empty() {
        let rendered = effective
            .iter()
            .map(|l| format!("`{l}`"))
            .collect::<Vec<_>>()
            .join(", ");
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
        let body = announce_session_comment(
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
        let body = announce_session_comment(
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
        let body = announce_session_comment(
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
    use super::announce_session_comment;

    // Convenience: a `Vec<String>` from string literals for the detected-set arg.
    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn explicit_label_lists_that_single_label_with_guidance() {
        // An explicit `### Work Label` session: its detected set is exactly that one
        // label, so the announce lists it plus the labeling instruction.
        let body = announce_session_comment(
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
        let body = announce_session_comment(
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
        let body = announce_session_comment(
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
        let body = announce_session_comment(
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
        let body = announce_session_comment("s", Some("only"), &[], &[], None, false, None, "h");
        assert!(body.contains("**Work label:** `only`"), "{body}");
    }

    #[test]
    fn no_labels_at_all_omits_the_work_label_line() {
        // Empty detected set AND no explicit label: nothing to advertise — the block is
        // omitted, exactly as before. The rest of the announce still renders.
        let body = announce_session_comment("s", None, &[], &[], None, false, None, "h");
        assert!(!body.contains("**Work label"), "{body}");
        assert!(body.contains("fkst session `s` registered"), "{body}");
    }
}
