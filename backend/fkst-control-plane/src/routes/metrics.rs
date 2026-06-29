//! `GET /metrics`: hand-rendered Prometheus text exposition for the control
//! plane (issue #144).
//!
//! The control plane is API-only and datastore-free, so there is nothing to
//! scrape from a DB and no claim/worker authority to report. These gauges
//! reflect the in-memory session store:
//! - `fkst_sessions_total`   — sessions currently tracked by the store.
//! - `fkst_sessions_pending` — of those, the ones in the `Pending` state
//!   (recorded but not yet run; pod-per-session execution lands in milestone #9).
//!
//! Rendered by hand (no client library): the surface is two gauges, so a
//! dependency would be unjustified. The body follows the Prometheus text format
//! exactly — a `# HELP` line, a `# TYPE <name> gauge` line, then the sample —
//! per metric.
//!
//! Unauthenticated, like `/health`: it carries NO secret (only counts) and is
//! served on the ClusterIP-only surface, so Prometheus can scrape it without a
//! NyxID identity.

use axum::extract::State;
use axum::response::IntoResponse;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

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
fn render_metrics(sessions_total: u64, sessions_pending: u64) -> String {
    let gauges = [
        Gauge {
            name: "fkst_sessions_total",
            help: "Sessions currently tracked by the in-memory session store.",
            value: sessions_total,
        },
        Gauge {
            name: "fkst_sessions_pending",
            help: "Tracked sessions in the pending state (recorded, not yet run).",
            value: sessions_pending,
        },
    ];
    let mut out = String::new();
    for gauge in &gauges {
        render_gauge(&mut out, gauge);
    }
    out
}

/// `GET /metrics`: the control plane's Prometheus gauges. Unauthenticated.
#[utoipa::path(
    get,
    path = "/metrics",
    tag = "system",
    operation_id = "metrics",
    responses(
        (
            status = 200,
            description = "Prometheus text exposition (version 0.0.4) of the controller's live gauges",
            content_type = "text/plain",
            body = String
        )
    )
)]
async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    // Counts from the in-memory session store: total tracked sessions and, of
    // those, the ones still pending (recorded but not yet run).
    let snapshot = state.sessions.repo().snapshot().await;
    let sessions_total = snapshot.len() as u64;
    let sessions_pending = snapshot
        .iter()
        .filter(|s| matches!(s.status, crate::models::SessionStatus::Pending))
        .count() as u64;

    let body = render_metrics(sessions_total, sessions_pending);
    (
        [(axum::http::header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        body,
    )
}

/// `/metrics` route, mounted at the TOP level (unauthenticated, like `/health`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(metrics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_prometheus_text_for_each_gauge() {
        let body = render_metrics(2, 1);
        // Each gauge has its HELP, TYPE, and sample line in the exact format.
        for (name, value) in [("fkst_sessions_total", 2), ("fkst_sessions_pending", 1)] {
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
        let body = render_metrics(7, 0);
        assert!(
            body.contains("\nfkst_sessions_total 7\n")
                || body.starts_with("fkst_sessions_total 7\n"),
            "sessions-total gauge must read 7 in:\n{body}"
        );
    }
}
