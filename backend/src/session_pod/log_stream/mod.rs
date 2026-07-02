//! Session pod log-streaming (log-streaming Wave 1).
//!
//! A session pod's stdout/stderr is a stream of secrets waiting to happen: the
//! injected App token, the LLM key, per-user env values, and whatever a subprocess
//! (git, codex) chooses to echo. Before ANY of that leaves the pod boundary it must
//! pass through the [`redact`] redactor, which carries the hard no-leak guarantee.
//!
//! This module is the seam for that machinery. [`redact`] is the pure, exhaustively
//! tested redactor LIBRARY; the effectful fan-out that wires it onto a live pod's
//! log stream is a later wave and is deliberately absent here.

pub mod redact;
