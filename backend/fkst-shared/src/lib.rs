//! fkst-shared: role-neutral types and transport clients shared by both the
//! control-plane and the worker deployables.
//!
//! This crate is the structural foundation of the control-plane/worker split
//! (issue #145): code that BOTH roles need lives here so neither role has to
//! depend on the other. It is intentionally free of the `mongodb` driver and
//! of `axum` — the worker links this crate, and keeping those out here is what
//! makes "the worker never touches the database" a compiler-enforced fact.
//!
//! Modules are populated by the extraction commit; at scaffolding time the
//! crate is intentionally empty.
