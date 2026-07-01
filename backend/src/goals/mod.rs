//! Goal-issue parsing + lifecycle labels — the only goal-domain pieces v1 keeps.
//!
//! `issue_parse` turns a triggering issue body into the goal prompt + package
//! list the webhook builds a session from; `trigger_parse` is the Model B
//! counterpart that turns an `fkst-substrate-trigger` issue body into a launch
//! spec; `section_parse` holds the `### `-heading section-splitting skeleton both
//! parsers share; `labels` is the issue lifecycle-label vocabulary the trigger +
//! watcher use. (There is no goal store or goal API in v1 — a GitHub issue IS the
//! unit of work.)

pub mod issue_parse;
pub mod labels;
// The Model B `owner/repo@ref:path` package-reference grammar (issue #359,
// "packages are FETCHED"): the single source of truth for the ref shape shared by
// the trigger parser / launcher (writer) and the `run-substrate` entrypoint
// (reader).
pub mod package_ref;
pub mod section_parse;
pub mod trigger_parse;

pub use issue_parse::{parse_goal_issue_body, ParsedGoal};
pub use package_ref::{package_name_from_path, parse_package_ref, PackageRef};
