//! Ornn role-neutral types (issue #145 extraction).
//!
//! Only the Ornn DTOs + user-facing pin types live in the shared crate — both
//! roles persist `OrnnSkillPin`s on the session document. The `AppError`-coupled
//! registry client and the on-disk injector remain control-plane code (they
//! move to the worker with engine execution in a later database-free issue).

pub mod types;

pub use types::{OrnnPinKind, OrnnSkillPin, ResolvedNode};
