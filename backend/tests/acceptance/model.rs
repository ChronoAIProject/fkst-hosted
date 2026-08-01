//! The typed shape of `acceptance/requirement-matrix.toml`.
//!
//! Parsing is strict (`deny_unknown_fields`) on purpose: a typo in a field name
//! would otherwise be silently dropped, and a dropped `gate_env` is exactly the
//! mistake that turns an honestly-gated staging claim into a false "verified".

use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;

/// The whole matrix document.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matrix {
    pub schema_version: u32,
    pub epic: u64,
    pub gate_issue: u64,
    pub milestone: u64,
    #[serde(default)]
    pub requirement: Vec<Requirement>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

/// One normative requirement id from the epic's requirement table.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub id: String,
    pub area: String,
    pub summary: String,
    /// The component that answers for this requirement when its evidence breaks.
    ///
    /// A COMPONENT rather than a person on purpose: a personal name in a
    /// checked-in file goes stale the moment somebody changes team, and the
    /// question an on-call reviewer actually has is "whose code is this", which
    /// the component answers durably. The vocabulary is closed ([`OWNERS`]) so
    /// the field cannot decay into free text.
    pub owner: String,
}

/// One named automated test claimed as evidence for one requirement.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub requirement: String,
    pub tier: String,
    pub suite: String,
    pub test: String,
    pub status: String,
    /// Present exactly when `status = "gated"`: the environment variable whose
    /// absence makes the named test skip with a stated reason.
    #[serde(default)]
    pub gate_env: Option<String>,
}

/// The tiers the matrix may declare, in escalating cost order.
pub const TIERS: [&str; 3] = ["pr", "integration", "staging"];

/// The statuses the matrix may declare.
pub const STATUSES: [&str; 2] = ["verified", "gated"];

/// The components a requirement may be owned by.
///
/// One per top-level area of this repository, so "who fixes this" resolves to a
/// directory a reviewer can open.
pub const OWNERS: [&str; 4] = [
    // `backend/src/audit`, `backend/src/audit_relay`
    "control-plane-audit",
    // `backend/src/operations`, `backend/src/session_access`, `session_backend`
    "control-plane-operations",
    // `frontend/`
    "frontend",
    // `deploy/kubernetes/`
    "deployment",
];

/// Every requirement id in epic #5665's normative table.
///
/// Hard-coded rather than derived, because the point of the check is to notice
/// when the matrix and the epic disagree — deriving one from the other would
/// make them agree by construction.
pub const EPIC_REQUIREMENTS: [&str; 28] = [
    "AUTH-01", "AUTH-02", "AUTH-03", "AUTH-04", "AUTH-05", "AUTH-06", "AUD-01", "AUD-02", "AUD-03",
    "AUD-04", "AUD-05", "AUD-06", "AUD-07", "SBOX-01", "SBOX-02", "SBOX-03", "SBOX-04", "SBOX-05",
    "SBOX-06", "UI-01", "UI-02", "UI-03", "UI-04", "OPS-01", "OPS-02", "OPS-03", "OPS-04",
    "TEST-01",
];

impl Matrix {
    /// Read and parse the checked-in matrix.
    pub fn load(repo_root: &Path) -> Result<Self, String> {
        let path = repo_root.join("acceptance/requirement-matrix.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        Self::parse(&text)
    }

    /// Parse from text. Split out so the linter's own negative tests can feed it
    /// deliberately broken documents without touching the filesystem.
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|error| error.to_string())
    }

    /// The declared requirement ids, in declaration order.
    pub fn requirement_ids(&self) -> Vec<&str> {
        self.requirement.iter().map(|r| r.id.as_str()).collect()
    }

    /// The set of declared requirement ids.
    pub fn requirement_set(&self) -> BTreeSet<&str> {
        self.requirement.iter().map(|r| r.id.as_str()).collect()
    }

    /// Evidence rows claimed for one requirement id.
    pub fn evidence_for<'a>(&'a self, requirement: &str) -> Vec<&'a Evidence> {
        self.evidence
            .iter()
            .filter(|row| row.requirement == requirement)
            .collect()
    }
}
