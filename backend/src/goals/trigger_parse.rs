//! Parser for the `fkst-substrate-trigger` issue body (Model B, #359 §3).
//!
//! Model B triggers a substrate session from a GitHub issue opened via the
//! `fkst-substrate-trigger` Issue Form. This module turns that user-authored issue
//! **body** into a [`TriggerSpec`] — the launch inputs the Model B launcher needs
//! — keying only on the four canonical `### ` sections. It reuses the shared
//! section-splitting skeleton in [`crate::goals::section_parse`] so the structural
//! contract (duplicate `### ` heading → 422; intro before the first heading
//! ignored) is identical to the `fkst-goal` parser.
//!
//! Scope boundary: this parser *structures + validates the shape* of the launch
//! inputs. It does NOT fetch the referenced packages, resolve them to concrete
//! directories, nor reconcile the work label against GitHub — that is deferred to
//! the launcher and a later fetch/reachability pass. What it DOES enforce is the
//! safety-relevant grammar: a DNS-label session name, fully-qualified GitHub
//! package references (`owner/repo@ref:path`) whose ref and path are path-safe (no
//! absolute path, no `..` traversal), and a single-value comma-free work label
//! (the substrate reads it from a comma-separated env var).
//!
//! Secret hygiene: this module logs nothing and never echoes section content.

use std::sync::OnceLock;

use regex::Regex;

use crate::error::AppError;
use crate::goals::section_parse::{
    env_name_regex, is_valid_env_name, non_empty_lines, parse_environment_name, split_sections,
    strip_html_comments, MAX_ENV_NAME_LEN,
};

/// The canonical `fkst-substrate-trigger` section headings, in template order.
const HEADING_SESSION_NAME: &str = "### Session Name";
const HEADING_PACKAGES: &str = "### Packages";
const HEADING_WORK_LABEL: &str = "### Work Label";
const HEADING_ENVIRONMENT: &str = "### Environment";
const HEADING_AUTO_MERGE: &str = "### Auto-merge";
const HEADING_LOG_ACCESS: &str = "### Log Access Allowlist";
/// The CURRENT name of the trusted-users section (issue #487). The legacy
/// [`HEADING_LOG_ACCESS`] heading stays accepted as an alias forever: the
/// reconciler re-parses live issue bodies every tick, so dropping it would
/// change existing sessions' parsed list → flip their `full_config_hash` →
/// false `fkst-config-rejected` across the fleet.
const HEADING_FKST_CONTRIBUTORS: &str = "### FKST Contributors";
const HEADING_OUTPUT_LANGUAGE: &str = "### Output Language";
const HEADING_ENGINE_CONFIG: &str = "### Engine Config";

/// GitHub caps a label name at 50 characters; the Work Label must fit so the
/// launcher can apply it verbatim.
const MAX_WORK_LABEL_LEN: usize = 50;

/// The expected form of one `### Packages` line, echoed in every 422 so the author
/// can self-correct without leaving the issue.
const PACKAGE_REF_FORM: &str = "owner/repo@ref:path/to/package";

/// Anchored owner/repo-segment pattern: the safe token set a single `owner` or
/// `repo` segment of a package reference may draw from (letters, digits, `.`, `_`,
/// `-`). A `/` is deliberately absent — it separates owner from repo, so neither
/// segment may itself contain one.
fn owner_repo_segment_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9_.-]+$").expect("static owner/repo segment regex"))
}

/// Anchored pattern for the `ref` and `path` parts of a package reference: the
/// safe token set (letters, digits, `.`, `_`, `/`, `-`). The leading-`/` and
/// `..`-segment checks run separately so their 422 messages can be specific.
fn ref_path_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9_./-]+$").expect("static ref/path regex"))
}

/// Anchored pattern for the `### Output Language` value: a conservative locale
/// tag (`en`, `zh`, `zh-CN`, `zh_TW`, `cmn`) — a strict subset of the engine's
/// own `[A-Za-z0-9_-]+` locale charset, so the value is path-safe as the
/// `locales/<value>.lua` filename component the engine resolves it to.
fn output_lang_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new("^[a-z]{2,3}([-_][A-Za-z0-9]{2,8})?$").expect("static output-lang regex")
    })
}

/// A fully-qualified GitHub package reference parsed from one `### Packages` line,
/// of the form `owner/repo@ref:path`. Every part is shape-validated here; fetching
/// the package and checking reachability is deferred to a later pass. (`git_ref`
/// rather than `ref` because `ref` is a Rust keyword.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRef {
    /// The GitHub repository owner (user or org) — e.g. `ChronoAIProject`.
    pub owner: String,
    /// The GitHub repository name — e.g. `fkst-packages`.
    pub repo: String,
    /// The git ref (branch, tag, or SHA) the package is fetched at — e.g. `dev`.
    pub git_ref: String,
    /// The repo-relative path to the package directory — e.g.
    /// `packages/github-devloop`.
    pub path: String,
}

/// The structured launch inputs parsed from an `fkst-substrate-trigger` issue
/// body. Every field is shape-validated; semantic resolution (fetching a package,
/// whether the label exists) is the launcher's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerSpec {
    /// The session name — a DNS-1123-label-ish token (same rule as an environment
    /// name) so it composes into valid Kubernetes object names downstream.
    pub name: String,
    /// One [`PackageRef`] per non-empty `### Packages` line, in author order. Each
    /// is a fully-qualified GitHub package reference (`owner/repo@ref:path`);
    /// fetching + resolving it is deferred to a later pass.
    pub packages: Vec<PackageRef>,
    /// The single GitHub work label the launcher applies to drive the session.
    /// Guaranteed ≤ 50 chars and comma-free.
    pub work_label: String,
    /// The single named ENVIRONMENT the OPTIONAL `### Environment` section selects,
    /// or `None` when the section is absent or blank.
    pub environment: Option<String>,
    /// The OPTIONAL `### Auto-merge` opt-in: `true` when the section's value is
    /// one of true/yes/on/enabled/1 (case-insensitive); `false` when the value is
    /// anything else, blank, or the section is absent. Never a 422 (lenient) so a
    /// pre-v2 trigger issue without the section still parses.
    pub auto_merge: bool,
    /// The OPTIONAL `### Log Access Allowlist`: the GitHub logins or numeric ids
    /// (beyond the issue author + the global admins) permitted to download this
    /// session's redacted logs from the identity-gated `/api/v1/logs/{session_id}`
    /// endpoint. A whitespace/comma/newline-separated list; lenient; default empty.
    /// FROZEN by config-immutability (it is part of `full_config_hash`) so it cannot
    /// be edited AFTER the session registers to grant access retroactively.
    pub log_access: Vec<String>,
    /// The OPTIONAL `### Output Language`: the locale the session's packages emit
    /// user-visible prose in (`FKST_OUTPUT_LANG` → the engine's `t()` i18n SDK,
    /// resolving `locales/<value>.lua` by EXACT filename match, falling back to
    /// `en`). `None` when the section is absent or blank; strictly validated when
    /// present (one value, conservative locale charset) — a 422 names the section.
    pub output_lang: Option<String>,
    /// The OPTIONAL `### Engine Config`: the validated, ALLOWLISTED engine
    /// tunables (`KEY=value` lines — see [`crate::goals::engine_config`]) the
    /// launcher injects as session env. Empty when the section is absent/blank.
    /// Every key/value is bounded at parse time; part of BOTH config hashes.
    pub engine_config: std::collections::BTreeMap<String, String>,
}

/// Parse the `fkst-substrate-trigger` issue body into a [`TriggerSpec`].
///
/// Returns [`AppError::Unprocessable`] (→ 422) whose message NAMES the offending
/// section for every malformed case: a missing/mis-shaped `### Session Name`; a
/// missing/empty/mis-shaped `### Packages`; a missing/mis-shaped `### Work Label`;
/// an invalid `### Environment`; or a duplicate `### ` heading. The 422 (not 400)
/// matches the template-format contract shared with the `fkst-goal` parser.
pub fn parse_trigger_issue_body(body: &str) -> Result<TriggerSpec, AppError> {
    let sections = split_sections(body)?;

    let name = parse_session_name(&sections)?;
    let packages = parse_packages(&sections)?;
    let work_label = parse_work_label(&sections)?;

    // `### Environment` — OPTIONAL, reusing the shared rule verbatim: absent or
    // blank → `None`; one valid name → `Some`; two or more or an invalid name → a
    // 422 naming the section.
    let environment = match sections
        .iter()
        .find(|(heading, _)| heading == HEADING_ENVIRONMENT)
    {
        Some((_, content)) => parse_environment_name(&strip_html_comments(content))?,
        None => None,
    };

    let auto_merge = parse_auto_merge(&sections);
    let log_access = parse_log_access(&sections);
    let output_lang = parse_output_language(&sections)?;

    // `### Engine Config` — OPTIONAL but STRICT; the allowlist parser owns the
    // rules (see `goals::engine_config`). Absent → empty map.
    let engine_config = match sections
        .iter()
        .find(|(heading, _)| heading == HEADING_ENGINE_CONFIG)
    {
        Some((_, content)) => crate::goals::engine_config::parse_engine_config(content)?,
        None => std::collections::BTreeMap::new(),
    };

    Ok(TriggerSpec {
        name,
        packages,
        work_label,
        environment,
        auto_merge,
        log_access,
        output_lang,
        engine_config,
    })
}

/// `### Output Language` — OPTIONAL but STRICT (mirrors `### Environment`):
/// absent, blank, or comment-only → `None`; exactly one non-empty line matching
/// the conservative locale pattern → `Some`; anything else → a 422 naming the
/// section and the rule. Template comments are stripped FIRST (the template
/// ships an explanatory `<!-- … -->` inside the section body, and this parser —
/// unlike the lenient Auto-merge scan — would otherwise count it as content).
fn parse_output_language(sections: &[(String, String)]) -> Result<Option<String>, AppError> {
    let block = match sections
        .iter()
        .find(|(heading, _)| heading == HEADING_OUTPUT_LANGUAGE)
    {
        Some((_, content)) => strip_html_comments(content),
        None => return Ok(None),
    };
    match non_empty_lines(&block).as_slice() {
        [] => Ok(None),
        [lang] if output_lang_regex().is_match(lang) => Ok(Some(lang.clone())),
        [lang] => Err(AppError::Unprocessable(format!(
            "the `### Output Language` section names an invalid locale {lang:?}: must match {} \
             (e.g. `en`, `zh`, `zh-CN`) and exactly match a `locales/<value>.lua` file shipped \
             by the session's package",
            output_lang_regex().as_str()
        ))),
        _ => Err(AppError::Unprocessable(
            "the `### Output Language` section must contain at most one non-empty line".to_string(),
        )),
    }
}

/// `### Auto-merge` — OPTIONAL, lenient. `true` iff the section's FIRST non-empty
/// line is one of true/yes/on/enabled/1 (case-insensitive). Absent/blank/any other
/// value → `false`. Never errors: this is an opt-in flag, not a validated field.
fn parse_auto_merge(sections: &[(String, String)]) -> bool {
    let block = match sections
        .iter()
        .find(|(heading, _)| heading == HEADING_AUTO_MERGE)
    {
        Some((_, content)) => content.as_str(),
        None => return false,
    };
    // Scan ALL non-empty lines for an exact truthy token (not only the first), so
    // the flag still reads `true` when the user leaves the template's explanatory
    // HTML comment above the value line. A comment/prose line never equals a bare
    // token, so this stays a false-positive-free opt-in.
    non_empty_lines(block).iter().any(|v| {
        matches!(
            v.to_ascii_lowercase().as_str(),
            "true" | "yes" | "on" | "enabled" | "1"
        )
    })
}

/// `### FKST Contributors` (legacy alias: `### Log Access Allowlist`) — OPTIONAL,
/// lenient. The session's trusted-users list, serving BOTH purposes: (a) extra
/// GitHub logins/ids (beyond the issue author + the global admins) allowed to
/// download the session's redacted logs, and (b) the logins injected into the
/// session as `FKST_GITHUB_AUTHORIZED_LOGINS`, which the packages' github author
/// policy uses to decide whose issues/comments the session acts on. Tokens are
/// separated by ANY whitespace, comma, or newline; a leading `@` is stripped;
/// empty tokens dropped. Both headings may appear — tokens merge (current heading
/// first), deduped case-insensitively. Absent/blank → empty. Never errors. Tokens
/// are NOT resolved to real accounts: log authz matches by numeric id AND
/// case-insensitive login, and the author policy matches logins, so a token that
/// names no real account simply never matches anything.
fn parse_log_access(sections: &[(String, String)]) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for heading in [HEADING_FKST_CONTRIBUTORS, HEADING_LOG_ACCESS] {
        let Some((_, content)) = sections.iter().find(|(h, _)| h == heading) else {
            continue;
        };
        for token in content
            .split(|c: char| c.is_whitespace() || c == ',')
            .map(|token| token.trim().trim_start_matches('@'))
            .filter(|token| !token.is_empty())
        {
            let folded = token.to_ascii_lowercase();
            if !seen.contains(&folded) {
                seen.push(folded);
                tokens.push(token.to_string());
            }
        }
    }
    tokens
}

/// `### Session Name` — required; EXACTLY ONE non-empty line that satisfies the
/// shared environment-name rule (so the name composes into valid Kubernetes object
/// names). Zero or two-plus lines is a 422; an ill-formed name is a 422 naming the
/// rule.
fn parse_session_name(sections: &[(String, String)]) -> Result<String, AppError> {
    let block = sections
        .iter()
        .find(|(heading, _)| heading == HEADING_SESSION_NAME)
        .map(|(_, content)| strip_html_comments(content))
        .ok_or_else(|| {
            AppError::Unprocessable("the `### Session Name` section is required".to_string())
        })?;
    match non_empty_lines(&block).as_slice() {
        [name] if is_valid_env_name(name) => Ok(name.clone()),
        [name] => Err(AppError::Unprocessable(format!(
            "the `### Session Name` section names an invalid session name {name:?}: must match {} \
             and be 1..={MAX_ENV_NAME_LEN} characters",
            env_name_regex().as_str()
        ))),
        _ => Err(AppError::Unprocessable(
            "the `### Session Name` section must contain exactly one non-empty line".to_string(),
        )),
    }
}

/// `### Packages` — required; at least one non-empty line, EACH a fully-qualified
/// GitHub package reference `owner/repo@ref:path`. A missing/empty section, or any
/// malformed line, is a 422 naming the section (and, for a malformed line, the
/// offending value and which part failed).
fn parse_packages(sections: &[(String, String)]) -> Result<Vec<PackageRef>, AppError> {
    let block = sections
        .iter()
        .find(|(heading, _)| heading == HEADING_PACKAGES)
        .map(|(_, content)| strip_html_comments(content))
        .ok_or_else(|| {
            AppError::Unprocessable("the `### Packages` section is required".to_string())
        })?;
    let lines = non_empty_lines(&block);
    if lines.is_empty() {
        return Err(AppError::Unprocessable(
            "the `### Packages` section must list at least one package".to_string(),
        ));
    }
    let mut packages = Vec::with_capacity(lines.len());
    for line in &lines {
        packages.push(parse_package_ref(line)?);
    }
    Ok(packages)
}

/// Parse one `### Packages` line as a fully-qualified GitHub package reference
/// `owner/repo@ref:path`. The split is greedy on the FIRST `@` (`owner/repo` vs
/// `ref:path`) then the FIRST `:` (`ref` vs `path`). Every failure is a 422 that
/// names the section, echoes the offending value, states which part failed, and
/// recalls the expected form.
fn parse_package_ref(value: &str) -> Result<PackageRef, AppError> {
    let reject = |reason: &str| {
        AppError::Unprocessable(format!(
            "the `### Packages` section lists an invalid package reference {value:?}: {reason}; \
             expected the form {PACKAGE_REF_FORM}"
        ))
    };

    // Split on the FIRST `@`: everything before is `owner/repo`, everything after
    // is `ref:path`.
    let (owner_repo, ref_path) = value
        .split_once('@')
        .ok_or_else(|| reject("missing `@` separating `owner/repo` from `ref:path`"))?;
    // Split `ref:path` on the FIRST `:`: the ref, then the repo-relative path.
    let (git_ref, path) = ref_path
        .split_once(':')
        .ok_or_else(|| reject("missing `:` separating the ref from the path"))?;

    // `owner/repo`: exactly one `/`, each side a non-empty safe segment.
    if owner_repo.matches('/').count() != 1 {
        return Err(reject(
            "the part before `@` must be exactly `owner/repo` with a single `/`",
        ));
    }
    let (owner, repo) = owner_repo
        .split_once('/')
        .expect("a single `/` is present after the count check");
    for (segment, which) in [(owner, "owner"), (repo, "repo")] {
        if segment.is_empty() {
            return Err(reject(&format!("the {which} must not be empty")));
        }
        if !owner_repo_segment_regex().is_match(segment) {
            return Err(reject(&format!("the {which} must match ^[A-Za-z0-9_.-]+$")));
        }
    }

    // `ref`: non-empty, no `..` traversal segment, only the safe token set.
    if git_ref.is_empty() {
        return Err(reject("the ref must not be empty"));
    }
    if git_ref.split('/').any(|segment| segment == "..") {
        return Err(reject("the ref must not contain a `..` path segment"));
    }
    if !ref_path_regex().is_match(git_ref) {
        return Err(reject("the ref must match ^[A-Za-z0-9_./-]+$"));
    }

    // `path`: non-empty, not absolute, no `..` traversal segment, only the safe
    // token set. Mirrors the path-safety checks the old Package Roots applied.
    if path.is_empty() {
        return Err(reject("the path must not be empty"));
    }
    if path.starts_with('/') {
        return Err(reject("the path must not start with `/`"));
    }
    if path.split('/').any(|segment| segment == "..") {
        return Err(reject("the path must not contain a `..` path segment"));
    }
    if !ref_path_regex().is_match(path) {
        return Err(reject("the path must match ^[A-Za-z0-9_./-]+$"));
    }

    Ok(PackageRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        git_ref: git_ref.to_string(),
        path: path.to_string(),
    })
}

/// `### Work Label` — required; EXACTLY ONE non-empty line that is a valid GitHub
/// label: ≤ 50 characters and comma-free. The comma ban is load-bearing — the
/// substrate reads the label from a comma-separated env var, so a comma would split
/// it into two labels. Zero or two-plus lines is a 422; an over-long or comma-bearing
/// value is a 422 naming the section.
fn parse_work_label(sections: &[(String, String)]) -> Result<String, AppError> {
    let block = sections
        .iter()
        .find(|(heading, _)| heading == HEADING_WORK_LABEL)
        .map(|(_, content)| strip_html_comments(content))
        .ok_or_else(|| {
            AppError::Unprocessable("the `### Work Label` section is required".to_string())
        })?;
    let label = match non_empty_lines(&block).as_slice() {
        [label] => label.clone(),
        _ => {
            return Err(AppError::Unprocessable(
                "the `### Work Label` section must contain exactly one non-empty line".to_string(),
            ))
        }
    };
    if label.chars().count() > MAX_WORK_LABEL_LEN {
        return Err(AppError::Unprocessable(format!(
            "the `### Work Label` section names a label {label:?} longer than \
             {MAX_WORK_LABEL_LEN} characters"
        )));
    }
    if label.contains(',') {
        return Err(AppError::Unprocessable(format!(
            "the `### Work Label` section names a label {label:?} containing a comma; the \
             substrate reads a comma-separated env var, so a comma would split it into two labels"
        )));
    }
    Ok(label)
}

#[cfg(test)]
#[path = "trigger_parse_tests.rs"]
mod tests;
