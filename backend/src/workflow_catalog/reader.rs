//! Repository Contents reader for the conventional `.fkst/packages` root.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use secrecy::SecretString;

use crate::error::AppError;
use crate::github_app::api::{GithubApi, RepoEntryKind};
use crate::models::RepoRef;

use super::blueprint::{parse_blueprint, WorkflowBlueprint};

const CATALOG_ROOT: &str = ".fkst/packages";
const MAX_BLUEPRINT_BYTES: u64 = 262_144;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogProblem {
    pub path: String,
    pub detail: String,
}

/// A control-plane preview of repo-local workflows at the conventional default
/// root. Package built-ins and any pod-side root override remain outside this view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogView {
    pub workflows: Vec<WorkflowBlueprint>,
    pub rejected: Vec<CatalogProblem>,
    pub disqualified_ids: Vec<String>,
}

pub async fn read_repo_catalog(
    api: &dyn GithubApi,
    token: &SecretString,
    repo: &RepoRef,
    git_ref: &str,
) -> Result<CatalogView, AppError> {
    let mut entries = api
        .list_dir(token, &repo.owner, &repo.name, CATALOG_ROOT, git_ref)
        .await?;
    entries.retain(|entry| {
        entry.kind == RepoEntryKind::File && is_root_json_file(entry.path.as_str())
    });
    entries.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));

    let mut workflows = Vec::new();
    let mut rejected = Vec::new();
    for entry in entries {
        if entry.size > MAX_BLUEPRINT_BYTES {
            rejected.push(problem(
                entry.path,
                format!("file exceeds the {MAX_BLUEPRINT_BYTES}-byte limit"),
            ));
            continue;
        }

        let Some(remote) = api
            .content_file(
                token,
                &repo.owner,
                &repo.name,
                &entry.path,
                Some(git_ref),
            )
            .await?
        else {
            rejected.push(problem(entry.path, "file disappeared before it could be read"));
            continue;
        };
        let compact: String = remote
            .content_base64
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .collect();
        let bytes = match STANDARD.decode(compact.as_bytes()) {
            Ok(bytes) => bytes,
            Err(_) => {
                rejected.push(problem(entry.path, "content is not valid standard base64"));
                continue;
            }
        };
        match parse_blueprint(&entry.path, &bytes) {
            Ok(workflow) => workflows.push(workflow),
            Err(error) => rejected.push(problem(entry.path, error.to_string())),
        }
    }

    Ok(CatalogView {
        workflows,
        rejected,
        disqualified_ids: Vec::new(),
    })
}

fn is_root_json_file(path: &str) -> bool {
    path.strip_prefix(&format!("{CATALOG_ROOT}/"))
        .is_some_and(|name| !name.contains('/') && name.ends_with(".json"))
}

fn problem(path: String, detail: impl Into<String>) -> CatalogProblem {
    CatalogProblem {
        path,
        detail: detail.into(),
    }
}
