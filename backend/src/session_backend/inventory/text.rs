//! Bounded, secret-safe projection of a backend's operational reason/message.
//!
//! A Kubernetes container `state.waiting.message` or an OpenSandbox
//! `status.message` is genuinely useful to an operator ("ImagePullBackOff",
//! "failed to pull image ..."), and just as genuinely a leak surface: image-pull
//! failures quote registry URLs, mount failures quote paths, and a backend under
//! stress echoes whatever it was handed. So every such string crosses ONE gate:
//!
//! ```text
//! raw  -> control-normalize -> strip URI userinfo/query -> central redactor
//!      -> collapse whitespace -> byte-bound (char-safe, marked)
//! ```
//!
//! Order matters. Control characters are flattened first so whitespace tokenizing
//! is reliable; URI material is stripped before redaction so a credential that the
//! entropy layer would miss (a short password in `user:pw@host`) is already gone;
//! redaction runs before truncation so a cut can never expose a secret prefix that
//! the mask would have covered.
//!
//! What NEVER enters here: container env, image-pull credentials, command or log
//! output, and serialized Pod/Sandbox JSON. The adapters read only the narrow
//! reason/message fields, and this module is the second line of defence.

use std::sync::OnceLock;

use crate::session_pod::log_stream::redact::Redactor;

/// Byte ceiling for a status reason. Backend reasons are short closed-ish tokens
/// (`CrashLoopBackOff`, `ImagePullBackOff`); anything longer is not a reason.
pub const MAX_STATUS_REASON_BYTES: usize = 128;

/// Byte ceiling for a status message — enough for a real diagnostic sentence,
/// far short of anything that could carry a log tail.
pub const MAX_STATUS_MESSAGE_BYTES: usize = 512;

/// Byte ceiling for the preserved backend-native state string. A state is a short
/// enum spelling; the bound exists so a hostile/garbage value cannot be echoed at
/// length.
pub const MAX_RAW_STATUS_BYTES: usize = 64;

/// Appended when truncation actually happened, so a clipped message never reads as
/// a complete one. Counted inside the byte budget.
const TRUNCATION_MARKER: &str = "…";

/// The shared redactor. It carries no known-secret literals (the inventory has no
/// session credentials in hand), so only the pattern-denylist and entropy layers
/// fire — which is exactly what a status message needs. Built once because the
/// automaton/regex construction is the expensive part.
fn redactor() -> &'static Redactor {
    static REDACTOR: OnceLock<Redactor> = OnceLock::new();
    REDACTOR.get_or_init(|| Redactor::new(&[]))
}

/// Project a raw backend string into a bounded, redacted operational summary.
///
/// `None` when the input is absent-equivalent (empty, whitespace-only, or reduced
/// to nothing by sanitization) — an empty string in the DTO would imply the
/// backend said something when it did not.
pub fn bounded_operational_text(raw: Option<&str>, max_bytes: usize) -> Option<String> {
    let raw = raw?;
    if raw.trim().is_empty() {
        return None;
    }
    let normalized = normalize_control(raw);
    let stripped = strip_uri_material(&normalized);
    let redacted = redactor().redact_line(&stripped);
    let collapsed = collapse_whitespace(&redacted);
    if collapsed.is_empty() {
        return None;
    }
    Some(truncate_bytes(&collapsed, max_bytes))
}

/// Replace every control character (newlines and tabs included) with a space.
///
/// Newlines are the important one: a multi-line backend message rendered into a
/// log line or a UI cell could otherwise forge a second record.
fn normalize_control(input: &str) -> String {
    input
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Strip credential/query material from every URI-shaped token.
///
/// Two shapes are handled: a full `scheme://[userinfo@]host[:port][/path][?query]`
/// (userinfo dropped, query/fragment dropped, host+path kept because they are the
/// operationally useful part), and a bare `user:password@host` (the credential
/// half replaced wholesale). A plain `user@host` email is left alone — it has no
/// password half and mangling it would lose real diagnostic detail.
fn strip_uri_material(input: &str) -> String {
    input
        .split(' ')
        .map(strip_token)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The credential placeholder left where userinfo was removed.
const CREDENTIAL_MASK: &str = "«REDACTED:url-credential»";

fn strip_token(token: &str) -> String {
    if let Some(scheme_end) = token.find("://") {
        let scheme = &token[..scheme_end];
        let after = &token[scheme_end + 3..];
        let authority_end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let authority = &after[..authority_end];
        let rest = &after[authority_end..];
        // Everything from the first query/fragment separator onwards is dropped:
        // signed URLs and callback URLs carry their secret material there.
        let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
        let path = &rest[..path_end];
        let host = match authority.rfind('@') {
            Some(at) => &authority[at + 1..],
            None => authority,
        };
        return format!("{scheme}://{host}{path}");
    }
    match token.rfind('@') {
        // `user:password@host` — a credential only when the userinfo half actually
        // has a password separator, which is what distinguishes it from an email.
        Some(at) if token[..at].contains(':') => {
            format!("{CREDENTIAL_MASK}@{}", &token[at + 1..])
        }
        _ => token.to_string(),
    }
}

/// Collapse whitespace runs to single spaces and trim. Runs are common after
/// control normalization (a `\r\n` becomes two spaces).
fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncate to at most `max_bytes` bytes on a character boundary, appending
/// [`TRUNCATION_MARKER`] when anything was actually removed.
///
/// The marker is inside the budget, so the returned string never exceeds
/// `max_bytes`; a budget too small for the marker degrades to a hard cut rather
/// than overshooting the bound.
fn truncate_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let marker_len = TRUNCATION_MARKER.len();
    if max_bytes <= marker_len {
        return cut_at_boundary(text, max_bytes).to_string();
    }
    let kept = cut_at_boundary(text, max_bytes - marker_len);
    format!("{kept}{TRUNCATION_MARKER}")
}

/// The longest prefix of `text` that is at most `max` bytes and ends on a
/// character boundary.
fn cut_at_boundary(text: &str, max: usize) -> &str {
    let mut end = max.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
