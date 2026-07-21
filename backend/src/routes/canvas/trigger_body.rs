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
    /// Fully-qualified package references (`owner/repo@ref:path`). Optional since
    /// a `### Manifest` reference can supply the packages (epic #594 I7), but at
    /// least one of `packages` / `manifests` must be non-empty.
    pub packages: Vec<String>,
    /// Optional fully-qualified fkst-manifest references (`owner/repo@ref:path`,
    /// the SAME grammar as `packages`): each names a JSON bundle the server
    /// expands into a package list (`### Manifest`). Empty renders no section.
    #[serde(default)]
    pub manifests: Vec<String>,
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
    /// Optional work-item collaborators (`### Session Collaborators`): GitHub
    /// logins granted authority over the session's WORK issues, beyond the
    /// trigger author. A DISTINCT list from `log_access` (log-download access) —
    /// it gates who may raise/label/comment on this session's work issues.
    #[serde(default)]
    pub collaborators: Vec<String>,
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
    // A session needs SOME package source. Since I7 a `### Manifest` reference can
    // supply the packages, so the hard `### Packages`-required rule is relaxed to
    // "≥1 of packages / manifests" — mirroring the trigger parser's own rule.
    let packages: Vec<&str> = req
        .packages
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    let manifests: Vec<&str> = req
        .manifests
        .iter()
        .map(|m| m.trim())
        .filter(|m| !m.is_empty())
        .collect();
    if packages.is_empty() && manifests.is_empty() {
        return Err(AppError::Validation(
            "at least one package or manifest is required".to_string(),
        ));
    }
    for package in &packages {
        require_inline(package, "packages entry")?;
    }
    for manifest in &manifests {
        require_inline(manifest, "manifests entry")?;
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
    let collaborators: Vec<&str> = req
        .collaborators
        .iter()
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .collect();
    for entry in &collaborators {
        require_inline(entry, "collaborators entry")?;
    }

    let mut body = String::new();
    push_section(&mut body, "### Session Name", &[name]);
    // `### Packages` is omitted when empty (a manifest-only session); `### Manifest`
    // follows it, matching the template's section order.
    if !packages.is_empty() {
        push_section(&mut body, "### Packages", &packages);
    }
    if !manifests.is_empty() {
        push_section(&mut body, "### Manifest", &manifests);
    }
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
    if !collaborators.is_empty() {
        push_section(&mut body, "### Session Collaborators", &collaborators);
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
    let requested_packages: Vec<String> = req
        .packages
        .iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    // Manifests parse with the SAME grammar as packages, so — like packages — a
    // rendered `### Manifest` line that parses back to a different reference (or an
    // entry that fans out) fails closed. Compare the parsed refs entry-for-entry.
    let rendered_manifests: Vec<String> =
        spec.manifest_refs.iter().map(render_package_ref).collect();
    let requested_manifests: Vec<String> = req
        .manifests
        .iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect();
    // The parser splits a log-access line on ANY whitespace/comma (and strips a
    // leading '@'), so one requested entry can silently become several
    // grantees — and that list doubles as the session's authorized-logins
    // policy. Compare it entry-for-entry like every other field.
    let requested_log_access: Vec<String> = req
        .log_access
        .iter()
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .collect();
    // Collaborators are the work-item authority list — the same split-on-any-
    // whitespace/comma-then-strip-'@' parsing as log_access, so one requested
    // entry can silently fan out into several grantees. Compare it entry-for-
    // entry too, so an entry that would grant authority the request never
    // listed fails closed.
    let requested_collaborators: Vec<String> = req
        .collaborators
        .iter()
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .collect();
    let round_trips = spec.name == req.name.trim()
        && rendered_packages == requested_packages
        && rendered_manifests == requested_manifests
        && spec.work_label.as_deref() == trimmed(req.work_label.as_deref())
        && spec.environment.as_deref() == trimmed(req.environment.as_deref())
        && spec.output_lang.as_deref() == trimmed(req.output_lang.as_deref())
        && spec.auto_merge == (req.auto_merge == Some(true))
        && spec.log_access == requested_log_access
        && spec.collaborators == requested_collaborators
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
