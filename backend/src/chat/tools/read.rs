//! The read-only tools: the concierge's entire view of live data.
//!
//! Each one maps its arguments to an encoded `/api/v1/...` path and dispatches a
//! GET through the real router as the calling user (see
//! [`crate::chat::dispatch`]). The response — INCLUDING a 4xx/5xx status — is
//! returned as data so the model can explain what happened truthfully rather than
//! guessing or retrying.
//!
//! Tool descriptions are written for the model to read: each says what the tool
//! answers and how it behaves when the user lacks access, because those two facts
//! are what the model needs to route a question correctly.

use std::sync::Arc;

use async_trait::async_trait;

use super::super::llm::ToolDef;
use super::{
    encode, optional_clamped_u64, optional_str, required_i64, required_str, ChatTool, ToolCtx,
    ToolError, ToolOutcome, ToolRegistry,
};

/// Default `tail_bytes` for [`TailLogFile`]: enough to hold the tail of a failing
/// run without dominating the model's context.
const DEFAULT_TAIL_BYTES: u64 = 16 * 1024;
/// Ceiling for `tail_bytes`. Above this the dispatch-level truncation would cut the
/// payload anyway, so clamping here keeps the answer honest.
const MAX_TAIL_BYTES: u64 = 64 * 1024;
/// Bounds for `observe_session`'s `limit`, matching the endpoint's own range.
const MIN_OBSERVE_LIMIT: u64 = 1;
const MAX_OBSERVE_LIMIT: u64 = 10_000;

/// An empty JSON-Schema parameter object for tools that take no arguments.
fn no_params() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

/// Build a JSON-Schema object from named properties plus a required list.
fn params(properties: serde_json::Value, required: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// Dispatch a GET and shape the result for the model.
///
/// `result_json` always carries BOTH the status and the body, so the model never has
/// to infer success from the body's shape.
async fn dispatch_get(
    ctx: &ToolCtx,
    path_and_query: &str,
    forward_broader: bool,
) -> Result<ToolOutcome, ToolError> {
    let broader = if forward_broader {
        ctx.broader.as_ref()
    } else {
        None
    };
    let response = ctx
        .dispatch
        .get(path_and_query, &ctx.bearer, broader)
        .await?;
    Ok(ToolOutcome {
        result_json: serde_json::json!({
            "status": response.status,
            "body": response.body,
        }),
        truncated: response.truncated,
        status: Some(response.status),
    })
}

// ---- get_overview ---------------------------------------------------------

struct GetOverview;

#[async_trait]
impl ChatTool for GetOverview {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_overview".to_string(),
            description: "List every GitHub account and repository this user can see, with each \
                 repository's fkst App installation status and its live session and package \
                 counts. Start here when the user asks what they have, which repos are \
                 connected, or where their sessions are running. Shows only what this user is \
                 authorized to see."
                .to_string(),
            parameters: no_params(),
        }
    }

    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: serde_json::Value,
    ) -> Result<ToolOutcome, ToolError> {
        // The only tool that forwards the broader-visibility token, because
        // /overview is the only endpoint that honors it — so the concierge sees
        // exactly the repository set the dashboard shows this user.
        dispatch_get(ctx, "/api/v1/overview", true).await
    }
}

// ---- list_repo_sessions ---------------------------------------------------

struct ListRepoSessions;

#[async_trait]
impl ChatTool for ListRepoSessions {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_repo_sessions".to_string(),
            description:
                "List every substrate session on one repository — each session's trigger issue, \
                 name, status labels, branches, work items and session id. Use this for \
                 \"what is running on owner/repo\", \"did my session start\", or to find a \
                 session id needed by the log and observe tools. Returns 403 or 404 when the \
                 user cannot see that repository."
                    .to_string(),
            parameters: params(
                serde_json::json!({
                    "owner": { "type": "string", "description": "Repository owner (user or org login)." },
                    "name": { "type": "string", "description": "Repository name." },
                }),
                &["owner", "name"],
            ),
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<ToolOutcome, ToolError> {
        let owner = required_str(&args, "owner")?;
        let name = required_str(&args, "name")?;
        let path = format!(
            "/api/v1/repos/{}/{}/sessions",
            encode(&owner),
            encode(&name)
        );
        dispatch_get(ctx, &path, false).await
    }
}

// ---- observe_session -----------------------------------------------------

struct ObserveSession;

#[async_trait]
impl ChatTool for ObserveSession {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "observe_session".to_string(),
            description: "Read a running session's live engine read-model: its queues and recent \
                 deliveries. Use it to explain what a session is doing RIGHT NOW. Needs a \
                 session_id from list_repo_sessions. Returns 404 when no runtime exists (the \
                 session is idle or retired), 409 when the session's packages declare no \
                 observable subscriptions, and 403 when the user lacks access."
                .to_string(),
            parameters: params(
                serde_json::json!({
                    "session_id": { "type": "string", "description": "Session id from list_repo_sessions." },
                    "limit": {
                        "type": "integer",
                        "description": "Max deliveries to return (1-10000).",
                        "minimum": MIN_OBSERVE_LIMIT,
                        "maximum": MAX_OBSERVE_LIMIT,
                    },
                }),
                &["session_id"],
            ),
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<ToolOutcome, ToolError> {
        let session_id = required_str(&args, "session_id")?;
        let limit = optional_clamped_u64(&args, "limit", MIN_OBSERVE_LIMIT, MAX_OBSERVE_LIMIT)?;
        let mut path = format!("/api/v1/sessions/{}/observe", encode(&session_id));
        if let Some(limit) = limit {
            path.push_str(&format!("?limit={limit}"));
        }
        dispatch_get(ctx, &path, false).await
    }
}

// ---- list_log_runs -------------------------------------------------------

struct ListLogRuns;

#[async_trait]
impl ChatTool for ListLogRuns {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_log_runs".to_string(),
            description:
                "List a session's log runs, newest first, with their start and end times. Call \
                 this before reading log files so you know which run to read. Log access is \
                 deny-by-default: a user who is not the session creator, a listed log-access \
                 principal, or a deployment admin gets 403."
                    .to_string(),
            parameters: params(
                serde_json::json!({
                    "session_id": { "type": "string", "description": "Session id from list_repo_sessions." },
                }),
                &["session_id"],
            ),
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<ToolOutcome, ToolError> {
        let session_id = required_str(&args, "session_id")?;
        let path = format!("/api/v1/logs/{}/runs", encode(&session_id));
        dispatch_get(ctx, &path, false).await
    }
}

// ---- get_log_manifest ----------------------------------------------------

struct GetLogManifest;

#[async_trait]
impl ChatTool for GetLogManifest {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_log_manifest".to_string(),
            description:
                "List the log files in one session run, with their sizes — the index you pick a \
                 path from before calling tail_log_file. Omit `run` for the latest run. Same \
                 deny-by-default log access as list_log_runs."
                    .to_string(),
            parameters: params(
                serde_json::json!({
                    "session_id": { "type": "string", "description": "Session id from list_repo_sessions." },
                    "run": { "type": "string", "description": "Run id from list_log_runs; omit for the latest run." },
                }),
                &["session_id"],
            ),
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<ToolOutcome, ToolError> {
        let session_id = required_str(&args, "session_id")?;
        let run = optional_str(&args, "run")?;
        let mut path = format!("/api/v1/logs/{}/manifest", encode(&session_id));
        if let Some(run) = run {
            path.push_str(&format!("?run={}", encode(&run)));
        }
        dispatch_get(ctx, &path, false).await
    }
}

// ---- tail_log_file -------------------------------------------------------

struct TailLogFile;

#[async_trait]
impl ChatTool for TailLogFile {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "tail_log_file".to_string(),
            description:
                "Read the tail of one log file from a session run — the tool for \"why did my \
                 session fail?\". Take `path` from get_log_manifest. Log content is redacted \
                 upstream (secrets appear as redaction markers). Same deny-by-default log \
                 access as list_log_runs."
                    .to_string(),
            parameters: params(
                serde_json::json!({
                    "session_id": { "type": "string", "description": "Session id from list_repo_sessions." },
                    "path": { "type": "string", "description": "File path exactly as listed by get_log_manifest." },
                    "tail_bytes": {
                        "type": "integer",
                        "description": "Bytes to read from the end of the file (default 16384, max 65536).",
                        "minimum": 1,
                        "maximum": MAX_TAIL_BYTES,
                    },
                    "run": { "type": "string", "description": "Run id from list_log_runs; omit for the latest run." },
                }),
                &["session_id", "path"],
            ),
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<ToolOutcome, ToolError> {
        let session_id = required_str(&args, "session_id")?;
        let file_path = required_str(&args, "path")?;
        let tail_bytes = optional_clamped_u64(&args, "tail_bytes", 1, MAX_TAIL_BYTES)?
            .unwrap_or(DEFAULT_TAIL_BYTES);
        let run = optional_str(&args, "run")?;
        let mut path = format!(
            "/api/v1/logs/{}/file?path={}&tail_bytes={}",
            encode(&session_id),
            encode(&file_path),
            tail_bytes
        );
        if let Some(run) = run {
            path.push_str(&format!("&run={}", encode(&run)));
        }
        dispatch_get(ctx, &path, false).await
    }
}

// ---- get_session_outcomes ------------------------------------------------

struct GetSessionOutcomes;

#[async_trait]
impl ChatTool for GetSessionOutcomes {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_session_outcomes".to_string(),
            description:
                "List what one session actually produced: its work items, pull requests and \
                 their merge state. Use it for \"what has my session shipped?\". Identify the \
                 session by its repository plus its TRIGGER issue number (from \
                 list_repo_sessions), not by session id."
                    .to_string(),
            parameters: params(
                serde_json::json!({
                    "owner": { "type": "string", "description": "Repository owner (user or org login)." },
                    "name": { "type": "string", "description": "Repository name." },
                    "issue_number": { "type": "integer", "description": "The session's trigger issue number." },
                }),
                &["owner", "name", "issue_number"],
            ),
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<ToolOutcome, ToolError> {
        let owner = required_str(&args, "owner")?;
        let name = required_str(&args, "name")?;
        let issue_number = required_i64(&args, "issue_number")?;
        let path = format!(
            "/api/v1/repos/{}/{}/sessions/{}/outcomes",
            encode(&owner),
            encode(&name),
            issue_number
        );
        dispatch_get(ctx, &path, false).await
    }
}

// ---- list_environment_profiles -------------------------------------------

struct ListEnvironmentProfiles;

#[async_trait]
impl ChatTool for ListEnvironmentProfiles {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_environment_profiles".to_string(),
            description:
                "List this user's saved named environment profiles (their names, validation \
                 state and non-secret metadata) — the profiles a trigger issue's Environment \
                 section can reference. Secret VALUES are write-only and never returned. \
                 Returns 503 when the environment store is unavailable."
                    .to_string(),
            parameters: no_params(),
        }
    }

    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: serde_json::Value,
    ) -> Result<ToolOutcome, ToolError> {
        dispatch_get(ctx, "/api/v1/users/me/environment-profiles", false).await
    }
}

/// Register every read-only tool. Order is the order the model sees them in, so the
/// broad "what do I have" tools come first and the narrow log reads last.
pub(super) fn register(registry: &mut ToolRegistry) {
    registry.register(Arc::new(GetOverview));
    registry.register(Arc::new(ListRepoSessions));
    registry.register(Arc::new(GetSessionOutcomes));
    registry.register(Arc::new(ObserveSession));
    registry.register(Arc::new(ListLogRuns));
    registry.register(Arc::new(GetLogManifest));
    registry.register(Arc::new(TailLogFile));
    registry.register(Arc::new(ListEnvironmentProfiles));
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
