//! Per-session helpers reused by the in-pod `run-session` runner: the codex
//! provider config renderer. (There is no in-memory session store or driver in
//! v1 — a session is the Kubernetes Job; see `routes/sessions.rs` for the
//! K8s-backed read/stop surface.)

pub mod codex_provider;
