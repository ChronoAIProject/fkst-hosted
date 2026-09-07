//! Parse and validate `.fkst/evolution/config.yaml`.
//!
//! Every check here FAILS CLOSED. A malformed or unsafe configuration must stop
//! enrollment rather than fall back to a default, because a silently-defaulted
//! safety policy reads as an active one.
//!
//! This validation is an INPUT CHECK, not a control. The write boundary — that
//! Evolution touches nothing outside `.fkst/evolution/`, and never `config.yaml`
//! or `intent/**` — is enforced separately at every write, precisely because
//! `config.yaml` is repository content and a boundary defined by the thing it is
//! supposed to bound is no boundary at all.
//!
//! The rules mirror `tools/evolution/src/config.ts` exactly. Two implementations
//! of one schema is a real risk, so the shared test vectors live with the
//! fingerprint vectors and any divergence is a bug in whichever moved.

use serde::Deserialize;

use crate::reconcile::branches::{parse_branch_ref, BranchRef};

/// The only schema version this control plane understands.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// The single root Evolution may write under.
pub const EVOLUTION_ROOT: &str = ".fkst/evolution/";
/// The pre-existing repo-local workflow catalog, independent of Evolution.
pub const PACKAGES_ROOT: &str = ".fkst/packages/";
/// Canonical location of the configuration this module parses.
pub const CONFIG_PATH: &str = ".fkst/evolution/config.yaml";

/// A configuration that failed to parse or violated a fail-closed rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConfigError {}

fn err(message: impl Into<String>) -> ConfigError {
    ConfigError(message.into())
}

/// A path include/exclude selector.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PathSelector {
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSource {
    branch: String,
    /// Required. There is deliberately no default — see [`validate`].
    #[serde(default)]
    product_relevant: Option<PathSelector>,
    coverage: PathSelector,
}

/// A managed output class toggle. `deny_unknown_fields` is what implements the
/// rule that a destination may never be configured: the subtree is fixed by
/// schema, so a `path`, `directory` or `destination` key is a hard failure
/// rather than an ignored extra.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutputToggle {
    #[serde(default)]
    pub enabled: bool,
}

/// The video class additionally selects a storage mode, which must be GitHub-native.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VideoToggle {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub storage: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedOutputs {
    #[serde(default)]
    pub documentation: OutputToggle,
    #[serde(default)]
    pub skills: OutputToggle,
    #[serde(default)]
    pub journeys: OutputToggle,
    #[serde(default)]
    pub screenshots: OutputToggle,
    #[serde(default)]
    pub slides: OutputToggle,
    #[serde(default)]
    pub video: VideoToggle,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Publication {
    pub mode: String,
    #[serde(default)]
    pub require_current_source: bool,
    #[serde(default)]
    pub require_checks: bool,
    #[serde(default)]
    pub allow_direct_push: bool,
    #[serde(default)]
    pub on_owner_close: Option<String>,
    #[serde(default)]
    pub suppression_label: Option<String>,
    #[serde(default)]
    pub max_regeneration_rounds: Option<u32>,
    #[serde(default)]
    pub cycle_deadline_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Drift {
    pub policy: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Security {
    pub run_pull_request_code: bool,
    pub allow_production_data: bool,
    pub allow_production_credentials: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawConfig {
    schema_version: u32,
    #[serde(default = "default_true")]
    enabled: bool,
    source: RawSource,
    artifact_repository: String,
    /// Accepted-but-unread. `deny_unknown_fields` is what enforces the rule that
    /// a misspelled safety policy cannot appear active, and it rejects ANY key it
    /// does not know — so a section this control plane does not yet consume still
    /// has to be declared, or a perfectly valid configuration fails to parse.
    ///
    /// `intent` names the human-owned files; Evolution locates them by path
    /// prefix rather than by this field, and the generator reads their content.
    #[allow(dead_code)]
    #[serde(default)]
    intent: Option<serde_yaml::Value>,
    managed_outputs: ManagedOutputs,
    #[serde(default)]
    absent_producer_roles: Vec<String>,
    #[serde(default)]
    locales: Vec<String>,
    /// Accepted-but-unread: latency controls, consumed once the reconcile loop
    /// schedules Evolution cycles.
    #[allow(dead_code)]
    #[serde(default)]
    triggers: Option<serde_yaml::Value>,
    publication: Publication,
    drift: Drift,
    generator_epoch: i64,
    /// Accepted-but-unread: Release retention is a deletion policy, and deletion
    /// is deliberately not implemented before an owner policy exists to authorise
    /// it. Until then retention candidates are reported, never removed.
    #[allow(dead_code)]
    #[serde(default)]
    retention: Option<serde_yaml::Value>,
    security: Security,
}

fn default_true() -> bool {
    true
}

/// A validated Evolution configuration.
#[derive(Debug, Clone)]
pub struct EvolutionConfig {
    pub enabled: bool,
    /// The trusted source branch, possibly the dynamic `@default` sentinel.
    pub branch: BranchRef,
    pub product_relevant: PathSelector,
    pub coverage: PathSelector,
    pub artifact_repository: String,
    pub managed_outputs: ManagedOutputs,
    pub absent_producer_roles: Vec<String>,
    pub locales: Vec<String>,
    pub publication: Publication,
    pub drift: Drift,
    pub generator_epoch: i64,
    pub security: Security,
}

/// Publication modes, in increasing order of autonomy.
const PUBLICATION_MODES: [&str; 5] = [
    "disabled",
    "observe",
    "propose",
    "automerge-managed",
    "release-gated",
];

const DRIFT_POLICIES: [&str; 3] = ["block", "repair", "adopt"];

/// An include entry may not EXPLICITLY name a reserved prefix.
///
/// A broad wildcard is permitted and is simply narrowed by the unconditional
/// removals; an explicit `.fkst/evolution/docs/**` is a request to re-include
/// generated output into its own input, and fails closed. The test is whether
/// the pattern mentions a reserved prefix literally at all, which catches both
/// `.fkst/evolution/**` and `**/.fkst/evolution/**` while letting a bare `**`
/// through — exactly the line the specification draws.
fn assert_no_reserved_include(selector: &PathSelector, field: &str) -> Result<(), ConfigError> {
    for pattern in &selector.include {
        for reserved in [".fkst/evolution", ".fkst/packages"] {
            if pattern.contains(reserved) {
                return Err(err(format!(
                    "{field}.include may not explicitly name {reserved} (pattern: {pattern})"
                )));
            }
        }
    }
    Ok(())
}

/// Parse and fully validate a configuration document.
pub fn parse_config(yaml: &str) -> Result<EvolutionConfig, ConfigError> {
    // `deny_unknown_fields` throughout: silent acceptance would let a MISSPELLED
    // safety policy appear active, e.g. `allowProductionDat: false` reading as a
    // policy that is in fact absent while the default silently applies.
    let raw: RawConfig =
        serde_yaml::from_str(yaml).map_err(|e| err(format!("config.yaml is not valid: {e}")))?;

    if raw.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(err(format!(
            "unsupported schemaVersion {} (supported: {SUPPORTED_SCHEMA_VERSION})",
            raw.schema_version
        )));
    }

    let branch = parse_branch_ref(&raw.source.branch)
        .map_err(|rule| err(format!("source.branch {rule}")))?;

    // No default set exists. An absent or empty product-relevant selector
    // silently disables ALL cycle admission, which is the failure mode that
    // produces no signal: the artifact that was never regenerated tells nobody.
    let product_relevant = raw
        .source
        .product_relevant
        .ok_or_else(|| err("source.productRelevant is required — no default set exists"))?;
    if product_relevant.include.is_empty() {
        return Err(err("source.productRelevant.include must not be empty"));
    }
    assert_no_reserved_include(&product_relevant, "source.productRelevant")?;
    assert_no_reserved_include(&raw.source.coverage, "source.coverage")?;

    if raw.artifact_repository.trim().is_empty() {
        return Err(err("artifactRepository must be a non-empty string"));
    }

    if !PUBLICATION_MODES.contains(&raw.publication.mode.as_str()) {
        return Err(err(format!(
            "publication.mode must be one of: {}",
            PUBLICATION_MODES.join(", ")
        )));
    }
    if raw.publication.allow_direct_push {
        return Err(err(
            "publication.allowDirectPush must be false — Evolution never pushes the trusted branch",
        ));
    }
    if !raw.publication.require_checks {
        return Err(err(
            "publication.requireChecks must be true — a merge policy that cannot honor required checks is rejected",
        ));
    }

    if !DRIFT_POLICIES.contains(&raw.drift.policy.as_str()) {
        return Err(err(format!(
            "drift.policy must be one of: {}",
            DRIFT_POLICIES.join(", ")
        )));
    }

    if raw.managed_outputs.video.enabled {
        // A non-GitHub-native storage mode is rejected: durable artifacts live in
        // GitHub, never in external storage.
        match raw.managed_outputs.video.storage.as_deref() {
            Some("github-release") => {}
            other => {
                return Err(err(format!(
                    "managedOutputs.video.storage must be \"github-release\" (got {other:?})"
                )))
            }
        }
    }

    if raw.security.run_pull_request_code {
        return Err(err(
            "security.runPullRequestCode must be false — pull request processing is read-only",
        ));
    }
    if raw.security.allow_production_data || raw.security.allow_production_credentials {
        return Err(err(
            "security.allowProductionData and allowProductionCredentials must be false",
        ));
    }

    let locales = if raw.locales.is_empty() {
        vec!["en".to_string()]
    } else {
        raw.locales
    };

    Ok(EvolutionConfig {
        enabled: raw.enabled,
        branch,
        product_relevant,
        coverage: raw.source.coverage,
        artifact_repository: raw.artifact_repository,
        managed_outputs: raw.managed_outputs,
        absent_producer_roles: raw.absent_producer_roles,
        locales,
        publication: raw.publication,
        drift: raw.drift,
        generator_epoch: raw.generator_epoch,
        security: raw.security,
    })
}

/// The producer role that emits each managed output class.
///
/// A class whose role the owner declared absent contributes no required
/// artifacts, so the verifier must not report it missing. The mapping names the
/// role that produces the class's committed bytes.
pub fn producer_role(class: &str) -> &'static str {
    match class {
        "documentation" => "documentation-maintainer",
        "skills" => "skill-builder",
        "journeys" | "screenshots" => "demo-producer",
        "slides" => "narrative-producer",
        "video" => "artifact-renderer",
        _ => "unknown",
    }
}

impl EvolutionConfig {
    /// Classes that contribute required artifacts: enabled, and not produced by a
    /// role the owner declared absent.
    pub fn required_classes(&self) -> Vec<&'static str> {
        let m = &self.managed_outputs;
        [
            ("documentation", m.documentation.enabled),
            ("skills", m.skills.enabled),
            ("journeys", m.journeys.enabled),
            ("screenshots", m.screenshots.enabled),
            ("slides", m.slides.enabled),
            ("video", m.video.enabled),
        ]
        .into_iter()
        .filter(|(class, enabled)| {
            *enabled
                && !self
                    .absent_producer_roles
                    .iter()
                    .any(|role| role == producer_role(class))
        })
        .map(|(class, _)| class)
        .collect()
    }

    /// True when this repository has opted into autonomous merging.
    pub fn is_automerge_managed(&self) -> bool {
        self.publication.mode == "automerge-managed"
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
