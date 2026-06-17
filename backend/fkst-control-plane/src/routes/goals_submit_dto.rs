//! Wire types + reference parsers for `POST /api/v1/goals/submit` (#178).
//!
//! Split out of [`super::goals_submit`] (the handler) purely for file-size
//! hygiene: this module holds the request/response DTOs and the pure URL/ref
//! parsers (`parse_repo_ref`, `parse_issue_ref`) plus their unit tests, so the
//! handler module stays focused on orchestration. The issue-BODY template
//! parser lives in [`crate::goals::issue_parse`] (a goals-domain concern).
//!
//! 422 contract: every parser here returns [`AppError::Unprocessable`] (→ 422)
//! on a malformed reference, matching the #178 Definition of Done (the input is
//! well-formed JSON but the referenced repo/issue is semantically unparseable).
//! This is deliberately distinct from the inline path's existing FIELD
//! validation, which keeps `validate_goal_fields`'s 400 (`AppError::Validation`).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::AppError;
use crate::goals::{validate_goal_fields, GoalStatus, RepoRef};
use crate::ornn::OrnnSkillPin;

use super::goals::{InlineSecretInput, RepoRefBody};

// ---- URL / ref parsers -----------------------------------------------------

/// Parse a repo reference: a full `https://github.com/{owner}/{name}` HTTPS URL
/// (optional trailing `.git` and/or `/`) OR a bare `{owner}/{name}`. Owner/name
/// are validated against the same grammar `validate_goal_fields` enforces.
///
/// Rejects anything else with a 422 (`AppError::Unprocessable`) per the #178
/// Definition of Done.
pub(crate) fn parse_repo_ref(input: &str) -> Result<RepoRefBody, AppError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(reject_repo("empty repo reference"));
    }

    let path = match trimmed.strip_prefix("https://github.com/") {
        Some(rest) => rest,
        None => {
            // Not an HTTPS URL: must be a bare `owner/name` (no scheme, no host).
            if trimmed.contains("://") {
                return Err(reject_repo("unsupported URL scheme or host"));
            }
            trimmed
        }
    };

    // Normalize: drop a trailing `/` then a trailing `.git`.
    let path = path.strip_suffix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);

    let mut segments = path.split('/');
    let (Some(owner), Some(name), None) = (segments.next(), segments.next(), segments.next())
    else {
        return Err(reject_repo("expected exactly `owner/name`"));
    };
    if owner.is_empty() || name.is_empty() {
        return Err(reject_repo("owner and name must be non-empty"));
    }

    let repo = RepoRefBody {
        owner: owner.to_string(),
        name: name.to_string(),
    };
    validate_repo_grammar(&repo)?;
    Ok(repo)
}

/// Parse a GitHub issue URL `https://github.com/{owner}/{name}/issues/{number}`
/// into `(repo, number)`. Rejects anything else with a 422. Reuses the same
/// owner/name grammar.
pub(crate) fn parse_issue_ref(input: &str) -> Result<(RepoRefBody, u64), AppError> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix("https://github.com/").ok_or_else(|| {
        AppError::Unprocessable(
            "issue url must be `https://github.com/{owner}/{name}/issues/{number}`".to_string(),
        )
    })?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);

    let mut segments = rest.split('/');
    let (Some(owner), Some(name), Some(kw), Some(num), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return Err(AppError::Unprocessable(
            "issue url must be `https://github.com/{owner}/{name}/issues/{number}`".to_string(),
        ));
    };
    if kw != "issues" {
        return Err(AppError::Unprocessable(format!(
            "issue url must contain `/issues/`, found `/{kw}/`"
        )));
    }
    let number: u64 = num.parse().map_err(|_| {
        AppError::Unprocessable(format!(
            "issue number must be a positive integer, got `{num}`"
        ))
    })?;

    let repo = RepoRefBody {
        owner: owner.to_string(),
        name: name.to_string(),
    };
    validate_repo_grammar(&repo)?;
    Ok((repo, number))
}

/// Build a uniform 422 for a rejected repo reference.
fn reject_repo(reason: &str) -> AppError {
    AppError::Unprocessable(format!("invalid repo reference: {reason}"))
}

/// Validate owner/name against `validate_goal_fields`' repo grammar, mapping a
/// grammar failure to 422 (the new parsers' contract — distinct from the inline
/// path's existing 400 field validation).
pub(crate) fn validate_repo_grammar(repo: &RepoRefBody) -> Result<(), AppError> {
    validate_goal_fields(
        "x",
        "x",
        &["x".to_string()],
        Some(&RepoRef {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
        }),
    )
    .map_err(AppError::Unprocessable)
}

// ---- DTOs ------------------------------------------------------------------

/// Request body for `POST /api/v1/goals/submit`. A tagged enum on `source`:
/// `issue` adopts an existing GitHub issue; `inline` carries all goal args.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "source", deny_unknown_fields)]
pub enum SubmitSessionRequest {
    /// Start from an existing user-authored GitHub issue. The issue body is
    /// fetched and parsed (#177 contract); secrets — if any — still come inline
    /// (never from the issue).
    Issue {
        issue: IssueRef,
        #[serde(default)]
        secrets: Option<Vec<InlineSecretInput>>,
    },
    /// Start from all-inline arguments (no pre-existing issue). The server files
    /// one issue whose body is the non-sensitive summary + marker only.
    Inline {
        /// The engine-facing goal prompt; content NEVER logged.
        goal: String,
        repo: RepoSpecBody,
        package_names: Vec<String>,
        #[serde(default)]
        ornn_skills: Option<Vec<OrnnSkillPin>>,
        #[serde(default)]
        secrets: Option<Vec<InlineSecretInput>>,
    },
}

/// An issue reference: either a full GitHub issue URL or the structured triple.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum IssueRef {
    /// `{ "url": "https://github.com/{o}/{n}/issues/{num}" }` (parsed at runtime).
    Url { url: String },
    /// `{ "owner": .., "name": .., "number": u64 }`.
    Parts {
        owner: String,
        name: String,
        number: u64,
    },
}

impl IssueRef {
    /// Resolve to `(repo, number)`, parsing the URL form (422 on a bad URL).
    pub(crate) fn resolve(&self) -> Result<(RepoRefBody, u64), AppError> {
        match self {
            IssueRef::Url { url } => parse_issue_ref(url),
            IssueRef::Parts {
                owner,
                name,
                number,
            } => {
                let repo = RepoRefBody {
                    owner: owner.clone(),
                    name: name.clone(),
                };
                validate_repo_grammar(&repo)?;
                Ok((repo, *number))
            }
        }
    }
}

/// A repo reference for the inline source: either a URL/`owner/name` string or
/// the structured `{ owner, name }`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum RepoSpecBody {
    /// `{ "url": "<repo url or owner/name>" }` (parsed via `parse_repo_ref`).
    Url { url: String },
    /// `{ "owner": .., "name": .. }`.
    Parts(RepoRefBody),
}

impl RepoSpecBody {
    /// Resolve to a validated [`RepoRef`] (422 on a bad URL/grammar).
    pub(crate) fn resolve(&self) -> Result<RepoRef, AppError> {
        let body = match self {
            RepoSpecBody::Url { url } => parse_repo_ref(url)?,
            RepoSpecBody::Parts(body) => {
                validate_repo_grammar(body)?;
                body.clone()
            }
        };
        Ok(RepoRef {
            owner: body.owner,
            name: body.name,
        })
    }
}

/// Response body for `POST /api/v1/goals/submit` (202). Mirrors `TriggerResponse`
/// (`goal_status` + a `&'static str` `session_status`) plus the issue locator.
#[derive(Debug, Serialize, ToSchema)]
pub struct SubmitSessionResponse {
    pub goal_id: String,
    pub session_id: String,
    pub issue_number: u64,
    pub issue_url: String,
    pub goal_status: GoalStatus,
    #[schema(value_type = String, example = "pending")]
    pub session_status: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_repo_ref ----

    #[test]
    fn parse_repo_ref_full_https_url() {
        let repo = parse_repo_ref("https://github.com/acme/site").expect("https url");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "site");
    }

    #[test]
    fn parse_repo_ref_git_suffix_and_trailing_slash() {
        let repo = parse_repo_ref("https://github.com/acme/site.git/").expect(".git suffix");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "site");
    }

    #[test]
    fn parse_repo_ref_bare_owner_name() {
        let repo = parse_repo_ref("acme/site").expect("bare owner/name");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "site");
    }

    #[test]
    fn parse_repo_ref_rejects_empty() {
        assert!(matches!(
            parse_repo_ref("   "),
            Err(AppError::Unprocessable(_))
        ));
    }

    #[test]
    fn parse_repo_ref_rejects_wrong_host() {
        assert!(matches!(
            parse_repo_ref("https://gitlab.com/acme/site"),
            Err(AppError::Unprocessable(_))
        ));
    }

    #[test]
    fn parse_repo_ref_rejects_three_segments() {
        assert!(matches!(
            parse_repo_ref("acme/site/extra"),
            Err(AppError::Unprocessable(_))
        ));
    }

    // ---- parse_issue_ref ----

    #[test]
    fn parse_issue_ref_valid_url() {
        let (repo, number) =
            parse_issue_ref("https://github.com/acme/site/issues/42").expect("valid issue url");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "site");
        assert_eq!(number, 42);
    }

    #[test]
    fn parse_issue_ref_rejects_non_numeric_number() {
        assert!(matches!(
            parse_issue_ref("https://github.com/acme/site/issues/abc"),
            Err(AppError::Unprocessable(_))
        ));
    }

    #[test]
    fn parse_issue_ref_rejects_wrong_path_shape() {
        // `/pull/` is not `/issues/`.
        assert!(matches!(
            parse_issue_ref("https://github.com/acme/site/pull/42"),
            Err(AppError::Unprocessable(_))
        ));
        // Missing the number segment.
        assert!(matches!(
            parse_issue_ref("https://github.com/acme/site/issues"),
            Err(AppError::Unprocessable(_))
        ));
    }

    // ---- SubmitSessionRequest deser ----

    #[test]
    fn submit_session_request_inline_variant_deserializes() {
        let json = r#"{
            "source": "inline",
            "goal": "do the thing",
            "repo": { "owner": "acme", "name": "site" },
            "package_names": ["pkg-a"],
            "secrets": [{"key":"OPENAI_API_KEY","value":"sk-x"}]
        }"#;
        let req: SubmitSessionRequest = serde_json::from_str(json).expect("inline deser");
        match req {
            SubmitSessionRequest::Inline {
                goal,
                repo,
                package_names,
                secrets,
                ..
            } => {
                assert_eq!(goal, "do the thing");
                assert_eq!(repo.resolve().unwrap().owner, "acme");
                assert_eq!(package_names, vec!["pkg-a"]);
                assert_eq!(secrets.expect("secrets").len(), 1);
            }
            _ => panic!("expected inline variant"),
        }
    }

    #[test]
    fn submit_session_request_inline_accepts_repo_url_string() {
        let json = r#"{
            "source": "inline",
            "goal": "g",
            "repo": { "url": "https://github.com/acme/site.git" },
            "package_names": ["pkg-a"]
        }"#;
        let req: SubmitSessionRequest = serde_json::from_str(json).expect("inline url deser");
        let SubmitSessionRequest::Inline { repo, .. } = req else {
            panic!("expected inline");
        };
        let resolved = repo.resolve().expect("repo url resolves");
        assert_eq!(resolved.owner, "acme");
        assert_eq!(resolved.name, "site");
    }

    #[test]
    fn submit_session_request_issue_variant_url_form() {
        let json = r#"{
            "source": "issue",
            "issue": { "url": "https://github.com/acme/site/issues/7" }
        }"#;
        let req: SubmitSessionRequest = serde_json::from_str(json).expect("issue url deser");
        let SubmitSessionRequest::Issue { issue, secrets } = req else {
            panic!("expected issue");
        };
        assert!(secrets.is_none());
        let (repo, number) = issue.resolve().expect("issue resolves");
        assert_eq!(repo.owner, "acme");
        assert_eq!(number, 7);
    }

    #[test]
    fn submit_session_request_issue_variant_parts_form() {
        let json = r#"{
            "source": "issue",
            "issue": { "owner": "acme", "name": "site", "number": 9 }
        }"#;
        let req: SubmitSessionRequest = serde_json::from_str(json).expect("issue parts deser");
        let SubmitSessionRequest::Issue { issue, .. } = req else {
            panic!("expected issue");
        };
        let (repo, number) = issue.resolve().expect("issue resolves");
        assert_eq!(repo.name, "site");
        assert_eq!(number, 9);
    }

    #[test]
    fn submit_session_request_rejects_unknown_fields() {
        let json = r#"{"source":"inline","goal":"g","repo":{"owner":"a","name":"b"},"package_names":["p"],"bogus":1}"#;
        assert!(
            serde_json::from_str::<SubmitSessionRequest>(json).is_err(),
            "unknown fields must be rejected"
        );
    }

    #[test]
    fn submit_session_request_rejects_unknown_source() {
        let json = r#"{"source":"telepathy","goal":"g"}"#;
        assert!(serde_json::from_str::<SubmitSessionRequest>(json).is_err());
    }

    /// A secret value supplied in the body must NEVER render through `{:?}`
    /// (the reused `InlineSecretInput` redacts it).
    #[test]
    fn submit_session_request_debug_redacts_secret_value() {
        let json = r#"{
            "source": "inline",
            "goal": "g",
            "repo": { "owner": "acme", "name": "site" },
            "package_names": ["pkg-a"],
            "secrets": [{"key":"OPENAI_API_KEY","value":"sk-leaky"}]
        }"#;
        let req: SubmitSessionRequest = serde_json::from_str(json).expect("deser");
        let rendered = format!("{req:?}");
        assert!(!rendered.contains("sk-leaky"), "secret leaked: {rendered}");
        assert!(rendered.contains("<redacted>"));
    }

    // ---- response shape ----

    #[test]
    fn submit_session_response_serializes_to_documented_shape() {
        let resp = SubmitSessionResponse {
            goal_id: "g".to_string(),
            session_id: "s".to_string(),
            issue_number: 42,
            issue_url: "https://github.com/acme/site/issues/42".to_string(),
            goal_status: GoalStatus::Triggered,
            session_status: "pending",
        };
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["goal_id"], "g");
        assert_eq!(body["session_id"], "s");
        assert_eq!(body["issue_number"], 42);
        assert_eq!(body["issue_url"], "https://github.com/acme/site/issues/42");
        assert_eq!(body["goal_status"], "triggered");
        assert_eq!(body["session_status"], "pending");
    }
}
