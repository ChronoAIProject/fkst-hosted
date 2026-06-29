//! Ornn injection wire types.
//!
//! The controller<->worker protocol (issue #134) was removed with the worker
//! deployable in the single-crate restructure; what survives is the resolved
//! Ornn injection plan ([`OrnnPlan`] / [`OrnnSkillRef`] / [`OrnnSource`]) that
//! the skill resolver produces and the session runner consumes. Pod-per-session
//! execution carries its work via [`crate::session_spec::SessionSpec`] instead.

pub mod types;

pub use types::{OrnnPlan, OrnnSkillRef, OrnnSource};
