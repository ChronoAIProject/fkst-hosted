//! The rules that make the matrix a gate rather than a document.
//!
//! Every rule returns a violation string instead of panicking, for one reason:
//! the linter has to be testable against deliberately broken input. A rule that
//! panics can only be proven by breaking the checked-in file and restoring it,
//! which is exactly the manual dance CI cannot perform.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::discovery::{self, Missing};
use super::model::{Matrix, EPIC_REQUIREMENTS, OWNERS, STATUSES, TIERS};

/// Everything wrong with a matrix, in a stable order.
pub fn violations(matrix: &Matrix, repo_root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    found.extend(structure(matrix));
    found.extend(coverage(matrix));
    found.extend(contradictions(matrix));
    found.extend(existence(matrix, repo_root));
    found.extend(super::ci::violations(matrix, repo_root));
    found.sort();
    found
}

/// Schema-level sanity: the vocabularies are closed and the gate field pairs
/// with the status that requires it.
fn structure(matrix: &Matrix) -> Vec<String> {
    let mut found = Vec::new();
    if matrix.schema_version != 1 {
        found.push(format!(
            "unsupported schema_version {}; this linter reads version 1",
            matrix.schema_version
        ));
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for requirement in &matrix.requirement {
        if !seen.insert(requirement.id.as_str()) {
            found.push(format!("{} is declared twice", requirement.id));
        }
        if requirement.summary.trim().is_empty() {
            found.push(format!("{} declares an empty summary", requirement.id));
        }
        // The issue asks for an owner beside the test mapping. An owner nobody
        // can look up is worse than none, so the vocabulary is closed to the
        // component teams this repository actually has.
        if !OWNERS.contains(&requirement.owner.as_str()) {
            found.push(format!(
                "{} names the unknown owner {:?}; use one of {OWNERS:?}",
                requirement.id, requirement.owner
            ));
        }
    }
    for row in &matrix.evidence {
        let at = format!("{} -> {}::{}", row.requirement, row.suite, row.test);
        if !TIERS.contains(&row.tier.as_str()) {
            found.push(format!("{at}: unknown tier {:?}", row.tier));
        }
        if !STATUSES.contains(&row.status.as_str()) {
            found.push(format!("{at}: unknown status {:?}", row.status));
        }
        match (row.status.as_str(), row.gate_env.as_deref()) {
            ("gated", None) => found.push(format!(
                "{at}: a gated row must name the environment variable that gates it"
            )),
            ("verified", Some(gate)) => found.push(format!(
                "{at}: a verified row must not claim the gate {gate}"
            )),
            _ => {}
        }
        if row.test.trim().is_empty() {
            found.push(format!("{at}: names no test"));
        }
    }
    found
}

/// Every epic requirement is declared, and every declared requirement carries at
/// least one evidence row whose requirement id actually exists.
fn coverage(matrix: &Matrix) -> Vec<String> {
    let mut found = Vec::new();
    let declared = matrix.requirement_set();
    let epic: BTreeSet<&str> = EPIC_REQUIREMENTS.into_iter().collect();
    for missing in epic.difference(&declared) {
        found.push(format!(
            "{missing} is in epic #5665's requirement table but not in the matrix"
        ));
    }
    for extra in declared.difference(&epic) {
        found.push(format!(
            "{extra} is in the matrix but not in epic #5665's requirement table"
        ));
    }
    let mut counts: BTreeMap<&str, usize> = declared.iter().map(|id| (*id, 0usize)).collect();
    for row in &matrix.evidence {
        match counts.get_mut(row.requirement.as_str()) {
            Some(count) => *count += 1,
            None => found.push(format!(
                "evidence names the undeclared requirement {}",
                row.requirement
            )),
        }
    }
    for (id, count) in counts {
        if count == 0 {
            found.push(format!("{id} maps to no automated test"));
        }
    }
    found
}

/// One test cannot be both verified and gated, cannot be claimed under two
/// tiers, and cannot be claimed twice for the same requirement.
fn contradictions(matrix: &Matrix) -> Vec<String> {
    let mut found = Vec::new();
    let mut by_test: BTreeMap<(&str, &str), (&str, &str)> = BTreeMap::new();
    let mut claimed: BTreeSet<(&str, &str, &str)> = BTreeSet::new();
    for row in &matrix.evidence {
        let key = (row.suite.as_str(), row.test.as_str());
        match by_test.get(&key) {
            Some((status, tier)) if *status != row.status || *tier != row.tier => {
                found.push(format!(
                    "{}::{} is claimed as {status}/{tier} and again as {}/{}",
                    row.suite, row.test, row.status, row.tier
                ));
            }
            Some(_) => {}
            None => {
                by_test.insert(key, (row.status.as_str(), row.tier.as_str()));
            }
        }
        if !claimed.insert((
            row.requirement.as_str(),
            row.suite.as_str(),
            row.test.as_str(),
        )) {
            found.push(format!(
                "{} claims {}::{} twice",
                row.requirement, row.suite, row.test
            ));
        }
    }
    found
}

/// Every named test resolves to a real definition in the working tree.
fn existence(matrix: &Matrix, repo_root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for row in &matrix.evidence {
        match discovery::find(repo_root, &row.suite, &row.test) {
            Ok(()) => {}
            Err(Missing::Suite) => found.push(format!(
                "{}: the suite {} does not exist",
                row.requirement, row.suite
            )),
            Err(Missing::Test) => found.push(format!(
                "{}: {} defines no test named {:?}",
                row.requirement, row.suite, row.test
            )),
            Err(Missing::UnknownSuiteKind) => found.push(format!(
                "{}: {} has no known test-definition form, so its evidence cannot \
                 be checked; claim a real test file instead",
                row.requirement, row.suite
            )),
        }
    }
    found
}
