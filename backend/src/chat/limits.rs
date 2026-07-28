//! Admission control for chat turns: a process-wide concurrency ceiling plus a
//! one-turn-per-user guard.
//!
//! A chat turn is expensive and long-lived (a model stream plus up to
//! `max_tool_iterations` rounds of tool calls, each an in-process HTTP request), so
//! it needs bounding at BOTH ends:
//!
//! * **Per user** — a second concurrent turn from the same signed-in user is almost
//!   always a double-submit or a stuck tab, never a real need. Rejecting it protects
//!   the user's own next turn from queueing behind their duplicate.
//! * **Globally** — the ceiling keeps one busy deployment from spending its whole
//!   provider budget and connection pool on chat.
//!
//! Both are held by ONE RAII guard that releases on every exit path — success,
//! error, deadline, panic, or the browser disconnecting mid-stream — because the
//! guard is moved into the streaming task and dropped when that task ends. This
//! mirrors the crate's only other admission control
//! ([`crate::k8s::env_validator`]), including mapping saturation to
//! [`AppError::RateLimited`] so the client gets a 429 with `Retry-After`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::AppError;

/// How long a request waits for a free global slot before being told to retry. A
/// short grace absorbs a burst; a longer one would just hold the client open.
const ADMISSION_GRACE: Duration = Duration::from_secs(2);

/// `Retry-After` (seconds) advertised on either rejection. Both resolve on the
/// order of one turn, so one value answers both.
const RETRY_AFTER_SECS: u64 = 5;

/// The admission state for one [`ChatRuntime`](crate::chat::ChatRuntime).
///
/// Unlike `env_validator`'s process-global statics, this is an owned value: the
/// ceiling comes from that runtime's config, and tests need independent instances.
#[derive(Clone)]
pub struct ChatLimits {
    semaphore: Arc<Semaphore>,
    /// GitHub user ids with a turn in flight.
    inflight: Arc<Mutex<HashSet<i64>>>,
}

/// Holds both admission resources for the lifetime of one turn.
///
/// Moved into the streaming task, so dropping it — however the task ends — is what
/// releases the slot. There is deliberately no `release()` method: an explicit call
/// could be missed on an early return.
// `Debug` so a rejection can be asserted with `expect_err`, which needs it on the
// success type. It carries no secret — an admission is a user id and a permit.
#[derive(Debug)]
pub struct TurnAdmission {
    /// Held only to release the global slot on drop.
    _permit: OwnedSemaphorePermit,
    inflight: Arc<Mutex<HashSet<i64>>>,
    user_id: i64,
}

impl Drop for TurnAdmission {
    fn drop(&mut self) {
        // A poisoned lock still lets us clear our own id (we only ever remove).
        let mut set = match self.inflight.lock() {
            Ok(set) => set,
            Err(poisoned) => poisoned.into_inner(),
        };
        set.remove(&self.user_id);
    }
}

impl ChatLimits {
    pub fn new(max_concurrent_turns: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent_turns)),
            inflight: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Admit one turn for `user_id`, or explain why not.
    ///
    /// The per-user check runs FIRST so a double-submit never consumes a global slot
    /// it is about to be denied.
    pub async fn admit(&self, user_id: i64) -> Result<TurnAdmission, AppError> {
        {
            let mut set = match self.inflight.lock() {
                Ok(set) => set,
                Err(poisoned) => poisoned.into_inner(),
            };
            if !set.insert(user_id) {
                return Err(AppError::RateLimited {
                    message: "a chat turn is already in flight for this account".to_string(),
                    retry_after_secs: RETRY_AFTER_SECS,
                });
            }
        }

        let permit =
            match tokio::time::timeout(ADMISSION_GRACE, self.semaphore.clone().acquire_owned())
                .await
            {
                Ok(Ok(permit)) => permit,
                // Elapsed grace, or a (never-closed) semaphore: both mean "no slot".
                Ok(Err(_)) | Err(_) => {
                    // Release the per-user entry we just took, or this account would be
                    // locked out until the process restarts.
                    let mut set = match self.inflight.lock() {
                        Ok(set) => set,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    set.remove(&user_id);
                    return Err(AppError::RateLimited {
                        message: "chat capacity busy, retry".to_string(),
                        retry_after_secs: RETRY_AFTER_SECS,
                    });
                }
            };

        Ok(TurnAdmission {
            _permit: permit,
            inflight: self.inflight.clone(),
            user_id,
        })
    }
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
