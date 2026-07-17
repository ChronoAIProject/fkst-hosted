//! Render a create-session request into an `fkst-substrate-trigger` issue
//! body, and validate it BEFORE any GitHub write by round-tripping the
//! rendered text through the reconciler's own trigger parser
//! ([`crate::goals::trigger_parse`]) — the parser owns the section grammar, so
//! a body that passes here is bit-for-bit a body the reconciler will register.
//!
//! Boundary hardening: every field must be a single line that does not start
//! with `#` (a smuggled line/heading could inject a section — e.g. an
//! `### Engine Config` block — into the rendered body), and the parsed spec is
//! compared back against the request so any renderer/parse divergence fails
//! closed as a 400 instead of creating a trigger that means something else.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::AppError;
use crate::goals::trigger_parse::parse_trigger_issue_body;
use crate::routes::canvas::types::render_package_ref;

/// Request body for creating a session (a trigger issue) on a repo.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    /// The session name (`### Session Name`; also the issue title).
    pub name: String,
    /// Fully-qualified package references (`owner/repo@ref:path`), at least one.
    pub packages: Vec<String>,
    /// The optional explicit work label (`### Work Label`).
    pub work_label: Option<String>,
    /// The optional named environment (`### Environment`).
    pub environment: Option<String>,
    /// The optional auto-merge opt-in (`### Auto-merge`); only an explicit
    /// `true` renders the section (absent and `false` mean the same thing to
    /// the parser).
    pub auto_merge: Option<bool>,
    /// Optional extra log-download grantees (`### Log Access Allowlist`).
    #[serde(default)]
    pub log_access: Vec<String>,
    /// The optional session output locale (`### Output Language`).
    pub output_lang: Option<String>,
}

/// Reject a field value that could break out of its section: anything
/// multi-line, or a line that could render as a Markdown heading.
fn require_inline(value: &str, what: &str) -> Result<(), AppError> {
    if value.contains('\n') || value.contains('\r') {
        return Err(AppError::Validation(format!(
            "{what} must be a single line"
        )));
    }
    if value.starts_with('#') {
        return Err(AppError::Validation(format!(
            "{what} must not start with '#'"
        )));
    }
    Ok(())
}

/// Append one `### ` section with its value lines.
fn push_section(body: &mut String, heading: &str, lines: &[&str]) {
    if !body.is_empty() {
        body.push('\n');
    }
    body.push_str(heading);
    body.push_str("\n\n");
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
}

/// A trimmed optional field: blank collapses to `None` (an omitted section).
fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// Render the trigger-issue body for `req` (sections in template order,
/// optional sections omitted when absent). Fails 400 on any field the
/// single-line boundary rejects, before anything is rendered.
fn render_trigger_body(req: &CreateSessionRequest) -> Result<String, AppError> {
    let name = req.name.trim();
    require_inline(name, "name")?;
    if req.packages.is_empty() {
        return Err(AppError::Validation(
            "at least one package is required".to_string(),
        ));
    }
    let packages: Vec<&str> = req.packages.iter().map(|p| p.trim()).collect();
    for package in &packages {
        require_inline(package, "packages entry")?;
    }
    let work_label = trimmed(req.work_label.as_deref());
    if let Some(value) = work_label {
        require_inline(value, "work_label")?;
    }
    let environment = trimmed(req.environment.as_deref());
    if let Some(value) = environment {
        require_inline(value, "environment")?;
    }
    let output_lang = trimmed(req.output_lang.as_deref());
    if let Some(value) = output_lang {
        require_inline(value, "output_lang")?;
    }
    let log_access: Vec<&str> = req
        .log_access
        .iter()
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .collect();
    for entry in &log_access {
        require_inline(entry, "log_access entry")?;
    }

    let mut body = String::new();
    push_section(&mut body, "### Session Name", &[name]);
    push_section(&mut body, "### Packages", &packages);
    if let Some(value) = work_label {
        push_section(&mut body, "### Work Label", &[value]);
    }
    if let Some(value) = environment {
        push_section(&mut body, "### Environment", &[value]);
    }
    if req.auto_merge == Some(true) {
        push_section(&mut body, "### Auto-merge", &["true"]);
    }
    if !log_access.is_empty() {
        push_section(&mut body, "### Log Access Allowlist", &log_access);
    }
    if let Some(value) = output_lang {
        push_section(&mut body, "### Output Language", &[value]);
    }
    Ok(body)
}

/// Render `req` and prove the result round-trips through the trigger parser
/// back to exactly the requested launch inputs. Any parser rejection surfaces
/// as a 400 carrying the parser's own section-naming message (not the
/// reconciler's later 422 on a live issue); any parse-back divergence — which
/// would mean the created trigger registers something other than what was
/// requested — fails closed as a 400.
pub(super) fn validated_trigger_body(req: &CreateSessionRequest) -> Result<String, AppError> {
    let body = render_trigger_body(req)?;
    let spec = match parse_trigger_issue_body(&body) {
        Ok(spec) => spec,
        Err(AppError::Unprocessable(message)) => return Err(AppError::Validation(message)),
        Err(other) => return Err(other),
    };

    let rendered_packages: Vec<String> = spec.packages.iter().map(render_package_ref).collect();
    let requested_packages: Vec<String> =
        req.packages.iter().map(|p| p.trim().to_string()).collect();
    let round_trips = spec.name == req.name.trim()
        && rendered_packages == requested_packages
        && spec.work_label.as_deref() == trimmed(req.work_label.as_deref())
        && spec.environment.as_deref() == trimmed(req.environment.as_deref())
        && spec.output_lang.as_deref() == trimmed(req.output_lang.as_deref())
        && spec.auto_merge == (req.auto_merge == Some(true))
        && spec.engine_config.is_empty();
    if !round_trips {
        // Defense in depth: reachable only if a value slips past the inline
        // guard yet still parses (renderer/parser drift). Never echo the body.
        tracing::warn!("canvas create-session: rendered trigger body did not round-trip");
        return Err(AppError::Validation(
            "the rendered trigger body did not round-trip through the trigger parser; \
             check the field values"
                .to_string(),
        ));
    }
    Ok(body)
}

#[cfg(test)]
#[path = "trigger_body_tests.rs"]
mod tests;
