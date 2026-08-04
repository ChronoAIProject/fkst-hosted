//! Bounded concurrency for activity queries — globally, and fairly per caller.
//!
//! A HogQL query is the most expensive thing this service can ask an upstream to
//! do, and the endpoint is polled by every open `/operations` tab. Two admission
//! limits keep that honest:
//!
//! - a GLOBAL cap, so the deployment cannot turn its own dashboard into a
//!   denial-of-service against PostHog;
//! - a PER-PRINCIPAL cap, so one caller with many tabs (or a script) cannot
//!   consume the global budget and starve everybody else.
//!
//! Exhaustion is an immediate `429` with a bounded `Retry-After`, never a queue:
//! waiting would convert a capacity problem into a latency problem, hold the
//! request open against the global timeout, and make the pressure invisible.
//!
//! The permit is RAII — [`ActivityPermit`] releases on drop — so an early return,
//! a `?`, or a panic inside the handler cannot leak capacity.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Global in-flight activity queries.
pub const DEFAULT_GLOBAL_LIMIT: u32 = 8;

/// In-flight activity queries per authenticated principal.
pub const DEFAULT_PER_PRINCIPAL_LIMIT: u32 = 2;

/// `Retry-After` seconds returned on exhaustion. Short, because the queries that
/// hold the permits are themselves bounded by the query timeout.
pub const RETRY_AFTER_SECS: u64 = 2;

/// Which limit refused admission. A closed enum: the only value that reaches a
/// metric label or a log line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionDenial {
    Global,
    PerPrincipal,
}

impl AdmissionDenial {
    /// The stable wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            AdmissionDenial::Global => "global_capacity",
            AdmissionDenial::PerPrincipal => "principal_capacity",
        }
    }
}

/// Shared admission state. Cheap to clone; every clone shares one budget.
#[derive(Clone, Debug)]
pub struct ActivityConcurrency {
    inner: Arc<Mutex<Counters>>,
    global_limit: u32,
    per_principal_limit: u32,
}

#[derive(Debug, Default)]
struct Counters {
    global: u32,
    per_principal: HashMap<i64, u32>,
}

impl Default for ActivityConcurrency {
    fn default() -> Self {
        Self::new(DEFAULT_GLOBAL_LIMIT, DEFAULT_PER_PRINCIPAL_LIMIT)
    }
}

impl ActivityConcurrency {
    /// Build a limiter with explicit budgets (tests drive tiny ones).
    pub fn new(global_limit: u32, per_principal_limit: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Counters::default())),
            global_limit,
            per_principal_limit,
        }
    }

    /// Try to admit one query for `principal_id`.
    ///
    /// The per-principal check runs FIRST so a heavy caller is told which of the
    /// two limits they hit, and so they cannot consume the last global slot at the
    /// exact moment a first-time caller arrives.
    pub fn try_acquire(&self, principal_id: i64) -> Result<ActivityPermit, AdmissionDenial> {
        let mut counters = match self.inner.lock() {
            Ok(counters) => counters,
            // A poisoned lock means a previous holder panicked while counting. The
            // budget is advisory, so recovering the guard is strictly better than
            // failing every subsequent query forever.
            Err(poisoned) => poisoned.into_inner(),
        };
        let held = counters
            .per_principal
            .get(&principal_id)
            .copied()
            .unwrap_or(0);
        if held >= self.per_principal_limit {
            return Err(AdmissionDenial::PerPrincipal);
        }
        if counters.global >= self.global_limit {
            return Err(AdmissionDenial::Global);
        }
        counters.global += 1;
        counters.per_principal.insert(principal_id, held + 1);
        Ok(ActivityPermit {
            inner: Arc::clone(&self.inner),
            principal_id,
        })
    }
}

/// An admitted query's capacity. Released on drop.
#[derive(Debug)]
pub struct ActivityPermit {
    inner: Arc<Mutex<Counters>>,
    principal_id: i64,
}

impl Drop for ActivityPermit {
    fn drop(&mut self) {
        let mut counters = match self.inner.lock() {
            Ok(counters) => counters,
            Err(poisoned) => poisoned.into_inner(),
        };
        counters.global = counters.global.saturating_sub(1);
        match counters.per_principal.get(&self.principal_id).copied() {
            Some(held) if held > 1 => {
                counters.per_principal.insert(self.principal_id, held - 1);
            }
            // The last permit for this principal removes the entry outright, so
            // the map cannot grow one slot per caller the process ever served.
            _ => {
                counters.per_principal.remove(&self.principal_id);
            }
        }
    }
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
