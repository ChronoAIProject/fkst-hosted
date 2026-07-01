//! fkst-control-plane library crate (formerly fkst-hosted-api).
//!
//! Hosts the hosted backend's public modules (config, error, router, state,
//! routes) so both the binary entrypoint and the integration tests can build
//! the application without a real TCP bind.

pub mod config;
// Named-environment / install-validation config knobs (`FKST_ENV_*`, issue
// #338 §6.1). Config surface only — no behaviour is wired to these yet.
pub mod env_config;
pub mod error;
pub mod github_app;
// GitHub-token identity verification + the `GithubUser` axum extractor (PR4a):
// trades `Authorization: Bearer <github token>` for the verified `{login, id}`
// that keys the per-user environment/secret store.
pub mod github_identity;
pub mod goals;
// Shared in-pod install-command runner + the `validate-env` subcommand (issue
// #338 §3.2/§3.4): runs an environment's ordered install commands and emits a
// machine-readable verdict frame. Reused by the env-validation pod (wired here)
// and, later, the session's pre-agent install step.
pub mod install;
pub mod models;
// Model B reconciler config knobs (`FKST_*`, issue #359 §4). Config surface only
// — no behaviour is wired to these yet (PR5b wires the loop; PR6 flips it on).
pub mod reconcile_config;
// Model B reconciler (issue #359 §4, PR5a core + PR5b wiring): the pure
// desired-state types + event→action planner (`plan_repo`) and trigger-issue →
// registration parse, plus the effectful reconcile loop (reachability pre-flight,
// action executor, per-repo driver, queue + sweep/full-resync loops). Gated on
// `FKST_POD_DISPATCH`; Model A is untouched until the PR6 flip.
pub mod reconcile;
// Reserved-env "keep-module" (Model B PR0, issue #359 §7/§9): holds
// `is_reserved_env_key` + `LLM_ENV_KEY` so they survive the later deletion of
// `engine/` and `sessions/codex_provider/`, which originally defined them.
pub mod reserved_env;
// Runtime OpenAPI 3 document (no static spec): assembled from the live
// `#[utoipa::path]` handlers + `ToSchema` types and served at GET /openapi.json.
pub mod k8s;
pub mod openapi;
pub mod router;
pub mod routes;
// In-pod `run-substrate` subcommand (Model B, issue #359 §5): the long-lived
// substrate-session entrypoint that fetches packages + the target repo, wires the
// rotating GitHub token into git + gh, renders the codex config, and execs
// `fkst-framework supervise`.
pub mod session_pod;
pub mod session_spec;
pub mod state;
