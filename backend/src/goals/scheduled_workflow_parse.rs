//! The `fkst-scheduled-workflow` work-issue grammar.
//!
//! This is the control plane's SECOND issue grammar. The first — the
//! `fkst-substrate-trigger` body ([`crate::goals::trigger_parse`]) — declares a
//! session: which packages run, on which branches, for whom. This one declares a
//! JOB that an already-registered session runs: which workflow, with which
//! arguments, once or on a cadence.
//!
//! It reuses [`crate::goals::section_parse`], so the structural contract is shared
//! and learned once: a duplicate `### ` heading is a 422, `#### ` is body text
//! rather than a boundary, and intro markdown before the first heading is ignored.
//!
//! Two deliberate differences from the trigger grammar:
//!
//! - **The body is editable.** Trigger configuration is frozen by
//!   [`crate::reconcile::hashing::full_config_hash`], because changing what a
//!   running session IS mid-flight has no safe meaning. A cadence you cannot change
//!   is simply a broken feature, so this body is re-parsed every pass and an edit
//!   takes effect on the next one.
//! - **Arguments are data, never shell fragments.** They are substituted into a
//!   step's argv/prompt by the runner with explicit escaping. A value that looks
//!   like a credential is refused outright (see [`reject_credential_shaped`]) —
//!   secrets reach a step through an environment profile, never through an issue
//!   body that every repository collaborator can read.
//!
//! Secret hygiene: no parse error ever echoes an `### Arguments` VALUE, and this
//! module logs nothing.

use std::collections::BTreeMap;

use crate::error::AppError;
use crate::goals::section_parse::{non_empty_lines, split_sections, strip_html_comments};
use crate::schedule::CronExpr;

const HEADING_WORKFLOW: &str = "### Workflow";
const HEADING_RUN_MODE: &str = "### Run Mode";
const HEADING_ARGUMENTS: &str = "### Arguments";

/// The `cron: ` prefix of the recurring run mode.
const CRON_PREFIX: &str = "cron:";
/// The one-shot run mode.
const ONCE: &str = "once";

/// Shape limits, mirroring [`crate::goals::package_env`] so an author meets one set
/// of bounds across both grammars.
const MAX_WORKFLOW_ID_BYTES: usize = 64;
const MAX_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_KEY_BYTES: usize = 64;
const MAX_ARGUMENT_VALUE_BYTES: usize = 1024;
const MAX_ARGUMENTS_SERIALIZED_BYTES: usize = 16 * 1024;

/// How often a definition runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunMode {
    /// Run exactly once, as soon as the schedule pass observes the issue. The
    /// definition stays open afterwards so its run record remains visible; the
    /// recorded run is what stops it firing again.
    Once,
    /// Run on a recurring UTC cadence.
    Cron(CronExpr),
}

impl RunMode {
    /// The author's text, round-tripped for the API projection and the run marker.
    pub fn render(&self) -> String {
        match self {
            RunMode::Once => ONCE.to_string(),
            RunMode::Cron(cron) => format!("cron: {}", cron.expression()),
        }
    }
}

/// A parsed scheduled-workflow definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledWorkflowSpec {
    /// The workflow id, resolving to `.fkst/workflows/<id>.toml` in the target repo.
    pub workflow_id: String,
    pub run_mode: RunMode,
    /// Author-supplied arguments, substituted into the workflow's steps by the
    /// runner. Ordered so the rendered run-issue body is deterministic.
    pub arguments: BTreeMap<String, String>,
}

/// Parse a `fkst-scheduled-workflow` issue body.
///
/// Every rejection is a 422 naming the offending section, because the message is
/// posted back onto the issue for the author to act on.
pub fn parse_scheduled_workflow(body: &str) -> Result<ScheduledWorkflowSpec, AppError> {
    let sections = split_sections(body)?;
    reject_unknown_sections(&sections)?;

    Ok(ScheduledWorkflowSpec {
        workflow_id: parse_workflow_id(&sections)?,
        run_mode: parse_run_mode(&sections)?,
        arguments: parse_arguments(&sections)?,
    })
}

/// Refuse a heading this grammar does not define.
///
/// Fail-closed rather than tolerant, which is the opposite of the trigger parser's
/// choice: a trigger body is a long form whose unknown headings are usually the
/// issue template's own prose, whereas this body is three sections long and an
/// unrecognised one is far more likely to be a typo in a heading the author
/// believes is doing something (`### Schedule` instead of `### Run Mode`).
fn reject_unknown_sections(sections: &[(String, String)]) -> Result<(), AppError> {
    for (heading, _) in sections {
        if !matches!(
            heading.as_str(),
            HEADING_WORKFLOW | HEADING_RUN_MODE | HEADING_ARGUMENTS
        ) {
            return Err(AppError::Unprocessable(format!(
                "unknown section `{heading}` in a scheduled-workflow issue: expected \
                 `{HEADING_WORKFLOW}`, `{HEADING_RUN_MODE}`, and optionally `{HEADING_ARGUMENTS}`"
            )));
        }
    }
    Ok(())
}

/// `### Workflow` — required; exactly one path-safe token naming the definition
/// file `.fkst/workflows/<id>.toml`.
fn parse_workflow_id(sections: &[(String, String)]) -> Result<String, AppError> {
    let id = single_line(sections, HEADING_WORKFLOW)?;
    if !is_path_safe_id(&id) {
        return Err(AppError::Unprocessable(format!(
            "the `{HEADING_WORKFLOW}` section names an invalid workflow id {id:?}: expected \
             1-{MAX_WORKFLOW_ID_BYTES} bytes of letters, digits, `.`, `_` or `-`, not starting \
             or ending with a separator and containing no `..`"
        )));
    }
    Ok(id)
}

/// A workflow id becomes a path segment, so it must not be able to traverse out of
/// `.fkst/workflows/` or name a dotfile.
fn is_path_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_WORKFLOW_ID_BYTES
        && !id.contains("..")
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && id
            .bytes()
            .next_back()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
}

/// `### Run Mode` — required; exactly `once` or `cron: <five-field expression>`.
fn parse_run_mode(sections: &[(String, String)]) -> Result<RunMode, AppError> {
    let line = single_line(sections, HEADING_RUN_MODE)?;
    if line.eq_ignore_ascii_case(ONCE) {
        return Ok(RunMode::Once);
    }
    let Some(expression) = strip_cron_prefix(&line) else {
        return Err(AppError::Unprocessable(format!(
            "the `{HEADING_RUN_MODE}` section must be `{ONCE}` or `cron: <expression>`, got \
             {line:?}"
        )));
    };
    // The cron parser's own message already names the offending field and token,
    // which is exactly what the author needs, so it is surfaced unchanged.
    Ok(RunMode::Cron(CronExpr::parse(expression)?))
}

/// Accept `cron:` case-insensitively with or without a space after the colon.
fn strip_cron_prefix(line: &str) -> Option<&str> {
    let prefix = line.get(..CRON_PREFIX.len())?;
    prefix
        .eq_ignore_ascii_case(CRON_PREFIX)
        .then(|| line[CRON_PREFIX.len()..].trim())
}

/// `### Arguments` — optional; `key: value` lines within the shared shape caps.
fn parse_arguments(sections: &[(String, String)]) -> Result<BTreeMap<String, String>, AppError> {
    let Some((_, content)) = sections
        .iter()
        .find(|(heading, _)| heading == HEADING_ARGUMENTS)
    else {
        return Ok(BTreeMap::new());
    };

    let mut arguments = BTreeMap::new();
    let mut serialized = 0usize;
    for line in non_empty_lines(&strip_html_comments(content)) {
        let (key, value) = line.split_once(':').ok_or_else(|| {
            AppError::Unprocessable(format!(
                "the `{HEADING_ARGUMENTS}` section expects `key: value` lines; one line has no \
                 `:` separator"
            ))
        })?;
        let key = key.trim().to_string();
        let value = value.trim().to_string();

        if !is_valid_argument_key(&key) {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING_ARGUMENTS}` section has an invalid key {key:?}: expected \
                 1-{MAX_ARGUMENT_KEY_BYTES} bytes of letters, digits and `_`, starting with a \
                 letter"
            )));
        }
        if value.is_empty() {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING_ARGUMENTS}` section leaves argument {key:?} empty"
            )));
        }
        if value.len() > MAX_ARGUMENT_VALUE_BYTES {
            // The VALUE is never echoed: it is author-controlled text of unbounded
            // provenance, and an over-long value is exactly the shape a pasted
            // credential has.
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING_ARGUMENTS}` section's {key:?} value is {} bytes; the limit is \
                 {MAX_ARGUMENT_VALUE_BYTES}",
                value.len()
            )));
        }
        if value.chars().any(char::is_control) {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING_ARGUMENTS}` section's {key:?} value contains control characters"
            )));
        }
        reject_credential_shaped(&key, &value)?;

        serialized += key.len() + value.len();
        if serialized > MAX_ARGUMENTS_SERIALIZED_BYTES {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING_ARGUMENTS}` section exceeds the {MAX_ARGUMENTS_SERIALIZED_BYTES}-byte \
                 total budget"
            )));
        }
        if arguments.insert(key.clone(), value).is_some() {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING_ARGUMENTS}` section declares {key:?} more than once"
            )));
        }
        if arguments.len() > MAX_ARGUMENTS {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING_ARGUMENTS}` section declares more than {MAX_ARGUMENTS} arguments"
            )));
        }
    }
    Ok(arguments)
}

fn is_valid_argument_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_ARGUMENT_KEY_BYTES
        && key.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Well-known credential prefixes. Matching one is conclusive: no legitimate
/// workflow argument starts with a provider's token marker.
const CREDENTIAL_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "nyx_",
    "sk-",
    "xoxb-",
    "xoxp-",
    "AKIA",
    "-----BEGIN",
];

/// The length at which an unbroken token-alphabet string stops looking like prose.
const CREDENTIAL_MIN_ENTROPY_LEN: usize = 32;

/// Refuse a value that looks like a credential, pointing at environment profiles.
///
/// The heuristic is deliberately two-part and cannot be perfect: a known provider
/// prefix (conclusive), or a long unbroken run of token-alphabet characters mixing
/// letters and digits (suggestive). A false positive costs the author one renamed
/// argument; a false negative publishes a live secret into an issue body that every
/// repository collaborator, and every future reader of the run history, can see.
///
/// The offending VALUE never appears in the error — that would move the secret from
/// the issue body into the control plane's error surface and its logs.
fn reject_credential_shaped(key: &str, value: &str) -> Result<(), AppError> {
    let looks_like_credential = CREDENTIAL_PREFIXES
        .iter()
        .any(|prefix| value.starts_with(prefix))
        || is_high_entropy_token(value);
    if !looks_like_credential {
        return Ok(());
    }
    Err(AppError::Unprocessable(format!(
        "the `{HEADING_ARGUMENTS}` section's {key:?} value looks like a credential. Arguments \
         are public issue content: put secrets in a named environment profile and reference \
         them from the workflow definition by key name instead."
    )))
}

fn is_high_entropy_token(value: &str) -> bool {
    value.len() >= CREDENTIAL_MIN_ENTROPY_LEN
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'_' | b'-')
        })
        && value.bytes().any(|byte| byte.is_ascii_digit())
        && value.bytes().any(|byte| byte.is_ascii_alphabetic())
}

/// Read a required section that must hold exactly one non-empty line, with the
/// issue templates' guidance comments stripped first.
fn single_line(sections: &[(String, String)], heading: &str) -> Result<String, AppError> {
    let (_, content) = sections
        .iter()
        .find(|(candidate, _)| candidate == heading)
        .ok_or_else(|| AppError::Unprocessable(format!("missing required section `{heading}`")))?;
    match non_empty_lines(&strip_html_comments(content)).as_slice() {
        [line] => Ok(line.clone()),
        [] => Err(AppError::Unprocessable(format!(
            "the `{heading}` section must not be empty"
        ))),
        _ => Err(AppError::Unprocessable(format!(
            "the `{heading}` section must contain exactly one non-empty line"
        ))),
    }
}

#[cfg(test)]
#[path = "scheduled_workflow_parse_tests.rs"]
mod tests;
