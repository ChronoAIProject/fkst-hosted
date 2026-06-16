//! Goals domain: data models, validation, and MongoDB persistence for the
//! `goals` collection.
//!
//! A *goal* captures the user's intent (a prompt the engine will eventually
//! work on), the set of fkst packages to run it with, and an optional target
//! GitHub repo. This module owns the `goals` collection shape and its
//! validation gate. The trigger issue consumes [`GoalDoc`] verbatim (esp.
//! [`GoalStatus`], [`active_session_id`](GoalDoc::active_session_id),
//! [`package_names`](GoalDoc::package_names), [`repo`](GoalDoc::repo)) and
//! adds CAS transitions — keep the enum wire strings and field names exactly
//! as specified.

pub mod issue_parse;
pub mod issue_store;
pub mod marker;
pub mod model;
pub mod repo_create;
mod repo_create_classify;

pub use issue_parse::{parse_goal_issue_body, ParsedGoal};
pub use issue_store::{GoalIssueStore, GoalPatch};
pub use model::{
    validate_goal_fields, GoalDoc, GoalStatus, RepoRef, MAX_GOAL_DESCRIPTION_BYTES,
    MAX_GOAL_PACKAGES, MAX_GOAL_TITLE_CHARS, MAX_PACKAGE_NAME_BYTES,
};
pub use repo_create::{CreateRepoError, CreateRepoSpec};
