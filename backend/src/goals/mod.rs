//! Goal-issue parsing + label set-math — the goal-domain pieces Model B keeps.
//!
//! `trigger_parse` turns an `fkst-substrate-trigger` issue body into a launch
//! spec; `section_parse` holds the `### `-heading section-splitting skeleton it
//! builds on; `labels` holds the read-then-replace label set-math the reconciler
//! uses; `package_ref` is the fetched-package ref grammar. (There is no goal store
//! or goal API — a GitHub issue IS the unit of work.)

pub mod labels;
// The Model B `owner/repo@ref:path` package-reference grammar (issue #359,
// "packages are FETCHED"): the single source of truth for the ref shape shared by
// the trigger parser / launcher (writer) and the `run-substrate` entrypoint
// (reader).
pub mod package_ref;
pub mod section_parse;
pub mod trigger_parse;

pub use package_ref::{package_name_from_path, parse_package_ref, PackageRef};
