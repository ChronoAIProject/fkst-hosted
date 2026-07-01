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
pub mod section_parse;
pub mod trigger_parse;

pub use issue_parse::{parse_goal_issue_body, ParsedGoal};
