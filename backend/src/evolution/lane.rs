//! The canonical Evolution lane: branch naming, machine markers, duplicate
//! repair, and the auto-merge exclusion.
//!
//! For each `(source repository, artifact repository, trusted source branch)`
//! there may be at most one open sync issue, one live execution, one open sync
//! pull request, and one pending Release-asset set per input fingerprint.
//!
//! **Why a lock is needed at all.** There is no compare-and-swap on GitHub issue
//! creation, leader election is an unfenced time-based Lease, and the issue index
//! is eventually consistent. Duplicate creation is therefore possible and needs a
//! specified repair, not merely a specified prohibition.
//!
//! **The primary lock is ref creation.** `POST /git/refs` fails with `422` when
//! the ref exists, which makes creating the sync branch the atomic
//! compare-and-swap the issue index cannot provide. That only works if two racing
//! reconciliations compute the SAME name, which is why the branch is keyed on the
//! input fingerprint alone — see [`sync_branch_name`].

use serde::{Deserialize, Serialize};

/// Prefix of every Evolution-owned sync branch.
pub const SYNC_BRANCH_PREFIX: &str = "fkst/evolution/";

/// Marker identifying a canonical sync pull request.
pub const PR_MARKER_TAG: &str = "fkst-evolution-pr:v1";
/// Marker identifying the singleton sync issue.
pub const SYNC_MARKER_TAG: &str = "fkst-evolution-sync:v1";
/// Marker identifying an advisory pull-request preview comment.
pub const PREVIEW_MARKER_TAG: &str = "fkst-evolution-preview:v1";

/// Upper bound on a marker's JSON payload.
///
/// Markers must be bounded. An unbounded payload is both a parsing cost on every
/// reconcile and a place to smuggle content into a resource the controller reads
/// automatically.
const MAX_MARKER_BYTES: usize = 4096;

/// Hex characters of a fingerprint used in a branch name or Release tag.
///
/// 16 hex characters is 64 bits. At 8 (32 bits) a collision becomes likely within
/// a few tens of thousands of artifacts, and a collision here is unrecoverable by
/// design: the name is taken and the content differs.
const SHORT_HASH_LEN: usize = 16;

/// Shorten a `sha256:<hex>` fingerprint for use in a ref or tag name.
pub fn short_hash(fingerprint: &str) -> Option<String> {
    let hex = fingerprint.strip_prefix("sha256:")?;
    if hex.len() < SHORT_HASH_LEN || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex[..SHORT_HASH_LEN].to_string())
}

/// The sync branch for an input fingerprint.
///
/// Keyed on the input fingerprint **alone**, deliberately. Creating this ref is
/// the lane's atomic lock, which only works if two racing reconciliations derive
/// the same name; including the issue number would give them different names and
/// defeat the lock entirely.
pub fn sync_branch_name(input_fingerprint: &str) -> Option<String> {
    Some(format!(
        "{SYNC_BRANCH_PREFIX}{}",
        short_hash(input_fingerprint)?
    ))
}

/// The namespaced Release tag holding an input's rendered artifacts.
pub fn release_tag(input_fingerprint: &str) -> Option<String> {
    Some(format!("fkst-evolution/{}", short_hash(input_fingerprint)?))
}

/// True for a tag Evolution itself owns.
///
/// A Release Evolution published must never be read as a product release, or
/// publishing artifacts re-triggers the cycle that produced them.
pub fn is_evolution_tag(tag: &str) -> bool {
    tag.starts_with("fkst-evolution/")
}

/// Marker payload carried by a canonical sync pull request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncPrMarker {
    /// The sync issue this pull request belongs to.
    pub issue: i64,
    /// The input fingerprint the pull request was generated for.
    pub input: String,
    #[serde(rename = "sourceHead")]
    pub source_head: String,
    pub generator: String,
    pub verification: String,
}

/// Render a marker as the HTML comment embedded in an issue or pull request body.
pub fn render_marker<T: Serialize>(tag: &str, payload: &T) -> Result<String, String> {
    let json = serde_json::to_string(payload).map_err(|e| format!("serialize marker: {e}"))?;
    if json.len() > MAX_MARKER_BYTES {
        return Err(format!("marker exceeds {MAX_MARKER_BYTES} bytes"));
    }
    Ok(format!("<!-- {tag}\n{json}\n-->"))
}

/// Extract and parse a marker payload from a body.
///
/// Returns `None` when the marker is absent, malformed, or over the size bound.
///
/// A parsed marker is DATA, never authority. It is trusted only when it is
/// attached to the expected resource, authored by the configured App identity,
/// and consistent with current GitHub state — none of which this function can
/// establish. Callers must check authorship themselves; marker text alone never
/// establishes singleton ownership.
pub fn parse_marker<T: for<'de> Deserialize<'de>>(body: &str, tag: &str) -> Option<T> {
    let opener = format!("<!-- {tag}");
    let start = body.find(&opener)? + opener.len();
    let rest = &body[start..];
    let end = rest.find("-->")?;
    let json = rest[..end].trim();
    if json.len() > MAX_MARKER_BYTES {
        return None;
    }
    serde_json::from_str(json).ok()
}

/// Whether a pull request is a canonical Evolution sync pull request.
///
/// This is the predicate that excludes Evolution's own pull requests from the
/// generic repository-level auto-merge hook. Evolution's merge is safety-gated —
/// path-scoped, current-head-scoped and check-gated — and the generic hook is
/// none of those: it merges any mergeable bot pull request once ANY session on
/// the repository has opted in. Without this exclusion, enrolling Evolution on a
/// repository that already had an auto-merge session would hand the generic hook
/// the artifact pull requests, bypassing every gate that makes autonomous
/// artifact merging safe.
pub fn is_sync_pull_request(body: &str) -> bool {
    parse_marker::<SyncPrMarker>(body, PR_MARKER_TAG).is_some()
}

/// Whether a pull request's head is an Evolution-owned sync branch.
///
/// The branch-name form of [`is_sync_pull_request`], for callers that have a
/// pull request's head but not its body.
///
/// `head_repo` is checked against `base_repo` first and is not optional
/// diligence: for a cross-repository (fork) pull request the head ref is a bare
/// branch name in the FORK, which anyone can name anything. Without that guard a
/// contributor could open a fork pull request from a branch called
/// `fkst/evolution/deadbeefdeadbeef` and have it treated as Evolution-owned.
/// Evolution's own sync branch is always same-repository, because the controller
/// creates it there.
pub fn is_sync_branch(head_ref: &str, head_repo: Option<&str>, base_repo: &str) -> bool {
    if head_repo != Some(base_repo) {
        return false;
    }
    let Some(rest) = head_ref.strip_prefix(SYNC_BRANCH_PREFIX) else {
        return false;
    };
    // Shape-checked rather than merely prefix-checked, so an unrelated branch
    // under the same prefix is not mistaken for a lane.
    rest.len() == SHORT_HASH_LEN && rest.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A candidate coordination resource observed while reconciling a lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneCandidate {
    pub number: i64,
}

/// The outcome of resolving duplicate coordination resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneResolution {
    /// The resource to keep. `None` when none were observed.
    pub canonical: Option<i64>,
    /// Resources to comment on and close, ascending.
    pub duplicates: Vec<i64>,
}

impl LaneResolution {
    /// True when duplication occurred and should be reported.
    ///
    /// Repeated duplication indicates a leader or index problem rather than a
    /// benign race, so it is surfaced rather than silently repaired.
    pub fn is_duplicated(&self) -> bool {
        !self.duplicates.is_empty()
    }
}

/// Resolve duplicate sync issues or pull requests for one lane.
///
/// The **lowest-numbered** resource is canonical. Markers and App identity cannot
/// disambiguate duplicates of the same lane — they carry identical markers and
/// the same author — so an arbitrary but *stable* rule is required, and creation
/// order is the only totally-ordered signal GitHub gives us. Duplicates are
/// closed with a pointer comment, never deleted: closing preserves the audit
/// trail that duplication happened.
pub fn resolve_lane(candidates: &[LaneCandidate]) -> LaneResolution {
    let mut numbers: Vec<i64> = candidates.iter().map(|c| c.number).collect();
    numbers.sort_unstable();
    numbers.dedup();
    let mut iter = numbers.into_iter();
    let canonical = iter.next();
    LaneResolution {
        canonical,
        duplicates: iter.collect(),
    }
}

#[cfg(test)]
#[path = "lane_tests.rs"]
mod tests;
