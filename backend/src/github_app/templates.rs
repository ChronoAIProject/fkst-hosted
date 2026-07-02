//! Bundled issue-template source of truth + the write-PR orchestration that keeps
//! every App-installed repo's `.github/ISSUE_TEMPLATE/` at the latest bundled
//! version (version-aware issue-template reconciliation).
//!
//! Two halves, split so the orchestration is fakeable:
//!   - PURE: [`FKST_ISSUE_TEMPLATES_VERSION`] (the compile-time source of truth),
//!     the bundled template files, and [`parse_installed_version`] (reads the
//!     `# fkst-issue-templates-version: N` marker from a repo's `config.yml`).
//!   - EFFECTFUL: the [`IssueTemplateGithub`] abstraction (injected so the
//!     reconcile ensure is unit-testable against a fake) and its production
//!     [`GithubAppTokens`] impl — a mint-then-call wrapper that reads the installed
//!     version and installs/updates all templates via a MERGED PR onto the repo's
//!     default branch (trusted fixed content, no review/CI gate).
//!
//! Secret hygiene: the installation token is minted with the least-privilege
//! [`issue_templates_permissions`] set (contents+pull_requests only, never the
//! `administration` grant `default_permissions` needs) and is NEVER logged.

use async_trait::async_trait;

use super::{issue_templates_permissions, GithubAppError, GithubAppTokens};

/// Compile-time source of truth for the bundled issue templates. Bump on ANY
/// change to the bundled files below; the bundled `config.yml` marker MUST equal
/// this (enforced by a unit test). Repos whose installed version is below this
/// get a merged PR to catch up.
pub const FKST_ISSUE_TEMPLATES_VERSION: u32 = 1;

/// Repo-relative directory the templates live under.
const TEMPLATE_DIR: &str = ".github/ISSUE_TEMPLATE";

/// One bundled template file: its repo-relative path and its exact content.
pub struct TemplateFile {
    pub path: &'static str,
    pub content: &'static str,
}

// The three bundled assets are stored as literal files under `templates_assets/`
// and embedded at compile time so `gitleaks`/reviewers see the exact text and
// this module stays small.
const CONFIG_YML: &str = include_str!("templates_assets/config.yml");
const SESSION_TEMPLATE: &str = include_str!("templates_assets/fkst-substrate-session.md");
const WORK_ITEM_TEMPLATE: &str = include_str!("templates_assets/fkst-work-item.md");

/// The bundled templates, each with its full repo-relative path. These are the
/// files the install PR writes.
pub fn bundled_templates() -> [TemplateFile; 3] {
    [
        TemplateFile {
            path: ".github/ISSUE_TEMPLATE/config.yml",
            content: CONFIG_YML,
        },
        TemplateFile {
            path: ".github/ISSUE_TEMPLATE/fkst-substrate-session.md",
            content: SESSION_TEMPLATE,
        },
        TemplateFile {
            path: ".github/ISSUE_TEMPLATE/fkst-work-item.md",
            content: WORK_ITEM_TEMPLATE,
        },
    ]
}

/// Scan `config.yml` for the `# fkst-issue-templates-version: N` marker; the first
/// hit wins. A missing marker (or an unparseable N) means installed version `0`
/// (which forces an install). This is the version read that gates a repo.
pub fn parse_installed_version(config_yml: &str) -> u32 {
    const MARKER: &str = "# fkst-issue-templates-version:";
    for line in config_yml.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(MARKER) {
            return rest.trim().parse::<u32>().unwrap_or(0);
        }
    }
    0
}

/// Decode a GitHub Contents `content` field: standard base64 with embedded
/// newlines (GitHub wraps the payload). Whitespace is stripped before decoding.
fn decode_github_content(b64: &str) -> Result<String, GithubAppError> {
    use base64::Engine;
    let cleaned: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .map_err(|e| GithubAppError::Http(format!("template content decode: {e}")))?;
    String::from_utf8(bytes).map_err(|e| GithubAppError::Http(format!("template utf8: {e}")))
}

/// Encode a bundled template body as standard base64 for a Contents `PUT`.
fn encode_content(content: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(content.as_bytes())
}

/// The GitHub-facing operations the template reconcile needs, injected so the
/// ensure orchestration is unit-testable against a fake without a live GitHub.
/// [`GithubAppTokens`] is the production implementation.
#[async_trait]
pub trait IssueTemplateGithub: Send + Sync {
    /// The version currently installed in `owner_repo`'s
    /// `.github/ISSUE_TEMPLATE/config.yml` (a missing file => `0`).
    async fn installed_templates_version(&self, owner_repo: &str) -> Result<u32, GithubAppError>;

    /// Install/update ALL bundled templates in `owner_repo` to `target_version`
    /// via a single merged PR onto the default branch.
    async fn install_templates(
        &self,
        owner_repo: &str,
        target_version: u32,
    ) -> Result<(), GithubAppError>;
}

#[async_trait]
impl IssueTemplateGithub for GithubAppTokens {
    async fn installed_templates_version(&self, owner_repo: &str) -> Result<u32, GithubAppError> {
        let (owner, repo) = owner_repo
            .split_once('/')
            .ok_or(GithubAppError::InvalidRepoRef)?;
        // Least-privilege mint (contents + pull_requests only). Never logged.
        let token = self
            .token_for_repo(owner_repo, Some(issue_templates_permissions()))
            .await?;
        let path = format!("{TEMPLATE_DIR}/config.yml");
        match self
            .api()
            .content_file(&token, owner, repo, &path, None)
            .await?
        {
            None => Ok(0),
            Some(file) => Ok(parse_installed_version(&decode_github_content(
                &file.content_base64,
            )?)),
        }
    }

    async fn install_templates(
        &self,
        owner_repo: &str,
        target_version: u32,
    ) -> Result<(), GithubAppError> {
        let (owner, repo) = owner_repo
            .split_once('/')
            .ok_or(GithubAppError::InvalidRepoRef)?;
        let token = self
            .token_for_repo(owner_repo, Some(issue_templates_permissions()))
            .await?;
        let api = self.api();

        let base = api.repo_default_branch(&token, owner, repo).await?;
        let head_sha = api.branch_head_sha(&token, owner, repo, &base).await?;
        let branch = format!("fkst/issue-templates-v{target_version}");

        // Create the working branch. If a stale one lingers from a prior failed
        // run, drop and recreate it so this run starts from the current head.
        if let Err(GithubAppError::RefExists) = api
            .create_ref(&token, owner, repo, &branch, &head_sha)
            .await
        {
            api.delete_ref(&token, owner, repo, &branch).await.ok();
            api.create_ref(&token, owner, repo, &branch, &head_sha)
                .await?;
        }

        for tf in bundled_templates() {
            // An existing blob on the base branch => UPDATE (PUT requires its
            // sha); None => CREATE.
            let existing = api
                .content_file(&token, owner, repo, tf.path, Some(&base))
                .await?;
            let sha = existing.map(|f| f.sha);
            let content_b64 = encode_content(tf.content);
            let msg = format!("chore(fkst): sync {} to v{target_version}", tf.path);
            api.put_file(
                &token,
                owner,
                repo,
                tf.path,
                &msg,
                &content_b64,
                &branch,
                sha.as_deref(),
            )
            .await?;
        }

        let title = format!("Install/Update fkst issue templates to v{target_version}");
        let number = api
            .create_pull_request(
                &token,
                owner,
                repo,
                &title,
                &branch,
                &base,
                "Automated by fkst-hosted: bundled issue templates (trusted fixed content). \
                 Merged without review/CI by design.",
            )
            .await?;
        api.merge_pull_request(&token, owner, repo, number, &title)
            .await?;
        // Best-effort cleanup of the merged branch; a failure here never fails
        // the install (the PR is already merged).
        api.delete_ref(&token, owner, repo, &branch).await.ok();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_missing_marker_is_zero() {
        assert_eq!(parse_installed_version("blank_issues_enabled: false"), 0);
        assert_eq!(parse_installed_version(""), 0);
    }

    #[test]
    fn parse_reads_marker() {
        let cfg = "# something\n# fkst-issue-templates-version: 3\nblank: false";
        assert_eq!(parse_installed_version(cfg), 3);
    }

    #[test]
    fn parse_unparseable_marker_is_zero() {
        let cfg = "# fkst-issue-templates-version: abc";
        assert_eq!(parse_installed_version(cfg), 0);
    }

    #[test]
    fn parse_reads_marker_with_leading_whitespace() {
        // The bundled marker is at column 0, but tolerate an indented copy.
        let cfg = "   # fkst-issue-templates-version: 7";
        assert_eq!(parse_installed_version(cfg), 7);
    }

    #[test]
    fn bundled_config_marker_matches_const() {
        // The version marker embedded in the bundled config.yml can never drift
        // from the compile-time constant.
        assert_eq!(
            parse_installed_version(CONFIG_YML),
            FKST_ISSUE_TEMPLATES_VERSION
        );
    }

    #[test]
    fn decode_github_content_strips_newlines() {
        // GitHub wraps base64 with embedded newlines; "hello" == aGVsbG8=.
        let wrapped = "aGVs\nbG8=\n";
        assert_eq!(decode_github_content(wrapped).unwrap(), "hello");
    }

    #[test]
    fn encode_then_decode_roundtrips() {
        let s = "line one\nline two\n";
        assert_eq!(decode_github_content(&encode_content(s)).unwrap(), s);
    }

    #[test]
    fn bundled_templates_paths_are_under_issue_template_dir() {
        for tf in bundled_templates() {
            assert!(
                tf.path.starts_with(".github/ISSUE_TEMPLATE/"),
                "path {} not under the ISSUE_TEMPLATE dir",
                tf.path
            );
            assert!(!tf.content.is_empty(), "{} is empty", tf.path);
        }
    }

    #[test]
    fn session_template_front_matter_has_trigger_label_and_title() {
        assert!(
            SESSION_TEMPLATE.contains("labels: [fkst-substrate-trigger]"),
            "session template must carry the trigger label"
        );
        assert!(
            SESSION_TEMPLATE.contains("title: \"[session] \""),
            "session template must carry the [session] title prefix"
        );
    }
}
