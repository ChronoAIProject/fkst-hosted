//! The documented, closed ordering of an authorized inventory.
//!
//! Three keys, applied in order:
//!
//! 1. **State rank** — active and problem states before settled terminal ones.
//! 2. **`created_at` descending**, nulls LAST. A runtime with no creation
//!    timestamp is the least informative row on the page, and putting it first
//!    would push the newest live sandboxes below a legacy orphan.
//! 3. **`runtime_id` ascending** — the backend's unique handle (a Pod name, a
//!    sandbox id), so two rows that agree on everything else still have exactly
//!    one order.
//!
//! ## Why this rank and not "alphabetical by status"
//!
//! An operations view is read top-down under time pressure. The ordering answers
//! "what needs attention, and what is still consuming the fleet" before it
//! answers anything else:
//!
//! | rank | state | why here |
//! |---|---|---|
//! | 0 | `failed` | terminal, but the only state that is a PROBLEM by itself |
//! | 1 | `pending` | alive and possibly stuck — the second-most actionable |
//! | 2 | `running` | alive and working |
//! | 3 | `transitioning` | alive, mid-pause/resume |
//! | 4 | `paused` | alive, deliberately parked |
//! | 5 | `terminating` | draining; still holds resources |
//! | 6 | `unknown` | indeterminate — may still be alive, so before the settled ones |
//! | 7 | `succeeded` | settled, finished well |
//! | 8 | `terminated` | settled, with no claim either way |
//!
//! ## Order can never depend on a hidden row
//!
//! The comparator is a total order over the three keys of the row itself. It
//! reads no index, no position, and no neighbour, so sorting the authorized subset
//! yields exactly the same sequence whatever else the fleet contained. That is the
//! property the isolation tests assert byte-for-byte.

use std::cmp::Ordering;

use crate::session_backend::inventory::{RuntimeInventoryItem, RuntimeInventoryStatus};

/// The documented rank of one normalized state (lower sorts first).
pub fn status_rank(status: RuntimeInventoryStatus) -> u8 {
    match status {
        RuntimeInventoryStatus::Failed => 0,
        RuntimeInventoryStatus::Pending => 1,
        RuntimeInventoryStatus::Running => 2,
        RuntimeInventoryStatus::Transitioning => 3,
        RuntimeInventoryStatus::Paused => 4,
        RuntimeInventoryStatus::Terminating => 5,
        RuntimeInventoryStatus::Unknown => 6,
        RuntimeInventoryStatus::Succeeded => 7,
        RuntimeInventoryStatus::Terminated => 8,
    }
}

/// Compare two authorized runtimes by the documented keys.
pub fn compare(left: &RuntimeInventoryItem, right: &RuntimeInventoryItem) -> Ordering {
    status_rank(left.status)
        .cmp(&status_rank(right.status))
        // Descending, nulls last: `None` is treated as the oldest possible
        // instant so it falls to the bottom of its rank group.
        .then_with(|| match (left.created_at, right.created_at) {
            (Some(left), Some(right)) => right.cmp(&left),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| left.runtime_id.cmp(&right.runtime_id))
}

#[cfg(test)]
#[path = "order_tests.rs"]
mod tests;
