//! Proves the matrix's `tier` column is a statement about CI, not a wish.
//!
//! The matrix defines `tier = "pr", status = "verified"` as "the named test runs
//! and passes in the tier's CI". Nothing enforced that: for a long stretch the
//! only pull-request workflow was the Rust one, so every frontend and deployment
//! row claimed a CI run that never happened. A reviewer reading the generated
//! evidence would have seen those rows presented exactly like the Rust ones.
//!
//! This pass closes the loop from the other end. Each suite family is mapped to
//! the command that executes it, and the check proves some workflow triggered by
//! a pull request actually runs that command. A new row for a family nobody runs
//! is then a build failure with a message naming the missing command, and
//! deleting the workflow step that runs Playwright breaks the rows that depend
//! on it rather than quietly downgrading them.
//!
//! It is a coarse check on purpose: it asserts the RUNNER exists, not that a
//! particular assertion inside it passed. The latter is what `cargo test`, the
//! Vitest run, and the Playwright run already do — a failing named test fails its
//! job, and the job is required. What nothing else could notice is a claim whose
//! job was never wired up at all.

use std::collections::BTreeSet;
use std::path::Path;

/// One suite family and the command that must run it on a pull request.
///
/// Ordered longest-prefix-first: `frontend/e2e/` must win over `frontend/`.
const RUNNERS: [(&str, &str); 6] = [
    ("backend/tests/", "cargo test"),
    ("backend/src/", "cargo test"),
    ("frontend/e2e/", "npm run test:e2e"),
    ("frontend/src/", "npm test"),
    ("deploy/kubernetes/tests/", "deploy/kubernetes/tests/"),
    (
        "deploy/kubernetes/",
        "deploy/kubernetes/validate-manifests.sh",
    ),
];

/// The command that must appear in a pull-request workflow for `suite`.
///
/// `None` means the path is not one of the known families, which is itself a
/// violation: an unrecognised suite family cannot be claimed as CI-verified,
/// because nobody has said what would run it.
pub fn required_command(suite: &str) -> Option<&'static str> {
    RUNNERS
        .into_iter()
        .find(|(prefix, _)| suite.starts_with(prefix))
        .map(|(_, command)| command)
}

/// Every command any pull-request workflow runs, as one blob per workflow.
///
/// The whole file is used as the haystack rather than a parsed step list: these
/// workflows are small, and a YAML parser here would buy precision this check
/// does not need while adding a dependency it would otherwise not have.
pub fn pull_request_workflow_text(repo_root: &Path) -> String {
    let dir = repo_root.join(".github/workflows");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return String::new();
    };
    let mut combined = String::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yml")
            && path.extension().and_then(|e| e.to_str()) != Some("yaml")
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Only workflows that actually trigger on a pull request can gate one.
        if !text.contains("pull_request") {
            continue;
        }
        combined.push_str(&text);
        combined.push('\n');
    }
    combined
}

/// Violations for the deterministic tier's CI claim.
pub fn violations(matrix: &super::model::Matrix, repo_root: &Path) -> Vec<String> {
    let workflows = pull_request_workflow_text(repo_root);
    let mut found = BTreeSet::new();
    for row in &matrix.evidence {
        if row.tier != "pr" || row.status != "verified" {
            continue;
        }
        match required_command(&row.suite) {
            None => {
                found.insert(format!(
                    "{}: {} is not in a suite family any workflow knows how to run; \
                     add it to acceptance::ci::RUNNERS together with the workflow step",
                    row.requirement, row.suite
                ));
            }
            Some(command) if !workflows.contains(command) => {
                found.insert(format!(
                    "{}: {} claims tier \"pr\" but no pull-request workflow runs {:?}",
                    row.requirement, row.suite, command
                ));
            }
            Some(_) => {}
        }
    }
    found.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acceptance::model::Matrix;

    fn root() -> std::path::PathBuf {
        crate::acceptance::repo_root()
    }

    /// The families the matrix actually uses all resolve to a command.
    #[test]
    fn every_known_family_names_its_runner() {
        assert_eq!(
            required_command("backend/tests/acceptance_canary.rs"),
            Some("cargo test")
        );
        assert_eq!(
            required_command("frontend/e2e/operations.spec.ts"),
            Some("npm run test:e2e")
        );
        assert_eq!(
            required_command("frontend/src/pages/operations.test.tsx"),
            Some("npm test")
        );
        assert_eq!(
            required_command("deploy/kubernetes/validate-monitoring.rb"),
            Some("deploy/kubernetes/validate-manifests.sh")
        );
        assert_eq!(required_command("docs/something.md"), None);
    }

    /// A row in a family nobody runs is reported, rather than passing silently.
    #[test]
    fn an_unrunnable_family_is_a_violation() {
        let document = Matrix::parse(&crate::acceptance::synthetic(
            r#"
            evidence = [
              { requirement = "AUTH-01", tier = "pr", suite = "docs/manual-checklist.md", test = "someone checks it", status = "verified" },
            ]
            "#,
        ))
        .expect("the synthetic document parses");
        let problems = violations(&document, &root());
        assert!(
            problems.iter().any(|p| p.contains("manual-checklist.md")),
            "{problems:?}"
        );
    }

    /// A family whose command is absent from every workflow is reported.
    #[test]
    fn a_family_no_workflow_runs_is_a_violation() {
        let document = Matrix::parse(&crate::acceptance::synthetic(
            r#"
            evidence = [
              { requirement = "AUTH-01", tier = "pr", suite = "frontend/e2e/operations.spec.ts", test = "every authenticated user finds Operations in the nav and can open it", status = "verified" },
            ]
            "#,
        ))
        .expect("the synthetic document parses");
        // An empty repository root has no workflows at all, so the same row that
        // passes against the real tree must fail here.
        let empty = tempfile::TempDir::new().expect("temp dir");
        let problems = violations(&document, empty.path());
        assert!(
            problems.iter().any(|p| p.contains("npm run test:e2e")),
            "{problems:?}"
        );
        assert!(
            violations(&document, &root()).is_empty(),
            "the real tree does run the end-to-end suite"
        );
    }
}
