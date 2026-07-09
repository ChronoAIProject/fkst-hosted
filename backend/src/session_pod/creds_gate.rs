//! The in-pod credentials-complete GATE (issue #415).
//!
//! The `run-substrate` driver must not start the engine until the control plane has
//! finished writing every credential file into the mounted Secret volume. The
//! credential writer creates a
//! [`CREDS_COMPLETE_SENTINEL`](crate::session_spec::creds::CREDS_COMPLETE_SENTINEL)
//! marker LAST, so its presence proves the whole set is on disk. This module is the
//! bounded, poll-based wait the driver runs before its first credential read.
//!
//! In k8s-customized mode the sentinel rides the atomically-mounted Secret, so the
//! very first check already passes (the gate costs ~0ms). The bounded wait + timeout
//! exist only for a backend that writes the creds incrementally; a writer that never
//! finishes trips the timeout and the driver aborts engine start rather than running
//! with a half-written credential set.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Env var an operator can set (on the in-pod driver) to override the bounded wait
/// for the credentials-complete sentinel before the engine starts.
const CREDS_WAIT_TIMEOUT_ENV: &str = "FKST_CREDS_WAIT_TIMEOUT_SECS";
/// The bounded wait used when the env var is absent, blank, or unparseable.
const DEFAULT_CREDS_WAIT_TIMEOUT_SECS: u64 = 120;
/// How often the gate re-checks for the sentinel while it waits.
pub const CREDS_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// The bounded wait for the sentinel, resolved from [`CREDS_WAIT_TIMEOUT_ENV`]. A
/// thin wrapper over the pure [`parse_creds_wait_timeout`] so the parse rules stay
/// unit-testable without mutating the process-global env.
pub fn creds_wait_timeout_from_env() -> Duration {
    parse_creds_wait_timeout(std::env::var(CREDS_WAIT_TIMEOUT_ENV).ok().as_deref())
}

/// Parse the bounded-wait timeout from a raw env value: `Some("<u64>")` → that many
/// seconds; an absent value → the default (silent); a set-but-unparseable value → a
/// warning naming the var, the bad value, and the fallback, then the default.
fn parse_creds_wait_timeout(raw: Option<&str>) -> Duration {
    let Some(raw) = raw else {
        return Duration::from_secs(DEFAULT_CREDS_WAIT_TIMEOUT_SECS);
    };
    match raw.trim().parse::<u64>() {
        Ok(secs) => Duration::from_secs(secs),
        Err(_) => {
            tracing::warn!(
                var = CREDS_WAIT_TIMEOUT_ENV,
                value = %raw,
                fallback_secs = DEFAULT_CREDS_WAIT_TIMEOUT_SECS,
                "invalid credentials-wait timeout; using the default"
            );
            Duration::from_secs(DEFAULT_CREDS_WAIT_TIMEOUT_SECS)
        }
    }
}

/// The gate gave up: the credentials-complete sentinel never appeared within the
/// bounded wait. Non-secret — it carries only the (public) sentinel path + elapsed.
#[derive(Debug)]
pub struct CredsWaitTimeout {
    /// The sentinel path that never appeared.
    pub sentinel: PathBuf,
    /// How long the gate waited before giving up.
    pub elapsed: Duration,
}

impl std::fmt::Display for CredsWaitTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "credentials-complete sentinel {} did not appear within {:?}",
            self.sentinel.display(),
            self.elapsed
        )
    }
}

/// Wait (async) until `sentinel` exists, polling every `poll`, up to `timeout`.
/// Returns the elapsed wait on success, or [`CredsWaitTimeout`] once the deadline
/// passes without the sentinel. `poll` is a parameter so tests inject a tiny
/// interval; the final sleep is clamped to the remaining budget so the wait never
/// overshoots the deadline.
pub async fn wait_for_creds_complete(
    sentinel: &Path,
    timeout: Duration,
    poll: Duration,
) -> Result<Duration, CredsWaitTimeout> {
    let start = Instant::now();
    loop {
        if sentinel.exists() {
            return Ok(start.elapsed());
        }
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            return Err(CredsWaitTimeout {
                sentinel: sentinel.to_path_buf(),
                elapsed,
            });
        }
        tokio::time::sleep(poll.min(timeout.saturating_sub(elapsed))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_absent_value_uses_the_default() {
        assert_eq!(
            parse_creds_wait_timeout(None),
            Duration::from_secs(DEFAULT_CREDS_WAIT_TIMEOUT_SECS)
        );
    }

    #[test]
    fn parse_valid_value_is_honored() {
        assert_eq!(
            parse_creds_wait_timeout(Some("30")),
            Duration::from_secs(30)
        );
        // Whitespace is trimmed like the sibling env parsers.
        assert_eq!(
            parse_creds_wait_timeout(Some("  45 ")),
            Duration::from_secs(45)
        );
    }

    #[test]
    fn parse_unparseable_value_falls_back_to_the_default() {
        assert_eq!(
            parse_creds_wait_timeout(Some("not-a-number")),
            Duration::from_secs(DEFAULT_CREDS_WAIT_TIMEOUT_SECS)
        );
    }

    #[tokio::test]
    async fn returns_immediately_when_the_sentinel_already_exists() {
        let dir = tempfile::tempdir().expect("dir");
        let sentinel = dir.path().join(".creds-complete");
        std::fs::write(&sentinel, "1").expect("write sentinel");

        let waited =
            wait_for_creds_complete(&sentinel, Duration::from_secs(2), Duration::from_millis(5))
                .await
                .expect("sentinel present");
        // A pre-existing sentinel passes on the first check (k8s-customized mode).
        assert!(waited < Duration::from_millis(100), "waited {waited:?}");
    }

    #[tokio::test]
    async fn resolves_once_the_sentinel_appears_mid_wait() {
        let dir = tempfile::tempdir().expect("dir");
        let sentinel = dir.path().join(".creds-complete");
        let sentinel_bg = sentinel.clone();
        // Create the sentinel after a few poll cycles, mimicking an incremental writer.
        let creator = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            std::fs::write(&sentinel_bg, "1").expect("write sentinel");
        });

        let waited =
            wait_for_creds_complete(&sentinel, Duration::from_secs(2), Duration::from_millis(5))
                .await
                .expect("sentinel appeared");
        creator.await.expect("creator task");
        // It waited roughly the creation delay, well under the timeout.
        assert!(waited >= Duration::from_millis(30), "waited {waited:?}");
        assert!(waited < Duration::from_secs(1), "waited {waited:?}");
    }

    #[tokio::test]
    async fn times_out_when_the_sentinel_never_appears() {
        let dir = tempfile::tempdir().expect("dir");
        let sentinel = dir.path().join(".creds-complete");

        let err = wait_for_creds_complete(
            &sentinel,
            Duration::from_millis(150),
            Duration::from_millis(10),
        )
        .await
        .expect_err("must time out");
        assert_eq!(err.sentinel, sentinel);
        assert!(
            err.elapsed >= Duration::from_millis(150),
            "elapsed {:?}",
            err.elapsed
        );
    }
}
