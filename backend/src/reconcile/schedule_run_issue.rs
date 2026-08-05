//! The RUN ISSUE: how a due slot becomes a running session.
//!
//! There is no new spawn path and no push channel into a live pod. A due slot
//! creates an ordinary work issue — App-authored, carrying the session's
//! deployment-effective work label, assigned to exactly the session creator — and
//! that issue wakes the session through the gate that already exists
//! ([`crate::reconcile::pending`] → [`crate::reconcile::desired::plan_repo`]). This
//! is what keeps the clock inside the existing control plane with no additional
//! deployable and no standing pod between firings.
//!
//! Routing is verified rather than assumed: work routing never inspects the author
//! ([`crate::reconcile::routing`]) and the configured App is admitted as a system
//! principal ([`crate::reconcile::work_authz`]), so an App-authored, correctly
//! assigned, correctly labelled issue is indistinguishable from a human-filed one.
//!
//! ## Arguments are rendered as escaped data, never as shell text
//!
//! The run issue is how a definition's arguments reach the pod. They are emitted as
//! a fenced TOML block of quoted basic strings, so the pod side parses values
//! rather than interpolating them. An argument that reached a step's argv by string
//! substitution would be a command-injection channel from a public issue body.

use std::collections::BTreeMap;

use k8s_openapi::chrono::{DateTime, SecondsFormat, Utc};

/// The hidden marker that identifies a run issue and binds it to its slot.
///
/// The pod-side workflow runner keys on this to find what to run; the control
/// plane keys on it to recognise a run issue it created and to correlate the run
/// back to its definition.
pub const RUN_ISSUE_MARKER: &str = "fkst-cron-dispatch:v1";

/// Everything the run issue must carry, assembled by the schedule pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunIssueRequest {
    /// The definition issue this run belongs to.
    pub schedule_issue: i64,
    pub workflow_id: String,
    pub slot: DateTime<Utc>,
    pub arguments: BTreeMap<String, String>,
    /// The session's deployment-effective work label — the single label that routes
    /// this issue to the right session.
    pub work_label: String,
    /// The session creator: the sole assignee, which is the routing key.
    pub creator_login: String,
    /// True for a dashboard "run now" rather than a clock firing.
    pub manual: bool,
}

impl RunIssueRequest {
    /// The issue title. Slot-stamped so a run is identifiable at a glance in the
    /// issue list and so two runs of one workflow never share a title.
    pub fn title(&self) -> String {
        format!(
            "[scheduled] {} — {}",
            self.workflow_id,
            timestamp(self.slot)
        )
    }
}

/// Render the run issue's body.
///
/// Pure so the exact bytes the pod reads are unit-testable, and so a change to the
/// contract shows up as a test diff rather than as a production surprise.
pub fn render_run_issue_body(request: &RunIssueRequest) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<!-- {RUN_ISSUE_MARKER} schedule=\"{}\" workflow=\"{}\" slot=\"{}\" manual=\"{}\" -->\n\n",
        request.schedule_issue,
        request.workflow_id,
        timestamp(request.slot),
        request.manual
    ));
    body.push_str("### Scheduled Run\n\n");
    body.push_str(&format!(
        "Workflow `{}`, slot `{}`{}.\n\n",
        request.workflow_id,
        timestamp(request.slot),
        if request.manual {
            ", started manually"
        } else {
            ""
        }
    ));
    body.push_str("### Arguments\n\n");
    if request.arguments.is_empty() {
        body.push_str("_None._\n\n");
    } else {
        body.push_str("```toml\n");
        for (key, value) in &request.arguments {
            body.push_str(&format!("{key} = {}\n", toml_basic_string(value)));
        }
        body.push_str("```\n\n");
    }
    body.push_str(&format!(
        "---\n\n_Created by the fkst control plane for scheduled workflow #{}. \
         Editing or closing this issue does not change the schedule — edit #{} instead._\n",
        request.schedule_issue, request.schedule_issue
    ));
    body
}

/// Quote a value as a TOML basic string, escaping what the format requires.
///
/// The parser is defined by TOML rather than by convention, so a value containing
/// a quote, a backslash, or a newline round-trips as data instead of terminating
/// the string and becoming syntax.
fn toml_basic_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Remaining control characters have no legal literal form in a basic
            // string; TOML requires the \uXXXX escape.
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
#[path = "schedule_run_issue_tests.rs"]
mod tests;
