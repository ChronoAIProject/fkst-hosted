//! The capacity worksheet, read from the document that owns it, plus the
//! artifact writer and percentile helpers the performance gate uses.
//!
//! ## Why the assumptions are parsed rather than copied
//!
//! `deploy/kubernetes/AUDIT-TRACE.md` carries the capacity worksheet the
//! checked-in PVC size, `FKST_AUDIT_RELAY_MAX_RECORDS`, and the disk-pressure
//! alert thresholds are all derived from. The acceptance criterion is that
//! "capacity results meet documented production assumptions" — which is only a
//! checkable statement if the test reads the SAME numbers a reviewer would.
//!
//! Copying them into a Rust constant would satisfy the letter and defeat the
//! point: the constant and the document would drift, and the test would go on
//! passing against a worksheet nobody had honoured. Parsing means an edit to the
//! worksheet immediately re-aims the gate, and deleting a row from it fails the
//! gate rather than silently removing an assertion.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};

/// The documented production assumptions, keyed by the worksheet's `Input`
/// column.
#[derive(Debug)]
pub struct Worksheet {
    rows: BTreeMap<String, String>,
}

impl Worksheet {
    /// Parse the `## Capacity worksheet` table out of `AUDIT-TRACE.md`.
    pub fn load(repo_root: &Path) -> Self {
        let path = repo_root.join("deploy/kubernetes/AUDIT-TRACE.md");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let section = text
            .split("## Capacity worksheet")
            .nth(1)
            .unwrap_or_else(|| panic!("{} has no capacity worksheet", path.display()));
        // The table ends at the next fenced block (the derivation) or heading.
        let table = section
            .split("```")
            .next()
            .unwrap_or(section)
            .split("\n## ")
            .next()
            .unwrap_or(section);

        let mut rows = BTreeMap::new();
        for line in table.lines() {
            let line = line.trim();
            if !line.starts_with('|') {
                continue;
            }
            let cells: Vec<&str> = line
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect::<Vec<_>>();
            if cells.len() < 2 || cells[0].starts_with("---") || cells[0] == "Input" {
                continue;
            }
            rows.insert(cells[0].to_string(), cells[1].to_string());
        }
        assert!(
            rows.len() > 10,
            "the capacity worksheet parsed to {} rows; the table shape changed",
            rows.len()
        );
        Self { rows }
    }

    /// The raw `Assumed` cell for one input, which must exist.
    pub fn assumed(&self, input: &str) -> &str {
        self.rows
            .get(input)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("the capacity worksheet no longer documents {input:?}"))
    }

    /// The first number in an `Assumed` cell (`"~1.0 KiB"` -> `1.0`,
    /// `"5 / s"` -> `5.0`, `"< 30 s"` -> `30.0`).
    pub fn number(&self, input: &str) -> f64 {
        let cell = self.assumed(input);
        let mut digits = String::new();
        for character in cell.chars() {
            if character.is_ascii_digit() || (character == '.' && !digits.is_empty()) {
                digits.push(character);
            } else if !digits.is_empty() {
                break;
            }
        }
        digits
            .parse()
            .unwrap_or_else(|_| panic!("the {input:?} worksheet cell {cell:?} states no number"))
    }
}

/// One measured quantity, as it appears in the evidence artifact.
pub struct Measurement {
    pub name: &'static str,
    pub unit: &'static str,
    pub value: f64,
}

impl Measurement {
    pub fn new(name: &'static str, unit: &'static str, value: f64) -> Self {
        Self { name, unit, value }
    }
}

/// Nearest-rank percentiles over an unsorted sample.
pub struct Profile {
    pub p95: f64,
    pub p99: f64,
}

pub fn summarize(mut observations: Vec<f64>) -> Profile {
    observations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Profile {
        p95: percentile(&observations, 0.95),
        p99: percentile(&observations, 0.99),
    }
}

pub fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() as f64) * fraction).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

/// The repository root, derived from the crate manifest directory.
pub fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the backend crate always has a parent directory")
        .to_path_buf()
}

/// Write the measured numbers next to the requirement evidence.
///
/// The artifact carries measurements and the documented assumptions they were
/// compared against — no request payload, no identity, no credential — so it can
/// be attached to the milestone record as it stands.
///
/// `name` is per-suite: several suites in this binary measure different things,
/// and one shared file name would leave whichever ran last as the only record.
pub fn write_artifact(name: &str, measurements: &[Measurement], assumptions: &Value) {
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"),
    };
    let dir = target.join("acceptance");
    if std::fs::create_dir_all(&dir).is_err() {
        eprintln!("acceptance: could not create the artifact directory; skipping the record");
        return;
    }
    let document = json!({
        "kind": "fkst-acceptance-performance",
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "note": "Measured by backend/tests/acceptance_performance.rs. Debug-profile \
                 numbers are an upper bound on the release build. `assumptions` is \
                 read from deploy/kubernetes/AUDIT-TRACE.md's capacity worksheet.",
        "assumptions": assumptions,
        "measurements": measurements
            .iter()
            .map(|measurement| json!({
                "name": measurement.name,
                "unit": measurement.unit,
                "value": (measurement.value * 1_000.0).round() / 1_000.0,
            }))
            .collect::<Vec<_>>(),
    });
    let rendered = serde_json::to_string_pretty(&document).unwrap_or_else(|_| "{}".to_string());
    if std::fs::write(dir.join(name), rendered).is_err() {
        eprintln!("acceptance: could not write {name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worksheet parses and still documents the rows the gate compares
    /// against. A worksheet edit that drops one of them fails HERE, naming it,
    /// rather than silently removing an assertion from the capacity gate.
    #[test]
    fn the_capacity_worksheet_still_documents_every_input_the_gate_uses() {
        let worksheet = Worksheet::load(&repo_root());
        assert_eq!(worksheet.number("peak sustained audited requests"), 5.0);
        assert_eq!(worksheet.number("average safe event bytes"), 1.0);
        assert_eq!(worksheet.number("normal PostHog ingestion lag"), 30.0);
        assert_eq!(worksheet.number("capture batch size"), 100.0);
        assert_eq!(worksheet.number("max accepted record body"), 64.0);
    }

    /// The number parser handles every shape the worksheet's cells use.
    #[test]
    fn the_assumed_cell_parser_reads_each_documented_shape() {
        let worksheet = Worksheet::load(&repo_root());
        assert!(worksheet.assumed("relay outage to absorb").contains("24 h"));
        assert_eq!(worksheet.number("relay outage to absorb"), 24.0);
        assert_eq!(worksheet.number("writer queue depth"), 512.0);
    }
}
