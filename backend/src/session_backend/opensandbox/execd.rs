//! [`ExecdClient`]: a thin reqwest client over the in-sandbox exec daemon (execd),
//! reached THROUGH the OpenSandbox lifecycle proxy (issue #417).
//!
//! execd runs inside a sandbox on a fixed port; the lifecycle service reverse-
//! proxies it at `/v1/sandboxes/{id}/proxy/{port}{execd_path}`. This client owns
//! that URL shape and stamps BOTH the lifecycle API key AND the per-session execd
//! access token on every request via a single choke-point ([`ExecdClient::request`]),
//! reusing the sibling [`lifecycle`] module's `API_KEY_HEADER` + `map_response` so
//! the two clients share one auth header and one status -> error mapping.
//!
//! Secrets discipline: the two secrets (API key + execd token) are exposed at
//! exactly one site (`request`), and NOTHING here logs a token, a file's contents,
//! or a command's output/stream body — only method / path / status (via
//! `map_response`) and, at most, the numeric log cursor.

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use super::dto::{
    CommandRef, CommandStatus, ExecdFileMetadata, FileInfo, OsbError, RunCommandBody,
    ServerStreamFrame,
};
use super::lifecycle;

/// Header carrying the per-session execd access token (derived by
/// [`super::token::derive_execd_token`]).
const EXECD_TOKEN_HEADER: &str = "X-EXECD-ACCESS-TOKEN";

/// The deliberately-wrong token [`ExecdClient::probe_auth_rejection`] sends. A fixed
/// non-secret literal: it can never equal a real token (real tokens are 64 lowercase
/// hex chars of an HMAC).
const PROBE_WRONG_TOKEN: &str = "fkst-auth-probe-invalid";

/// Outcome of the exec-plane auth probe ([`ExecdClient::probe_auth_rejection`]) —
/// a security GATE result, deliberately not folded into [`OsbError`] so an
/// "accepted the wrong token" cannot be mistaken for an ordinary API failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthProbeOutcome {
    /// The daemon rejected the wrong token (401/403) — token auth is enforced.
    Rejected,
    /// The daemon ACCEPTED the wrong token (a 2xx) — the exec plane is
    /// unauthenticated; carrying the offending status for the log line.
    Accepted { status: u16 },
}

/// Budget for `/files/upload`: credential files are small; the proxy hop adds
/// little. Without a budget a wedged connection blocks the calling reconciler verb
/// indefinitely (reqwest's default is NO request timeout).
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// Budget for the quick JSON/plain verbs (`/files/info`, `/command/status/{id}`,
/// `/command/{id}/logs`) and for a `background: true` command launch (execd answers
/// with just the `init` frame and closes the stream).
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Slack added on top of a FOREGROUND command's own execd-side `timeout` to bound
/// the `/command` request: [`ExecdClient::run_command`] reads the BUFFERED SSE body,
/// which lasts the command's lifetime, so the request budget must cover the command
/// plus transport/startup overhead.
const COMMAND_SLACK: Duration = Duration::from_secs(30);

/// Per-verb request budgets, injectable so tests can shrink them to milliseconds
/// (wiremock stall tests must not sleep 30s). Production always uses `default()`.
#[derive(Debug, Clone, Copy)]
pub(super) struct ExecdTimeouts {
    /// Budget for `/files/upload` (see [`UPLOAD_TIMEOUT`]).
    pub(super) upload: Duration,
    /// Budget for the quick verbs + background command launches
    /// (see [`QUERY_TIMEOUT`]).
    pub(super) query: Duration,
    /// Foreground `/command` slack on top of the command's own timeout
    /// (see [`COMMAND_SLACK`]).
    pub(super) command_slack: Duration,
}

impl Default for ExecdTimeouts {
    fn default() -> Self {
        Self {
            upload: UPLOAD_TIMEOUT,
            query: QUERY_TIMEOUT,
            command_slack: COMMAND_SLACK,
        }
    }
}

/// The fixed in-sandbox port execd listens on; the lifecycle service proxies it at
/// `/proxy/{port}`.
const EXECD_PROXY_PORT: u16 = 44772;

/// Response header on `GET /command/{id}/logs` carrying the NEXT tail cursor to poll
/// from (a 0-based line integer).
const TAIL_CURSOR_HEADER: &str = "EXECD-COMMANDS-TAIL-CURSOR";

/// A client bound to one sandbox's execd, reached through the lifecycle proxy.
///
/// `#[derive(Debug)]` is safe: both [`SecretString`] fields redact themselves in
/// `Debug` (asserted by a unit test), so neither secret appears.
#[derive(Debug)]
pub struct ExecdClient {
    base_url: String,
    api_key: SecretString,
    sandbox_id: String,
    execd_token: SecretString,
    http: reqwest::Client,
    timeouts: ExecdTimeouts,
}

impl ExecdClient {
    /// Build a client over the SAME lifecycle base URL the sibling
    /// [`lifecycle::OsbLifecycleClient`] uses (this client owns the
    /// `/v1/sandboxes/{id}/proxy/{port}` prefix); a trailing slash is trimmed so
    /// path joins never double up. Request budgets are the production
    /// [`ExecdTimeouts::default`].
    pub fn new(
        lifecycle_base: reqwest::Url,
        api_key: SecretString,
        sandbox_id: String,
        execd_token: SecretString,
        http: reqwest::Client,
    ) -> Self {
        Self {
            base_url: lifecycle_base.as_str().trim_end_matches('/').to_string(),
            api_key,
            sandbox_id,
            execd_token,
            http,
            timeouts: ExecdTimeouts::default(),
        }
    }

    /// Test-only budget override (milliseconds-scale stall tests).
    #[cfg(test)]
    pub(super) fn with_timeouts(mut self, timeouts: ExecdTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Start a request through the proxy with BOTH auth headers AND the verb's
    /// request budget set. Every verb builds its request here so none can omit a
    /// header or ride without a deliberate budget choice (`None` = unbounded,
    /// reserved for a foreground command with no execd-side timeout). `execd_path`
    /// is the daemon-relative path (e.g. `/files/upload`) — the core execd paths
    /// carry NO `/v1` prefix.
    fn request(
        &self,
        method: reqwest::Method,
        execd_path: &str,
        timeout: Option<Duration>,
    ) -> reqwest::RequestBuilder {
        self.request_with_token(
            method,
            execd_path,
            timeout,
            self.execd_token.expose_secret(),
        )
    }

    /// The actual request construction — the ONLY place either secret is exposed.
    /// Private to the two callers: [`Self::request`] (the real token) and
    /// [`Self::probe_auth_rejection`] (a deliberately WRONG token).
    fn request_with_token(
        &self,
        method: reqwest::Method,
        execd_path: &str,
        timeout: Option<Duration>,
        execd_token: &str,
    ) -> reqwest::RequestBuilder {
        let url = format!(
            "{}/v1/sandboxes/{}/proxy/{}{}",
            self.base_url, self.sandbox_id, EXECD_PROXY_PORT, execd_path
        );
        let mut builder = self.http.request(method, url);
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }
        builder
            .header(lifecycle::API_KEY_HEADER, self.api_key.expose_secret())
            .header(EXECD_TOKEN_HEADER, execd_token)
    }

    /// Security probe: sends `GET /files/info?path=/` through the proxy with a
    /// DELIBERATELY WRONG token and reports whether the daemon REJECTED it.
    ///
    /// WHY: the deployed lifecycle server exempts the proxy route from API-key
    /// auth, so the per-session execd token is the ONLY auth on the exec plane —
    /// a sandbox whose execd accepts a bad token (a template/config regression
    /// dropping the `EXECD_ACCESS_TOKEN` env) is an UNAUTHENTICATED exec surface
    /// on a credential-bearing pod. execd's contract: empty or mismatched header
    /// → 401 (403 tolerated as an equivalent rejection).
    ///
    /// `Ok(Rejected)` = enforcement proven; `Ok(Accepted{status})` = the security
    /// failure (a 2xx for the wrong token); any other status is an ordinary
    /// [`OsbError::Api`] infrastructure failure (NOT a security verdict), and
    /// transport errors propagate as [`OsbError::Transport`].
    pub async fn probe_auth_rejection(&self) -> Result<AuthProbeOutcome, OsbError> {
        let execd_path = "/files/info";
        let method = reqwest::Method::GET;
        let response = self
            .request_with_token(
                method.clone(),
                execd_path,
                Some(self.timeouts.query),
                PROBE_WRONG_TOKEN,
            )
            .query(&[("path", "/")])
            .send()
            .await?;
        let status = response.status();
        tracing::debug!(method = %method, path = %execd_path, status = status.as_u16(), "opensandbox execd auth probe");
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Ok(AuthProbeOutcome::Rejected);
        }
        if status.is_success() {
            return Ok(AuthProbeOutcome::Accepted {
                status: status.as_u16(),
            });
        }
        // Neither a rejection nor an acceptance (404 holder gone, 5xx proxy blip):
        // an ordinary infrastructure failure the caller retries/propagates.
        let message = response.text().await.unwrap_or_default();
        Err(OsbError::Api {
            status: status.as_u16(),
            message,
        })
    }

    /// `POST /files/upload` — write `contents` to `path` inside the sandbox with the
    /// given permission `mode` (REAL bits, e.g. `0o400` for owner-read-only).
    ///
    /// The daemon reads `mode` as `ParseUint(Itoa(mode), 8)`, so the real bits are
    /// converted to the octal-DIGITS integer the wire expects (`0o400` -> `400`).
    /// Two multipart parts, BOTH carrying filenames (the server reads each as a file
    /// part): a JSON `metadata` part and the raw `file` bytes. `owner` / `group` are
    /// not sent (the server no-ops on empty). NEVER logs `contents`.
    pub async fn upload_file(
        &self,
        path: &str,
        contents: &[u8],
        mode: u32,
    ) -> Result<(), OsbError> {
        // Real permission bits -> octal-DIGITS wire integer (0o400 -> 400). The
        // format is infallible; the parse only fails on overflow, so fall back to
        // the raw value rather than silently corrupting the mode.
        let wire_mode = format!("{mode:o}").parse::<u32>().unwrap_or(mode);
        let metadata = ExecdFileMetadata {
            path: path.to_string(),
            mode: wire_mode,
        };
        // Serializing `{path, mode}` cannot fail by construction (no non-string map
        // keys, no fallible custom Serialize), matching the repo's `.expect` on
        // other infallible-by-construction ops.
        let metadata_json =
            serde_json::to_string(&metadata).expect("ExecdFileMetadata serializes to JSON");

        let metadata_part = reqwest::multipart::Part::text(metadata_json)
            .file_name("metadata.json")
            .mime_str("application/json")?;
        let file_part = reqwest::multipart::Part::bytes(contents.to_vec())
            .file_name(basename(path).to_string())
            .mime_str("application/octet-stream")?;
        let form = reqwest::multipart::Form::new()
            .part("metadata", metadata_part)
            .part("file", file_part);

        let path = "/files/upload";
        let method = reqwest::Method::POST;
        let response = self
            .request(method.clone(), path, Some(self.timeouts.upload))
            .multipart(form)
            .send()
            .await?;
        lifecycle::map_response(&method, path, response).await?;
        Ok(())
    }

    /// `GET /files/info?path=…` — the restart-wipe probe: fetch one file's metadata.
    ///
    /// The response is a JSON map keyed by path; this returns the entry for `path`
    /// (falling back to the single/first entry). An empty map, an absent key, or a
    /// `404` all surface as [`OsbError::NotFound`] — i.e. "the file is gone".
    pub async fn file_info(&self, path: &str) -> Result<FileInfo, OsbError> {
        let execd_path = "/files/info";
        let method = reqwest::Method::GET;
        let response = self
            .request(method.clone(), execd_path, Some(self.timeouts.query))
            .query(&[("path", path)])
            .send()
            .await?;
        let response = lifecycle::map_response(&method, execd_path, response).await?;
        let mut map: std::collections::BTreeMap<String, FileInfo> = response.json().await?;
        if let Some(info) = map.remove(path) {
            return Ok(info);
        }
        map.into_values().next().ok_or(OsbError::NotFound)
    }

    /// `POST /command` — launch a command; returns its execd-assigned id.
    ///
    /// The response is a `text/event-stream`; the id lives in the leading `init`
    /// frame's `text` (execd's `OnExecuteInit`). This reads the BUFFERED body (no
    /// streaming loop), takes the first non-empty frame, defensively strips a leading
    /// `data:`, and requires a non-empty `init` frame. A success response without a
    /// usable init frame is an [`OsbError::Api`] carrying the status but NOT the raw
    /// stream body (which can hold command output).
    pub async fn run_command(
        &self,
        command: &str,
        timeout_ms: Option<u64>,
        background: bool,
    ) -> Result<CommandRef, OsbError> {
        let body = RunCommandBody {
            command: command.to_string(),
            timeout: timeout_ms,
            background,
        };
        // The request budget follows the launch mode, because the body below is
        // read BUFFERED (the response lasts until the SSE stream closes):
        // - background: execd answers with just the `init` frame and closes — the
        //   quick-query budget applies.
        // - foreground with an execd-side timeout: the stream lasts the command's
        //   lifetime, so the budget is that timeout + slack (never severs a
        //   legitimately long command).
        // - foreground WITHOUT an execd-side timeout: deliberately unbounded — a
        //   fixed budget here would sever a legitimately long command (no such
        //   call site today; validation always passes a timeout).
        let request_timeout = match (background, timeout_ms) {
            (true, _) => Some(self.timeouts.query),
            (false, Some(ms)) => {
                Some(Duration::from_millis(ms).saturating_add(self.timeouts.command_slack))
            }
            (false, None) => None,
        };
        let path = "/command";
        let method = reqwest::Method::POST;
        let response = self
            .request(method.clone(), path, request_timeout)
            .json(&body)
            .send()
            .await?;
        let response = lifecycle::map_response(&method, path, response).await?;
        let status = response.status().as_u16();
        let stream = response.text().await?;

        // The command id is the leading `init` frame's `text`. Take the first
        // non-empty `\n\n`-delimited frame, defensively strip a leading `data:`,
        // parse it, and require a non-empty `init` frame — all in one flat chain.
        let command_id = stream
            .split("\n\n")
            .map(str::trim)
            .find(|frame| !frame.is_empty())
            .map(|frame| frame.strip_prefix("data:").map(str::trim).unwrap_or(frame))
            .and_then(|frame| serde_json::from_str::<ServerStreamFrame>(frame).ok())
            .filter(|parsed| parsed.r#type == "init")
            .and_then(|parsed| parsed.text)
            .filter(|id| !id.is_empty());

        match command_id {
            Some(id) => Ok(CommandRef { id }),
            // Do NOT echo `stream` into the error: it can contain command output.
            None => Err(OsbError::Api {
                status,
                message: "opensandbox execd: command response had no usable init frame".to_string(),
            }),
        }
    }

    /// `GET /command/status/{id}` — poll a launched command's status.
    pub async fn command_status(&self, id: &str) -> Result<CommandStatus, OsbError> {
        let path = format!("/command/status/{id}");
        let method = reqwest::Method::GET;
        let response = self
            .request(method.clone(), &path, Some(self.timeouts.query))
            .send()
            .await?;
        let response = lifecycle::map_response(&method, &path, response).await?;
        Ok(response.json::<CommandStatus>().await?)
    }

    /// `GET /command/{id}/logs?cursor=…` — fetch the log tail from `cursor`.
    ///
    /// Returns `(body, next_cursor)`: the raw log text and the NEXT cursor to poll
    /// from, read off the [`TAIL_CURSOR_HEADER`]. A missing/unparseable header reuses
    /// the passed `cursor` (logged at debug). NEVER logs the body.
    pub async fn command_logs(&self, id: &str, cursor: u64) -> Result<(String, u64), OsbError> {
        let path = format!("/command/{id}/logs");
        let method = reqwest::Method::GET;
        let response = self
            .request(method.clone(), &path, Some(self.timeouts.query))
            .query(&[("cursor", cursor.to_string())])
            .send()
            .await?;
        let response = lifecycle::map_response(&method, &path, response).await?;
        let next_cursor = response
            .headers()
            .get(TAIL_CURSOR_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.parse::<u64>().ok())
            .unwrap_or_else(|| {
                tracing::debug!(
                    cursor,
                    "execd logs response missing/unparseable tail cursor header; reusing input cursor"
                );
                cursor
            });
        let body = response.text().await?;
        Ok((body, next_cursor))
    }
}

/// The final path segment of a unix-style path, used as the multipart file part's
/// filename. Falls back to `"file"` when `path` has no non-empty final segment.
fn basename(path: &str) -> &str {
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("file")
}

#[cfg(test)]
#[path = "execd_tests.rs"]
mod tests;
