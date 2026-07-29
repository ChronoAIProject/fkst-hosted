//! Structured display cards projected from a tool's own result.
//!
//! A data-heavy answer is the case prose serves worst: an environment profile's install
//! commands, a session's pull requests, a run's log files. The model can describe them,
//! but a description is lossy, unsortable, and — the part that matters — UNVERIFIABLE,
//! because it is generated text. So the SPA renders the DATA, and the prose stays an
//! explanation of it.
//!
//! Two rules make that safe, and they are the same rules `SessionRef` already follows:
//!
//! 1. **A card is projected from the tool RESULT, never from the model's prose.** The
//!    model chooses which tool to call; it does not get to author what the card says. A
//!    card that links somewhere, or claims a PR merged, must not be steerable by
//!    generated text.
//! 2. **Only a 200 projects.** A 403 or a 504 is an explanation the model gives in
//!    words; rendering a half-empty card from an error body would imply the lookup
//!    worked.
//!
//! Projection is deliberately total and lossy: unknown shapes yield `None` rather than a
//! partially-filled card, because a card with blank fields reads as "there is nothing
//! there" when the truth is "this build does not understand that payload".

use serde::Serialize;
use utoipa::ToSchema;

/// Bound on rows in any one card. A card is a summary the user scans before deciding
/// where to click; past this the dashboard is the better surface, and the card says so.
const MAX_ROWS: usize = 12;

/// One `NAME=value` pair on an environment card.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct CardVariable {
    pub key: String,
    pub value: String,
}

/// One saved environment profile in the list card.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct EnvironmentSummaryCard {
    pub name: String,
    pub status: String,
    pub validated_at: String,
    pub install_command_count: u32,
    pub variable_count: u32,
    pub secret_count: u32,
}

/// One pull request in an outcomes card.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct PullRequestCard {
    pub number: i64,
    pub title: String,
    pub html_url: String,
    /// `open` or `closed`.
    pub state: String,
    pub merged: bool,
    pub work_issue: Option<i64>,
    pub files_changed: u32,
}

/// One log run in a runs card.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct LogRunCard {
    pub run_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
}

/// One file in a log-manifest card.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct LogFileCard {
    pub path: String,
    pub size_bytes: u64,
}

/// A structured rendering of one tool result.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataCard {
    /// The caller's saved environment profiles.
    Environments {
        profiles: Vec<EnvironmentSummaryCard>,
        /// Rows beyond [`MAX_ROWS`], so the card can say what it is not showing rather
        /// than silently truncating.
        omitted: u32,
    },
    /// One environment profile in full. Secret NAMES only — the values are write-only
    /// and the endpoint never returns them.
    EnvironmentDetail {
        name: String,
        status: String,
        validated_at: String,
        install: Vec<String>,
        variables: Vec<CardVariable>,
        secret_keys: Vec<String>,
    },
    /// What one session actually shipped.
    Outcomes {
        owner: String,
        name: String,
        trigger_issue: i64,
        pull_requests: Vec<PullRequestCard>,
        merged: u32,
        omitted: u32,
    },
    /// A session's log runs, newest first.
    LogRuns {
        session_id: String,
        runs: Vec<LogRunCard>,
        omitted: u32,
    },
    /// The files in one log run.
    LogManifest {
        session_id: String,
        run: Option<String>,
        files: Vec<LogFileCard>,
        omitted: u32,
    },
}

/// Take at most [`MAX_ROWS`] rows, reporting how many were dropped.
fn bounded<T>(mut rows: Vec<T>) -> (Vec<T>, u32) {
    let omitted = rows.len().saturating_sub(MAX_ROWS);
    rows.truncate(MAX_ROWS);
    (rows, u32::try_from(omitted).unwrap_or(u32::MAX))
}

fn string_at(value: &serde_json::Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

fn u32_at(value: &serde_json::Value, key: &str) -> u32 {
    u32::try_from(value[key].as_u64().unwrap_or(0)).unwrap_or(u32::MAX)
}

/// Project a tool result into a card, or `None` when there is nothing renderable.
///
/// `args` is read only for values the RESPONSE does not carry (a session id the endpoint
/// does not echo). Everything the response states wins, so a card always describes what
/// the server returned rather than what the model asked for.
pub fn project(
    tool_name: &str,
    args: &serde_json::Value,
    result: &serde_json::Value,
) -> Option<DataCard> {
    // Only a success projects: a card built from an error body would imply the lookup
    // worked. The status lives beside the body in every dispatch-backed tool result.
    if result["status"].as_u64() != Some(200) {
        return None;
    }
    let body = &result["body"];

    match tool_name {
        "list_environment_profiles" => {
            let (profiles, omitted) = bounded(
                body["environment_profiles"]
                    .as_array()?
                    .iter()
                    .map(|entry| EnvironmentSummaryCard {
                        name: string_at(entry, "name"),
                        status: string_at(entry, "status"),
                        validated_at: string_at(entry, "validated_at"),
                        install_command_count: u32_at(entry, "install_command_count"),
                        variable_count: u32_at(entry, "variable_count"),
                        secret_count: u32_at(entry, "secret_count"),
                    })
                    .collect(),
            );
            // An empty list is a real, useful answer ("you have none yet"), so it still
            // renders — unlike an unparseable shape, which yields None above.
            Some(DataCard::Environments { profiles, omitted })
        }

        "get_environment_profile" => {
            let name = body["name"].as_str()?.to_string();
            let variables = body["variables"]
                .as_object()
                .map(|map| {
                    map.iter()
                        .map(|(key, value)| CardVariable {
                            key: key.clone(),
                            value: value.as_str().unwrap_or_default().to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(DataCard::EnvironmentDetail {
                name,
                status: string_at(body, "status"),
                validated_at: string_at(body, "validated_at"),
                install: body["install"]
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| row.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                variables,
                secret_keys: body["secret_keys"]
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| row.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        }

        "get_session_outcomes" => {
            let prs = body["prs"].as_array()?;
            let merged = u32::try_from(prs.iter().filter(|pr| pr["merged"] == true).count())
                .unwrap_or(u32::MAX);
            let (pull_requests, omitted) = bounded(
                prs.iter()
                    .map(|pr| PullRequestCard {
                        number: pr["number"].as_i64().unwrap_or_default(),
                        title: string_at(pr, "title"),
                        html_url: string_at(pr, "html_url"),
                        state: string_at(pr, "state"),
                        merged: pr["merged"].as_bool().unwrap_or(false),
                        work_issue: pr["work_issue"].as_i64(),
                        files_changed: u32::try_from(
                            pr["files"].as_array().map(Vec::len).unwrap_or(0),
                        )
                        .unwrap_or(u32::MAX),
                    })
                    .collect(),
            );
            Some(DataCard::Outcomes {
                owner: string_at(body, "owner"),
                name: string_at(body, "name"),
                trigger_issue: body["trigger_issue"].as_i64().unwrap_or_default(),
                pull_requests,
                merged,
                omitted,
            })
        }

        "list_log_runs" => {
            let (runs, omitted) = bounded(
                body["runs"]
                    .as_array()?
                    .iter()
                    .map(|run| LogRunCard {
                        run_id: string_at(run, "run_id"),
                        started_at: string_at(run, "started_at"),
                        ended_at: run["ended_at"].as_str().map(str::to_string),
                    })
                    .collect(),
            );
            Some(DataCard::LogRuns {
                // The runs endpoint does not echo the session id, so it comes from the
                // argument — the one value a response cannot supply.
                session_id: string_at(args, "session_id"),
                runs,
                omitted,
            })
        }

        "get_log_manifest" => {
            let (files, omitted) = bounded(
                body["files"]
                    .as_array()?
                    .iter()
                    .map(|file| LogFileCard {
                        path: string_at(file, "path"),
                        size_bytes: file["size_bytes"].as_u64().unwrap_or_default(),
                    })
                    .collect(),
            );
            Some(DataCard::LogManifest {
                session_id: string_at(args, "session_id"),
                run: body["run"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| args["run"].as_str().map(str::to_string)),
                files,
                omitted,
            })
        }

        // Every other tool answers in prose, or already contributes a `SessionRef`.
        _ => None,
    }
}

#[cfg(test)]
#[path = "cards_tests.rs"]
mod tests;
