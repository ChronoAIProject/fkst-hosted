//! Renders the milestone evidence artifact.
//!
//! The artifact exists so a reviewer can see, in one page, which requirement is
//! held up by which named test, at which tier, and against which build — without
//! reading 200 test files. It deliberately contains no request payload, no event
//! arguments, no user id, and no credential: it is generated FROM the matrix,
//! which only ever names files and tests, so there is nothing sensitive for it
//! to leak. The forbidden-substring assertion in the gate proves that stays true
//! rather than assuming it.

use std::fmt::Write as _;
use std::path::Path;

use super::model::Matrix;

/// The build the evidence describes.
///
/// Falls back to `unknown` rather than failing: a source tarball with no `.git`
/// is a legitimate way to run the suite, and a missing commit is honest, whereas
/// a fabricated one would not be.
pub fn build_commit(repo_root: &Path) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => "unknown".to_string(),
    }
}

/// Render the matrix as a compact evidence table.
pub fn render(matrix: &Matrix, build_commit: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Milestone {} acceptance evidence (epic #{}, gate #{})",
        matrix.milestone, matrix.epic, matrix.gate_issue
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "build_commit: {build_commit}");
    let _ = writeln!(out, "requirements: {}", matrix.requirement.len());
    let _ = writeln!(out, "evidence_rows: {}", matrix.evidence.len());
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "This artifact names requirement ids, suites, and test names only. It \
         carries no request payload, event argument, user id, or credential."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "| requirement | tier | result | suite | test |");
    let _ = writeln!(out, "|---|---|---|---|---|");
    for requirement in &matrix.requirement {
        for row in matrix.evidence_for(&requirement.id) {
            let result = match row.status.as_str() {
                "verified" => "pass".to_string(),
                "gated" => format!("gated:{}", row.gate_env.as_deref().unwrap_or("unspecified")),
                other => other.to_string(),
            };
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} |",
                requirement.id, row.tier, result, row.suite, row.test
            );
        }
    }
    out
}

/// Write `contents` to `artifact_dir/name`, creating the directory.
pub fn write(
    artifact_dir: &Path,
    name: &str,
    contents: &str,
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(artifact_dir)?;
    let path = artifact_dir.join(name);
    std::fs::write(&path, contents)?;
    Ok(path)
}

/// Substrings that must never appear in any generated evidence.
///
/// The list is the union of the credential families the epic forbids and the
/// per-record fields that would turn an evidence artifact into a data export.
pub const FORBIDDEN_IN_EVIDENCE: [&str; 16] = [
    "Authorization:",
    "Bearer ",
    "ghp_",
    "ghs_",
    "github_pat_",
    "phc_",
    "phx_",
    "client_secret",
    "refresh_token",
    "access_token",
    "X-Hub-Signature",
    "OPEN-SANDBOX-API-KEY",
    "actor_id=",
    "distinct_id",
    "-----BEGIN",
    "canary-",
];

/// Scan generated evidence for anything forbidden.
pub fn forbidden_hits(contents: &str) -> Vec<&'static str> {
    FORBIDDEN_IN_EVIDENCE
        .into_iter()
        .filter(|needle| contents.contains(needle))
        .collect()
}
