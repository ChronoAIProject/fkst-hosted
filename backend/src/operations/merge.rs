//! Merging two already-authorized source pages into one keyset page.
//!
//! The merge layer performs NO authorization. Every record reaching it passed its
//! source's own predicate before that source's `LIMIT` (see
//! [`super::source::ActivitySource`]), which is what makes the page boundary a
//! function of rows the caller may actually see.
//!
//! ## Partial is a first-class answer
//!
//! ```text
//! posthog ok  + relay ok        -> complete page
//! posthog ok  + relay down      -> authorized history,       partial = true
//! posthog down + relay ok       -> authorized recent rows,   partial = true
//! posthog down + relay down     -> 503, never `items: []`
//! ```
//!
//! The last line is the whole point: an empty page and an outage look identical
//! to a user, and only one of them means "nothing happened". A source that cannot
//! answer is reported, never rounded down to zero rows.
//!
//! ## Deduplication keeps the more alarming truth
//!
//! One event can arrive from both sources. PostHog's CONTENT wins — it is the
//! verified projection — but the delivery STATE is merged by severity, so a relay
//! copy saying a delivery is stuck or dead is not erased by a PostHog copy saying
//! it is fine (see [`super::record::ActivityRecord::merge_delivery`]).
//!
//! ## The constraint invariant
//!
//! [`enforce_constraint_invariant`] re-checks the personal actor predicate on
//! records that came back. It is an ASSERTION, not the authorization: the
//! predicate is already in the source query, and this can only ever remove rows,
//! never admit them. It exists because a source adapter is the one place a
//! predicate could regress silently, and a loud metric plus a dropped row is a
//! much better failure than a leaked one. It is deliberately excluded from the
//! user-visible row-error counters, so a regression shows up on a dashboard
//! rather than in somebody's page metadata.

use std::collections::HashMap;

use crate::session_access::ActivityVisibilityConstraint;

use super::cursor::CursorKey;
use super::record::{ActivityRecord, ActivitySourceKind};
use super::source::{SourceError, SourcePage};

/// The health of one source for one query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceHealth {
    /// The deployment does not configure this source at all.
    NotConfigured,
    /// Answered, and every row decoded.
    Healthy,
    /// Answered, but some already-authorized rows could not be decoded.
    Degraded,
    /// Could not answer.
    Unavailable,
}

impl SourceHealth {
    /// The stable wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceHealth::NotConfigured => "not_configured",
            SourceHealth::Healthy => "healthy",
            SourceHealth::Degraded => "degraded",
            SourceHealth::Unavailable => "unavailable",
        }
    }
}

/// Bounded, stable codes describing WHY a page is partial. Deployment health
/// only — never a count or a property of hidden records.
pub mod message_codes {
    pub const POSTHOG_UNAVAILABLE: &str = "posthog_unavailable";
    pub const RELAY_UNAVAILABLE: &str = "relay_unavailable";
    pub const ROWS_DROPPED: &str = "activity_rows_dropped";
}

/// The per-source health block returned with every page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceStatus {
    pub posthog: SourceHealth,
    pub relay: SourceHealth,
    pub partial: bool,
    pub message_code: Option<&'static str>,
}

/// One assembled page.
#[derive(Debug)]
pub struct MergedPage {
    pub items: Vec<ActivityRecord>,
    /// The sort key of the last returned row, when another page may exist.
    pub next_key: Option<CursorKey>,
    pub status: SourceStatus,
    /// Rows dropped by the typed row contract, across already-authorized
    /// candidates only.
    pub row_errors: usize,
    /// Rows dropped by [`enforce_constraint_invariant`]. Operator telemetry; never
    /// part of any response.
    pub constraint_violations: usize,
    /// Second copies of an event id that both sources returned. Bounded
    /// telemetry: at-least-once delivery makes duplicates NORMAL, so this is a
    /// rate to watch rather than an error.
    pub duplicates: usize,
}

/// One source's outcome, as handed to the merge.
pub type SourceOutcome = Option<Result<SourcePage, SourceError>>;

/// Merge the two source outcomes into one page.
///
/// Returns `Err` with the more severe source error when NEITHER source could
/// answer; the caller maps that onto `502`/`503`. A configured-but-absent source
/// (`None`) is `not_configured` and never counts as an outage.
pub fn merge(
    constraint: &ActivityVisibilityConstraint,
    posthog: SourceOutcome,
    relay: SourceOutcome,
    limit: u32,
    fetch_limit: u32,
) -> Result<MergedPage, SourceError> {
    let (posthog_health, posthog_page, posthog_error) = classify(posthog);
    let (relay_health, relay_page, relay_error) = classify(relay);

    // NO source produced a page. An unconfigured source is not an outage, but it
    // is also not an answer: if the only configured source failed, there is no
    // authorized history to show, and `items: []` would be a confident lie.
    if posthog_page.is_none() && relay_page.is_none() {
        // Prefer the fault that an operator can act on: an auth/schema failure is
        // a deployment problem, a transient one is a retry.
        let error = [posthog_error, relay_error]
            .into_iter()
            .flatten()
            .max_by_key(|error| u8::from(error.is_upstream_fault()))
            .unwrap_or(SourceError::Transient { kind: "no_source" });
        return Err(error);
    }

    let mut row_errors = 0usize;
    let mut saturated = false;
    let mut candidates = Vec::new();
    for page in [posthog_page, relay_page].into_iter().flatten() {
        row_errors += page.row_errors;
        saturated |= page.saturated(fetch_limit);
        candidates.extend(page.records);
    }

    let constraint_violations = enforce_constraint_invariant(constraint, &mut candidates);
    let candidate_count = candidates.len();
    let mut items = deduplicate(candidates);
    let duplicates = candidate_count - items.len();
    sort_newest_first(&mut items);

    let has_more = items.len() > limit as usize || saturated;
    items.truncate(limit as usize);
    let next_key = (has_more && !items.is_empty()).then(|| {
        let last = &items[items.len() - 1];
        CursorKey {
            timestamp: last.sort_timestamp(),
            event_id: last.event_id().to_string(),
        }
    });

    let partial = posthog_health == SourceHealth::Unavailable
        || relay_health == SourceHealth::Unavailable
        || row_errors > 0;
    let message_code = if posthog_health == SourceHealth::Unavailable {
        Some(message_codes::POSTHOG_UNAVAILABLE)
    } else if relay_health == SourceHealth::Unavailable {
        Some(message_codes::RELAY_UNAVAILABLE)
    } else if row_errors > 0 {
        Some(message_codes::ROWS_DROPPED)
    } else {
        None
    };

    Ok(MergedPage {
        items,
        next_key,
        status: SourceStatus {
            posthog: posthog_health,
            relay: relay_health,
            partial,
            message_code,
        },
        row_errors,
        constraint_violations,
        duplicates,
    })
}

/// Split a source outcome into health, page, and error.
fn classify(outcome: SourceOutcome) -> (SourceHealth, Option<SourcePage>, Option<SourceError>) {
    match outcome {
        None => (SourceHealth::NotConfigured, None, None),
        Some(Ok(page)) if page.row_errors > 0 => (SourceHealth::Degraded, Some(page), None),
        Some(Ok(page)) => (SourceHealth::Healthy, Some(page), None),
        Some(Err(error)) => (SourceHealth::Unavailable, None, Some(error)),
    }
}

/// Drop any record that contradicts the personal actor predicate. See the module
/// docs: an assertion, not the authorization.
fn enforce_constraint_invariant(
    constraint: &ActivityVisibilityConstraint,
    records: &mut Vec<ActivityRecord>,
) -> usize {
    let ActivityVisibilityConstraint::Mine(scope) = constraint else {
        return 0;
    };
    let viewer_id = scope.actor_id();
    let session_id = scope.lifecycle_session_id();
    let before = records.len();
    records.retain(|record| {
        let allowed = if record.is_lifecycle() {
            // A lifecycle row is a SYSTEM record: it is visible because its
            // session was authorized, never because of who its actor is.
            session_id.is_some_and(|authorized| record.session_id() == Some(authorized))
        } else {
            record.actor_id() == Some(viewer_id)
        };
        if !allowed {
            tracing::error!(
                source = record.source().as_str(),
                lifecycle = record.is_lifecycle(),
                "operations: a source returned a record outside the personal visibility \
                 constraint; dropping it — the source predicate has regressed"
            );
        }
        allowed
    });
    before - records.len()
}

/// Collapse duplicate event ids, preferring PostHog's content and retaining the
/// most severe delivery state.
fn deduplicate(records: Vec<ActivityRecord>) -> Vec<ActivityRecord> {
    let mut order: Vec<String> = Vec::with_capacity(records.len());
    let mut kept: HashMap<String, ActivityRecord> = HashMap::with_capacity(records.len());
    for record in records {
        let key = record.event_id().to_string();
        match kept.get_mut(&key) {
            None => {
                order.push(key.clone());
                kept.insert(key, record);
            }
            Some(existing) => {
                let incoming_state = record.delivery_state();
                if existing.source() != ActivitySourceKind::Posthog
                    && record.source() == ActivitySourceKind::Posthog
                {
                    let existing_state = existing.delivery_state();
                    *existing = record;
                    existing.merge_delivery(existing_state);
                } else {
                    existing.merge_delivery(incoming_state);
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|key| kept.remove(&key))
        .collect()
}

/// Newest first, with the event id breaking ties so identical timestamps still
/// order deterministically — the same total order every source applies.
fn sort_newest_first(records: &mut [ActivityRecord]) {
    records.sort_by(|left, right| {
        right
            .sort_timestamp()
            .cmp(&left.sort_timestamp())
            .then_with(|| right.event_id().cmp(left.event_id()))
    });
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
