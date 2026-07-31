//! Bounds and validators every safe-argument DTO is built from.
//!
//! Two rules make this module the whole redaction argument for `arguments`
//! (epic `AUD-03`):
//!
//! 1. **A value is captured only after the product's own parser accepted it.**
//!    Every helper here returns `Option`, and `None` means "the caller handed us
//!    something the validated form does not describe" — which the DTO answers by
//!    OMITTING the field, never by echoing a bounded prefix of the raw value. An
//!    invalid owner, run id, or blob sha is exactly the kind of attacker-chosen
//!    string an analytics store must not hold.
//! 2. **Every string and list is bounded.** The caps below are stated once and
//!    are deliberately at or slightly above the upstream format's own maximum (a
//!    GitHub login is 39 characters, a label 50, a branch 200), so a bound can
//!    never silently truncate a legitimate value — it only stops an unbounded
//!    one.
//!
//! Bounding a LIST never changes the business request: [`bounded_list`] projects
//! a prefix for the record while the handler keeps working with the full input,
//! and reports the true `count` plus a `truncated` marker so a reader can tell a
//! short list from a clipped one.

use crate::goals::section_parse::is_valid_env_name;
use crate::reconcile::branches::validate_branch_name;

/// Maximum entries kept from a package/manifest reference list (spec: 100).
pub const MAX_REF_ENTRIES: usize = 100;
/// Maximum bytes of one package/manifest reference (spec: 256).
pub const MAX_REF_LEN: usize = 256;
/// Maximum bytes of a repository owner login. GitHub's own cap is 39.
pub const MAX_OWNER_LEN: usize = 64;
/// Maximum bytes of a repository name. GitHub's own cap is 100.
pub const MAX_REPO_LEN: usize = 100;
/// Maximum bytes of a git branch name, matching [`validate_branch_name`].
pub const MAX_BRANCH_LEN: usize = 200;
/// Maximum bytes of a GitHub label name. GitHub's own cap is 50.
pub const MAX_WORK_LABEL_LEN: usize = 50;
/// Maximum bytes of an `### Output Language` locale tag.
pub const MAX_OUTPUT_LANG_LEN: usize = 16;
/// Maximum bytes of a session id (a UUIDv5 today; the audit contract's bound).
pub const MAX_SESSION_ID_LEN: usize = 128;
/// Maximum bytes of a log run id.
pub const MAX_RUN_ID_LEN: usize = 64;
/// Maximum bytes of a git object id (sha-256 hex is 64).
pub const MAX_BLOB_SHA_LEN: usize = 64;
/// Maximum bytes of a normalized media type.
pub const MAX_CONTENT_TYPE_LEN: usize = 128;

/// The `run` selector recorded when a request asked for the authoritative
/// whole-session bundle (absent, blank, or the literal `latest`).
pub const RUN_LATEST: &str = "latest";

/// A bounded list projection: the retained prefix, the true length, and whether
/// anything was dropped.
///
/// The handler keeps operating on the complete input — this is a view built for
/// the record only, which is what "never let audit serialization change the
/// business request" means in practice.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BoundedList {
    /// The retained prefix, at most `limit` entries.
    pub items: Vec<String>,
    /// The TRUE number of entries, before any bound was applied.
    pub count: usize,
    /// True when `items` is a prefix rather than the whole list.
    pub truncated: bool,
}

/// Project `values` into a bounded list of at most `limit` entries.
///
/// Entries that `accept` rejects are dropped from `items` but still counted:
/// `count` describes the request the caller actually made, not the subset that
/// happened to be recordable.
pub fn bounded_list<'a, I, F>(values: I, limit: usize, accept: F) -> BoundedList
where
    I: IntoIterator<Item = &'a str>,
    F: Fn(&str) -> Option<String>,
{
    let mut items = Vec::new();
    let mut count = 0usize;
    for value in values {
        count = count.saturating_add(1);
        if items.len() < limit {
            if let Some(accepted) = accept(value) {
                items.push(accepted);
            }
        }
    }
    BoundedList {
        truncated: count > items.len(),
        items,
        count,
    }
}

/// The UTF-8 byte length of `value`, saturating into the wire's `u64`.
///
/// A byte COUNT is the only thing a free-text field (an issue title, a
/// repository description, a chat message) may contribute to a record: it is
/// useful for spotting an anomalous request and carries none of the content.
pub fn byte_len(value: &str) -> u64 {
    u64::try_from(value.len()).unwrap_or(u64::MAX)
}

/// Accept `value` when it is a bounded, control-character-free ASCII token drawn
/// from alphanumerics plus `extra`.
///
/// This is the shape check every identifier helper below is built on. It is
/// deliberately conservative: a value that is legal upstream but not expressible
/// here is dropped rather than recorded, because the cost of a missing
/// correlation handle is far lower than the cost of a smuggled one.
fn ascii_token(value: &str, max: usize, extra: &[char]) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > max {
        return None;
    }
    value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
        .then(|| value.to_string())
}

/// A repository owner login, in the validated form the canvas routes enforce.
pub fn safe_owner(value: &str) -> Option<String> {
    ascii_token(value, MAX_OWNER_LEN, &['.', '_', '-'])
}

/// A repository name, in the validated form the canvas routes enforce.
pub fn safe_repo(value: &str) -> Option<String> {
    ascii_token(value, MAX_REPO_LEN, &['.', '_', '-'])
}

/// `owner/name`, present only when BOTH halves validate. Used for the top-level
/// correlation field, whose own contract requires exactly two non-empty parts.
pub fn safe_repo_full_name(owner: &str, name: &str) -> Option<String> {
    Some(format!("{}/{}", safe_owner(owner)?, safe_repo(name)?))
}

/// A deterministic session id.
pub fn safe_session_id(value: &str) -> Option<String> {
    ascii_token(value, MAX_SESSION_ID_LEN, &['.', '_', '-'])
}

/// A log run id, or [`RUN_LATEST`] for the authoritative whole-session bundle.
///
/// `None` means the caller supplied a `?run=` value that is not a run id — the
/// DTO then omits the field and reports `invalid`, because echoing the raw
/// selector is exactly the "never echo invalid material" rule.
pub fn safe_run_id(value: Option<&str>) -> Option<String> {
    match value.map(str::trim) {
        None | Some("") | Some(RUN_LATEST) => Some(RUN_LATEST.to_string()),
        Some(run) => ascii_token(run, MAX_RUN_ID_LEN, &['.', '_', '-']),
    }
}

/// A git object id, validated as hex exactly like the blob route's own guard.
pub fn safe_blob_sha(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= MAX_BLOB_SHA_LEN
        && value.chars().all(|c| c.is_ascii_hexdigit()))
    .then(|| value.to_string())
}

/// A git branch name, accepted only by the product's own branch validator so the
/// record can never describe a branch the request could not have used.
pub fn safe_branch(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() > MAX_BRANCH_LEN {
        return None;
    }
    validate_branch_name(value).ok().map(|()| value.to_string())
}

/// A GitHub label name, bounded to GitHub's own cap and free of the separators
/// that would let one value forge two fields in a structured log or query.
pub fn safe_work_label(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_WORK_LABEL_LEN {
        return None;
    }
    (!value
        .chars()
        .any(|c| c.is_control() || c == ',' || c == '"'))
    .then(|| value.to_string())
}

/// A named-environment name, accepted only in the store's own validated form.
pub fn safe_environment_name(value: &str) -> Option<String> {
    let value = value.trim();
    is_valid_env_name(value).then(|| value.to_string())
}

/// An `### Output Language` locale tag.
pub fn safe_output_lang(value: &str) -> Option<String> {
    ascii_token(value, MAX_OUTPUT_LANG_LEN, &['-', '_'])
}

/// The normalized media type of a request body: lower-cased, parameters (charset,
/// boundary) stripped, and bounded.
///
/// Parameters are dropped rather than kept because a `boundary=` value is
/// caller-chosen free text, which the malformed-input contract forbids.
pub fn safe_content_type(value: &str) -> Option<String> {
    let media = value.split(';').next().unwrap_or_default().trim();
    if media.is_empty() || media.len() > MAX_CONTENT_TYPE_LEN {
        return None;
    }
    (!media
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '"'))
    .then(|| media.to_ascii_lowercase())
}

#[cfg(test)]
#[path = "bounds_tests.rs"]
mod tests;
