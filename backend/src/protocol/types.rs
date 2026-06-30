//! Ornn injection wire types.
//!
//! What remains here after the worker fleet was removed (single-crate
//! restructure): the resolved Ornn injection plan the skill resolver produces
//! and the session runner consumes. The controller<->worker protocol (register
//! / heartbeat / pull / dispatch / credential-refresh) is gone with the worker
//! deployable; pod-per-session execution uses the
//! [`crate::session_spec::SessionSpec`] contract instead.
//!
//! Input validation: every struct is `#[serde(deny_unknown_fields)]` so
//! malformed/extra-field input is rejected at the trust boundary.

use serde::{Deserialize, Serialize};

/// Where a resolved Ornn skill's bytes come from: an inlined base64 ZIP (small
/// skillsets) or a presigned URL fetched directly (egress-free escape hatch for
/// large ones).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrnnSource {
    ZipB64(String),
    PresignedUrl(String),
}

/// One resolved Ornn skill to install into the engine's codex home.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OrnnSkillRef {
    pub name: String,
    pub source: OrnnSource,
}

/// Resolved Ornn injection plan (#114): the AGENTS.md appends + the skills.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OrnnPlan {
    pub agents_md_appends: Vec<String>,
    pub skills: Vec<OrnnSkillRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ornn_source_serializes_snake_case_tagged() {
        assert_eq!(
            serde_json::to_string(&OrnnSource::ZipB64("abc".into())).unwrap(),
            r#"{"zip_b64":"abc"}"#
        );
        assert_eq!(
            serde_json::to_string(&OrnnSource::PresignedUrl("u".into())).unwrap(),
            r#"{"presigned_url":"u"}"#
        );
    }

    #[test]
    fn ornn_plan_round_trips_and_rejects_unknown_fields() {
        let plan = OrnnPlan {
            agents_md_appends: vec!["use skill X".into()],
            skills: vec![OrnnSkillRef {
                name: "x".into(),
                source: OrnnSource::PresignedUrl("https://store/x.zip".into()),
            }],
        };
        let json = serde_json::to_string(&plan).unwrap();
        let back: OrnnPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(plan, back);

        let bad = r#"{"agents_md_appends":[],"skills":[],"extra":true}"#;
        assert!(serde_json::from_str::<OrnnPlan>(bad).is_err());
    }
}
