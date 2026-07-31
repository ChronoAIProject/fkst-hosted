//! Repo-local workflow catalog preview.
//!
//! The control plane can inspect JSON files in the repository's conventional
//! `.fkst/packages` root, but it cannot see package built-in Lua workflows. The
//! pod remains the final authority over its effective workflow catalog.

pub mod blueprint;
pub mod reader;

#[cfg(test)]
#[path = "blueprint_tests.rs"]
mod blueprint_tests;

#[cfg(test)]
#[path = "reader_tests.rs"]
mod reader_tests;
