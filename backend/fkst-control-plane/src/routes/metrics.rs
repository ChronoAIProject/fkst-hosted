//! `GET /metrics`: hand-rendered Prometheus text exposition for the controller
//! (issue #144).
//!
//! Datastore-free (#143), so there is nothing to scrape from a DB; these gauges
//! reflect the controller's live in-memory authorities:
//! - `fkst_pending_work`        — claims in the `Pending` (unplaced) state.
//! - `fkst_workers_registered`  — workers currently tracked by the registry.
//! - `fkst_workers_alive`       — of those, the ones within the liveness TTL.
//!
//! Rendered by hand (no client library): the surface is three gauges, so a
//! dependency would be unjustified. The body follows the Prometheus text format
//! exactly — a `# HELP` line, a `# TYPE <name> gauge` line, then the sample —
//! per metric.
//!
//! Unauthenticated, like `/health`: it carries NO secret (only counts) and is
//! served on the ClusterIP-only surface, so Prometheus can scrape it without a
//! NyxID identity.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use crate::state::AppState;

/// The Prometheus text content type (version 0.0.4 exposition format).
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// One gauge metric to render.
struct Gauge {
    name: &'static str,
    help: &'static str,
    value: u64,
}

/// Render one gauge as the three canonical lines (`# HELP`, `# TYPE`, sample).
fn render_gauge(out: &mut String, gauge: &Gauge) {
    out.push_str("# HELP ");
    out.push_str(gauge.name);
    out.push(' ');
    out.push_str(gauge.help);
    out.push('\n');
    out.push_str("# TYPE ");
    out.push_str(gauge.name);
    out.push_str(" gauge\n");
    out.push_str(gauge.name);
    out.push(' ');
    out.push_str(&gauge.value.to_string());
    out.push('\n');
}

/// Render the full exposition body from the supplied gauge values. Split out so
/// it is unit-testable without constructing an HTTP request.
fn render_metrics(pending_work: u64, workers_registered: u64, workers_alive: u64) -> String {
    let gauges = [
        Gauge {
            name: "fkst_pending_work",
            help: "Claims in the pending (unplaced) state awaiting placement.",
            value: pending_work,
        },
        Gauge {
            name: "fkst_workers_registered",
            help: "Workers currently tracked by the controller registry.",
            value: workers_registered,
        },
        Gauge {
            name: "fkst_workers_alive",
            help: "Tracked workers within the liveness TTL (heartbeating).",
            value: workers_alive,
        },
    ];
    let mut out = String::new();
    for gauge in &gauges {
        render_gauge(&mut out, gauge);
    }
    out
}

/// `GET /metrics`: the controller's Prometheus gauges. Unauthenticated.
async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    // Pending work: count of `Pending` claims (0 when no controller is wired).
    let pending_work = state
        .claims
        .as_ref()
        .map(|c| c.pending_count() as u64)
        .unwrap_or(0);

    // Worker counts from the registry snapshot (0/0 when none is wired).
    let (registered, alive) = match &state.worker_registry {
        Some(registry) => {
            let snapshot = registry.snapshot().await;
            let alive = snapshot.iter().filter(|w| w.alive).count() as u64;
            (snapshot.len() as u64, alive)
        }
        None => (0, 0),
    };

    let body = render_metrics(pending_work, registered, alive);
    (
        [(axum::http::header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        body,
    )
}

/// `/metrics` route, mounted at the TOP level (unauthenticated, like `/health`).
pub fn router() -> Router<AppState> {
    Router::new().route("/metrics", get(metrics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_prometheus_text_for_each_gauge() {
        let body = render_metrics(2, 3, 1);
        // Each gauge has its HELP, TYPE, and sample line in the exact format.
        for (name, value) in [
            ("fkst_pending_work", 2),
            ("fkst_workers_registered", 3),
            ("fkst_workers_alive", 1),
        ] {
            assert!(
                body.contains(&format!("# TYPE {name} gauge")),
                "missing TYPE line for {name} in:\n{body}"
            );
            assert!(
                body.contains(&format!("# HELP {name} ")),
                "missing HELP line for {name} in:\n{body}"
            );
            assert!(
                body.contains(&format!("\n{name} {value}\n"))
                    || body.starts_with(&format!("{name} {value}\n")),
                "missing sample `{name} {value}` in:\n{body}"
            );
        }
    }

    #[test]
    fn gauge_value_reflects_the_argument() {
        let body = render_metrics(7, 0, 0);
        assert!(
            body.contains("\nfkst_pending_work 7\n") || body.starts_with("fkst_pending_work 7\n"),
            "pending gauge must read 7 in:\n{body}"
        );
    }
}
