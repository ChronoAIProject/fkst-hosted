//! Proves a named test actually exists in the file the matrix says owns it.
//!
//! Without this pass the matrix is a list of aspirations: a renamed Rust test, a
//! reworded Playwright title, or a deleted shell case would leave the claim
//! standing while the evidence quietly vanished. The check is deliberately
//! language-aware rather than a blanket substring match for Rust, because
//! `fn some_name(` is the one form that cannot be satisfied by a passing mention
//! in a comment.

use std::path::Path;

/// Why a claimed test could not be found.
#[derive(Debug, PartialEq, Eq)]
pub enum Missing {
    /// The suite file itself is absent.
    Suite,
    /// The suite exists but does not define the named test.
    Test,
}

/// Look for `test` inside `suite`, relative to the repository root.
pub fn find(repo_root: &Path, suite: &str, test: &str) -> Result<(), Missing> {
    let path = repo_root.join(suite);
    let Ok(source) = std::fs::read_to_string(&path) else {
        return Err(Missing::Suite);
    };
    let found = match extension(suite) {
        // A Rust test is a function definition; nothing else counts.
        "rs" => source.contains(&format!("fn {test}(")),
        // Playwright/Vitest titles are string literals. Both quote styles are in
        // use across the frontend suites, and a title may itself contain an
        // apostrophe, so match on the delimiter pair rather than the bare name.
        "ts" | "tsx" => {
            source.contains(&format!("'{test}'"))
                || source.contains(&format!("\"{test}\""))
                || source.contains(&format!("`{test}`"))
        }
        // Shell cases, Ruby constants, and YAML fixture keys are all literal
        // tokens in their own file; a bare containment check is the honest
        // strongest available form.
        _ => source.contains(test),
    };
    if found {
        Ok(())
    } else {
        Err(Missing::Test)
    }
}

fn extension(suite: &str) -> &str {
    Path::new(suite)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        crate::acceptance::repo_root()
    }

    #[test]
    fn a_real_rust_test_is_found_and_a_renamed_one_is_not() {
        assert_eq!(
            find(
                &root(),
                "backend/tests/audit_outcomes.rs",
                "a_route_scoped_timeout_is_recorded_as_a_timeout"
            ),
            Ok(())
        );
        assert_eq!(
            find(
                &root(),
                "backend/tests/audit_outcomes.rs",
                "a_route_scoped_timeout_was_renamed"
            ),
            Err(Missing::Test)
        );
    }

    #[test]
    fn an_absent_suite_is_distinguished_from_an_absent_test() {
        assert_eq!(
            find(&root(), "backend/tests/no_such_suite.rs", "anything"),
            Err(Missing::Suite)
        );
    }

    #[test]
    fn a_frontend_title_is_matched_by_its_quoted_literal() {
        assert_eq!(
            find(
                &root(),
                "frontend/src/pages/operations.security.test.tsx",
                "refreshes sandboxes every 5 seconds"
            ),
            Ok(())
        );
        assert_eq!(
            find(
                &root(),
                "frontend/src/pages/operations.security.test.tsx",
                "refreshes sandboxes every 6 seconds"
            ),
            Err(Missing::Test)
        );
    }
}
