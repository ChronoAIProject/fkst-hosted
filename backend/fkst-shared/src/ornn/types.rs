//! Ornn DTOs and the user-facing skill-pin types (issue #114).
//!
//! These mirror the verified Ornn `main` (v0.12.0) API contract reached through
//! the NyxID proxy slug `ornn-api`. Every deserialize is TOLERANT (unknown
//! fields ignored) so the types survive Ornn field drift; the integration tests
//! pin the exact contract.
//!
//! Secret hygiene: a `presigned_package_url` is sensitive (it is a time-limited
//! capability granting direct read of the package bytes from chrono-storage),
//! so it is held only transiently and NEVER logged. The pin types carry no
//! secret material — only names + concrete versions.

use serde::{Deserialize, Serialize};

/// Validation bound: an Ornn skill/skillset name is at most this many bytes.
/// Mirrors the registry's own naming limit and keeps a pin compact.
pub const MAX_PIN_NAME_BYTES: usize = 64;

/// Which kind of Ornn artifact a pin refers to. Serializes lowercase on the
/// wire (`"skill"` / `"skillset"`) so the trigger request and the persisted
/// `SessionDoc.ornn_skills` share one stable representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrnnPinKind {
    /// A single Ornn skill package.
    Skill,
    /// An Ornn skillset: a master prompt plus a closure of member skills.
    Skillset,
}

/// One user-pinned Ornn artifact: a concrete `name@version` plus its kind.
///
/// Supplied at session trigger and persisted (resolved) onto `SessionDoc` so a
/// failover rebuild re-injects the identical set. Carries no secret material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrnnSkillPin {
    pub kind: OrnnPinKind,
    pub name: String,
    /// Concrete `<major>.<minor>` version (Ornn has no `@latest`/dist-tags).
    pub version: String,
}

/// A resolved leaf node to install: a single skill `name@version`. Produced by
/// expanding skillset closures and flattening skill pins, then deduped by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNode {
    pub name: String,
    pub version: String,
}

/// Hop-1 skill detail (`GET /api/v1/skills/<name>?version=<m.minor>`).
///
/// The `presigned_package_url` is the time-limited DIRECT chrono-storage URL
/// for the verbatim package zip (hop 2). SENSITIVE — never logged.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillDetail {
    pub name: String,
    pub version: String,
    #[serde(rename = "presignedPackageUrl")]
    pub presigned_package_url: String,
}

/// One member of a skillset closure (`GET /api/v1/skillsets/<name>/closure`).
/// `items` is deduped + topo-sorted; only `name` + `version` are load-bearing
/// for installation (the rest of the node is tolerated and ignored).
#[derive(Debug, Clone, Deserialize)]
pub struct ClosureNode {
    pub name: String,
    pub version: String,
}

/// Result of a skillset closure: the master `instructions` prompt plus the
/// member skills to install via the two-hop flow.
#[derive(Debug, Clone, Deserialize)]
pub struct ClosureResult {
    /// The skillset's master prompt, appended to `$CODEX_HOME/AGENTS.md`.
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub items: Vec<ClosureNode>,
}

/// One version row from `GET .../<name>/versions` (newest-first). Tolerant:
/// only the fields the picker needs are typed; the rest are ignored.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionRow {
    pub version: String,
    #[serde(default, rename = "isDeprecated")]
    pub is_deprecated: bool,
    #[serde(default, rename = "releaseNotes")]
    pub release_notes: Option<String>,
    #[serde(default, rename = "createdOn")]
    pub created_on: Option<String>,
}

/// One search row from `GET /api/v1/skill-search` / `skillset-search`. Tolerant:
/// search rows do not carry a full version list (loaded lazily via `/versions`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchRow {
    pub name: String,
    #[serde(default)]
    pub guid: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, rename = "isPrivate")]
    pub is_private: bool,
    #[serde(default, rename = "isSystemSkill")]
    pub is_system_skill: bool,
    #[serde(default)]
    pub kind: Option<String>,
}

/// A page of search rows plus pagination metadata, as fkst-hosted re-serializes
/// the Ornn search response for its catalog API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchPage {
    #[serde(default)]
    pub items: Vec<SearchRow>,
    #[serde(default)]
    pub page: u32,
    #[serde(default, rename = "pageSize")]
    pub page_size: u32,
    #[serde(default)]
    pub total: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_round_trips_through_json_with_lowercase_kind() {
        let pin = OrnnSkillPin {
            kind: OrnnPinKind::Skillset,
            name: "web-research".to_string(),
            version: "2.0".to_string(),
        };
        let json = serde_json::to_value(&pin).expect("serialize");
        assert_eq!(json["kind"], "skillset");
        assert_eq!(json["name"], "web-research");
        assert_eq!(json["version"], "2.0");
        let back: OrnnSkillPin = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, pin);
    }

    #[test]
    fn skill_detail_deserializes_camelcase_presigned_url() {
        let detail: SkillDetail = serde_json::from_value(serde_json::json!({
            "name": "demo",
            "version": "1.0",
            "presignedPackageUrl": "https://storage.example/pkg.zip?sig=abc",
            "guid": "ignored",
            "isPrivate": true
        }))
        .expect("deserialize");
        assert_eq!(detail.name, "demo");
        assert_eq!(detail.version, "1.0");
        assert_eq!(
            detail.presigned_package_url,
            "https://storage.example/pkg.zip?sig=abc"
        );
    }

    #[test]
    fn skill_detail_debug_must_not_be_emitted_to_logs_but_is_inspectable() {
        // The url is sensitive; this test documents that callers must not log
        // it, and confirms Debug still works for tests (Debug is not a leak by
        // itself — the discipline is "never put it in a tracing line").
        let detail: SkillDetail = serde_json::from_value(serde_json::json!({
            "name": "demo", "version": "1.0",
            "presignedPackageUrl": "https://storage.example/secret"
        }))
        .expect("deserialize");
        assert!(format!("{detail:?}").contains("demo"));
    }

    #[test]
    fn closure_result_tolerates_missing_fields() {
        let empty: ClosureResult = serde_json::from_value(serde_json::json!({})).expect("empty");
        assert!(empty.instructions.is_empty());
        assert!(empty.items.is_empty());

        let full: ClosureResult = serde_json::from_value(serde_json::json!({
            "instructions": "Do the thing.",
            "items": [
                { "name": "a", "version": "1.0", "depth": 0, "ref": "a@1.0" },
                { "name": "b", "version": "2.1" }
            ]
        }))
        .expect("full");
        assert_eq!(full.instructions, "Do the thing.");
        assert_eq!(full.items.len(), 2);
        assert_eq!(full.items[1].name, "b");
        assert_eq!(full.items[1].version, "2.1");
    }

    #[test]
    fn search_row_tolerates_unknown_fields_and_maps_camelcase_flags() {
        let row: SearchRow = serde_json::from_value(serde_json::json!({
            "name": "fmt",
            "guid": "g1",
            "description": "format code",
            "tags": ["lint"],
            "isPrivate": true,
            "isSystemSkill": true,
            "unknown": 42
        }))
        .expect("deserialize");
        assert_eq!(row.name, "fmt");
        assert!(row.is_private);
        assert!(row.is_system_skill);
        assert_eq!(row.tags, vec!["lint".to_string()]);
    }

    #[test]
    fn version_row_defaults_deprecated_to_false() {
        let row: VersionRow =
            serde_json::from_value(serde_json::json!({ "version": "1.0" })).expect("deserialize");
        assert_eq!(row.version, "1.0");
        assert!(!row.is_deprecated);
    }
}
