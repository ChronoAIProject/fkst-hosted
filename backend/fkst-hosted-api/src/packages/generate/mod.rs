//! LLM package generation: turn a natural-language description into a
//! validated fkst package draft, optionally dry-run engine conformance.
//!
//! The deliverable is the DRAFT, so a malformed model response or a draft that
//! fails validation/conformance is a normal 200 report (`validation.ok=false`
//! / `conformance.status="skipped"|"failed"`), NEVER a 5xx. Only a gateway
//! failure (the LLM is unreachable) bubbles up as a 503 (`AppError::Unavailable`).
//!
//! Logging discipline (security): only byte sizes, file counts, attempt count,
//! and the conformance status are logged — NEVER the user's description, the
//! prompt, the raw model output, or any file content.

pub mod prompt;

use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::engine::error::RunnerError;
use crate::engine::materialize::{materialize_package, write_fkst_env};
use crate::engine::process::run_conformance;
use crate::engine::{EngineConfig, PreparedPackage};
use crate::error::AppError;
use crate::llm::LlmGateway;
use crate::packages::model::{NewPackage, PackageFile};

use prompt::{user_prompt, SYSTEM_PROMPT};

/// Max generation attempts: one initial call plus one validation-feedback retry.
const MAX_ATTEMPTS: u32 = 2;

/// Hard cap on the conformance dry-run regardless of the engine's own
/// `conformance_timeout_secs`: generation runs inside an HTTP request, so the
/// dry-run must leave budget for the rest of the response.
const CONFORMANCE_CAP_SECS: u64 = 20;

// ---- Response DTOs ----------------------------------------------------------

/// The full generation report (always 200 when generation ran).
#[derive(Debug, Serialize)]
pub struct GenerateReport {
    pub package: PackageDraft,
    pub validation: ValidationReport,
    pub conformance: ConformanceReport,
    pub saved: bool,
    pub save_error: Option<String>,
    pub attempts: u32,
}

/// The generated package draft.
#[derive(Debug, Serialize)]
pub struct PackageDraft {
    pub name: String,
    pub files: Vec<PackageFile>,
    pub composed_deps: Vec<String>,
}

/// Outcome of the `NewPackage::validate` gate over the draft.
#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub ok: bool,
    pub errors: Vec<String>,
}

/// Outcome of the optional engine conformance dry-run.
#[derive(Debug, Serialize)]
pub struct ConformanceReport {
    pub status: ConformanceStatus,
    pub errors: Vec<String>,
    pub skipped_reason: Option<String>,
}

/// Conformance verdict, serialized lowercase to match the API contract.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConformanceStatus {
    Ok,
    Failed,
    Skipped,
}

// ---- Request + strict-parse DTOs --------------------------------------------

/// Resolved generation request (name already resolved + validated by the edge).
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub description: String,
    pub name: String,
    pub save: bool,
    /// Max bytes accepted from a single model completion (config-supplied).
    pub max_output_bytes: usize,
}

/// STRICT parse of the raw model output. Distinct from the forgiving
/// `NewPackage`: unknown keys are rejected so a model that hallucinates extra
/// top-level fields is caught and fed back on retry.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedDraft {
    files: Vec<PackageFile>,
    #[serde(default)]
    composed_deps: Vec<String>,
}

/// Strip ONE optional leading ```` ``` ```` / ```` ```json ```` fence line and a
/// matching trailing ```` ``` ```` fence from `raw`. Only the OUTERMOST single
/// fence is removed (defensive against a model that wraps its JSON despite the
/// "no fences" instruction); a body with no fence is returned untouched.
fn strip_one_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(after_open) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    // Drop the rest of the opening fence line (e.g. ```json -> "json").
    let body = match after_open.split_once('\n') {
        Some((_lang, rest)) => rest,
        None => return trimmed, // a lone "```..." with no newline: not a fence
    };
    // Remove a trailing closing fence if present.
    body.trim_end().strip_suffix("```").unwrap_or(body).trim()
}

// ---- Generation -------------------------------------------------------------

/// Generate a package draft, validate it, and (when it validates) run an engine
/// conformance dry-run within the remaining request budget.
///
/// Returns `Err(AppError::Unavailable)` ONLY when the gateway is unreachable
/// (a 503). Every other outcome — malformed output, validation failure,
/// conformance failure — is a populated `GenerateReport` (a 200).
pub async fn generate_package(
    gateway: &dyn LlmGateway,
    engine: &EngineConfig,
    request_budget: Duration,
    req: &GenerateRequest,
) -> Result<GenerateReport, AppError> {
    let started = Instant::now();

    // The last successfully PARSED draft (files, composed_deps) — surfaced in
    // the report even if it later fails the hard validator, so the caller sees
    // what the model produced.
    let mut last_parsed: Option<(Vec<PackageFile>, Vec<String>)> = None;
    // Validation errors from the most recent attempt, fed back on retry and
    // surfaced when no attempt validates.
    let mut last_errors: Vec<String> = Vec::new();
    // The validated draft, set once an attempt passes the hard validator.
    let mut validated: Option<(Vec<PackageFile>, Vec<String>)> = None;
    let mut success_attempt: u32 = MAX_ATTEMPTS;

    for attempt in 1..=MAX_ATTEMPTS {
        let user = if attempt == 1 {
            user_prompt(&req.description)
        } else {
            format!(
                "{}\n\nYour previous output failed: {}. Return the full corrected JSON object only.",
                user_prompt(&req.description),
                last_errors.join("; ")
            )
        };

        // A gateway error is a 503: the LLM is unreachable, not a draft issue.
        let raw = gateway.complete(SYSTEM_PROMPT, &user).await.map_err(|e| {
            tracing::warn!(attempt, error = %e, "llm gateway call failed");
            AppError::Unavailable("package generation gateway unavailable".into())
        })?;

        if raw.len() > req.max_output_bytes {
            tracing::info!(
                attempt,
                output_bytes = raw.len(),
                max = req.max_output_bytes,
                "generation output exceeded the byte cap"
            );
            last_errors = vec![format!(
                "model output exceeded {} bytes",
                req.max_output_bytes
            )];
            continue;
        }

        let stripped = strip_one_fence(&raw);
        let draft: GeneratedDraft = match serde_json::from_str(stripped) {
            Ok(draft) => draft,
            Err(e) => {
                tracing::info!(attempt, "generation output was not valid JSON");
                last_errors = vec![format!("invalid JSON: {e}")];
                continue;
            }
        };

        last_parsed = Some((draft.files.clone(), draft.composed_deps.clone()));

        let np = NewPackage {
            name: req.name.clone(),
            files: draft.files,
            composed_deps: draft.composed_deps,
        };
        match np.validate() {
            Err(reason) => {
                tracing::info!(
                    attempt,
                    file_count = np.files.len(),
                    "generated draft failed validation"
                );
                last_errors = vec![reason];
                continue;
            }
            Ok(()) => {
                tracing::info!(
                    attempt,
                    file_count = np.files.len(),
                    "generated draft validated"
                );
                validated = Some((np.files, np.composed_deps));
                success_attempt = attempt;
                break;
            }
        }
    }

    // Assemble the report from whichever stage was reached.
    if let Some((files, composed_deps)) = validated {
        let conformance = run_conformance_dry_run(
            engine,
            request_budget,
            started,
            &req.name,
            &files,
            &composed_deps,
        )
        .await;
        Ok(GenerateReport {
            package: PackageDraft {
                name: req.name.clone(),
                files,
                composed_deps,
            },
            validation: ValidationReport {
                ok: true,
                errors: Vec::new(),
            },
            conformance,
            saved: false,
            save_error: None,
            attempts: success_attempt,
        })
    } else {
        // Never validated: surface the last parsed draft (or an empty one) and
        // the accumulated errors; conformance is skipped.
        let (files, composed_deps) = last_parsed.unwrap_or_default();
        Ok(GenerateReport {
            package: PackageDraft {
                name: req.name.clone(),
                files,
                composed_deps,
            },
            validation: ValidationReport {
                ok: false,
                errors: last_errors,
            },
            conformance: ConformanceReport {
                status: ConformanceStatus::Skipped,
                errors: Vec::new(),
                skipped_reason: Some("draft did not validate".to_string()),
            },
            saved: false,
            save_error: None,
            attempts: MAX_ATTEMPTS,
        })
    }
}

/// Convenience builders for a skipped/failed conformance verdict.
fn skipped(reason: impl Into<String>) -> ConformanceReport {
    ConformanceReport {
        status: ConformanceStatus::Skipped,
        errors: Vec::new(),
        skipped_reason: Some(reason.into()),
    }
}

/// Run the engine conformance dry-run over a VALIDATED draft, gating in order:
/// budget, raiser-only structure, binary availability, then the real run. Any
/// host/draft issue degrades to `Skipped`/`Failed` — a draft problem must never
/// surface as a 5xx (the draft is the deliverable).
async fn run_conformance_dry_run(
    engine: &EngineConfig,
    request_budget: Duration,
    started: Instant,
    name: &str,
    files: &[PackageFile],
    composed_deps: &[String],
) -> ConformanceReport {
    // (1) BUDGET: leave room in the request for everything after the dry-run.
    let conf_timeout =
        Duration::from_secs(engine.conformance_timeout_secs.min(CONFORMANCE_CAP_SECS));
    let remaining = request_budget.saturating_sub(started.elapsed());
    if remaining < conf_timeout {
        tracing::info!(
            remaining_ms = remaining.as_millis() as u64,
            "conformance skipped: insufficient request budget"
        );
        return skipped("insufficient request budget for conformance");
    }

    // (2) RAISER-ONLY: the engine's conformance requires a department; a
    // raiser-only package would fail it for a structural, not a quality,
    // reason — report that as Skipped, not Failed.
    let prepared = PreparedPackage {
        package_name: name.to_string(),
        files: files.to_vec(),
        composed_deps: composed_deps.to_vec(),
    };
    if let Err(err) = prepared.validate() {
        let msg = err.to_string();
        if msg.contains("department") {
            return skipped("raiser-only package: engine conformance requires a department");
        }
        return skipped(msg);
    }

    // (3) BINARY: the engine binary must exist and be executable.
    if !binary_is_executable(&engine.framework_bin) {
        tracing::info!("conformance skipped: engine binary unavailable");
        return skipped("engine binary unavailable");
    }

    // (4) RUN: materialize, write the host env, and run conformance. Drop the
    // temp dirs (RAII) on every path.
    let pkg_dir = match materialize_package(&prepared, &engine.temp_root) {
        Ok(dir) => dir,
        Err(e) => return skipped(format!("could not materialize draft: {e}")),
    };
    if let Err(e) = write_fkst_env(
        pkg_dir.path(),
        &engine.candidate_prefix,
        &engine.candidate_from_sep,
    ) {
        return skipped(format!("could not write engine env: {e}"));
    }
    let rt_dir = match tempfile::Builder::new()
        .prefix("fkst-rt-gen-")
        .tempdir_in(&engine.temp_root)
    {
        Ok(dir) => dir,
        Err(e) => return skipped(format!("could not create runtime dir: {e}")),
    };

    // The package-generation dry-run has no per-session env profile; pass an
    // empty map so it runs under the same isolated (#101) env (env_clear + host
    // allow-list + FKST_RUNTIME_ROOT) as every other conformance run.
    let outcome = run_conformance(
        &engine.framework_bin,
        pkg_dir.path(),
        rt_dir.path(),
        conf_timeout,
        engine.error_capture_bytes,
        &std::collections::BTreeMap::new(),
    )
    .await;

    match outcome {
        Ok(()) => {
            tracing::info!("conformance dry-run passed");
            ConformanceReport {
                status: ConformanceStatus::Ok,
                errors: Vec::new(),
                skipped_reason: None,
            }
        }
        Err(RunnerError::ConformanceFailed { stderr, .. }) => {
            tracing::info!("conformance dry-run failed");
            let errors: Vec<String> = stderr
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
            ConformanceReport {
                status: ConformanceStatus::Failed,
                errors,
                skipped_reason: None,
            }
        }
        Err(other) => {
            // A spawn/IO/signal failure is host-side, not a draft verdict:
            // report Skipped with the reason rather than a 5xx.
            tracing::warn!(error = %other, "conformance dry-run could not run");
            skipped(other.to_string())
        }
    }
    // pkg_dir / rt_dir drop here, cleaning their trees by RAII.
}

/// True when `bin` exists and has at least one execute bit set.
fn binary_is_executable(bin: &std::path::Path) -> bool {
    std::fs::metadata(bin)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- strip_one_fence ----------------------------------------------------

    #[test]
    fn strip_one_fence_removes_a_json_fence() {
        let raw = "```json\n{\"files\":[]}\n```";
        assert_eq!(strip_one_fence(raw), "{\"files\":[]}");
    }

    #[test]
    fn strip_one_fence_removes_a_bare_fence() {
        let raw = "```\n{\"files\":[]}\n```";
        assert_eq!(strip_one_fence(raw), "{\"files\":[]}");
    }

    #[test]
    fn strip_one_fence_leaves_unfenced_body_untouched() {
        let raw = "{\"files\":[]}";
        assert_eq!(strip_one_fence(raw), "{\"files\":[]}");
    }

    #[test]
    fn strip_one_fence_trims_surrounding_whitespace() {
        let raw = "  \n```json\n{\"a\":1}\n```\n  ";
        assert_eq!(strip_one_fence(raw), "{\"a\":1}");
    }

    #[test]
    fn strip_one_fence_handles_missing_close_fence() {
        // An opening fence with no closing fence still yields the inner body.
        let raw = "```json\n{\"files\":[]}";
        assert_eq!(strip_one_fence(raw), "{\"files\":[]}");
    }

    #[test]
    fn strip_one_fence_ignores_lone_backtick_run_without_newline() {
        let raw = "```not-a-fence";
        assert_eq!(strip_one_fence(raw), "```not-a-fence");
    }

    // ---- GeneratedDraft strict parse ----------------------------------------

    #[test]
    fn generated_draft_rejects_unknown_top_level_keys() {
        let json = r#"{"files":[{"path":"a.lua","content":"x"}],"surprise":1}"#;
        let err = serde_json::from_str::<GeneratedDraft>(json).expect_err("unknown key rejected");
        assert!(err.to_string().contains("surprise"), "got: {err}");
    }

    #[test]
    fn generated_draft_defaults_composed_deps_to_empty() {
        let json = r#"{"files":[{"path":"a.lua","content":"x"}]}"#;
        let draft: GeneratedDraft = serde_json::from_str(json).expect("parse");
        assert_eq!(draft.files.len(), 1);
        assert!(draft.composed_deps.is_empty());
    }

    #[test]
    fn generated_draft_rejects_unknown_file_keys() {
        // PackageFile denies nothing by itself, but a missing required field
        // still fails — guarding the file subdocument shape.
        let json = r#"{"files":[{"path":"a.lua"}]}"#;
        assert!(serde_json::from_str::<GeneratedDraft>(json).is_err());
    }
}
