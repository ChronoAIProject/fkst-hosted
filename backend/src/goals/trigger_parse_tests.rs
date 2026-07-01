//! Tests for [`super`] (the `fkst-substrate-trigger` body parser). Split into a
//! sibling file to keep `trigger_parse.rs` under the 500-line limit; included via
//! `#[cfg(test)] #[path = "trigger_parse_tests.rs"] mod tests;`.

use super::*;

/// A valid, minimal package reference reused by tests whose focus is a DIFFERENT
/// section, so their `### Packages` block never fails first.
const VALID_PKG: &str = "o/r@dev:pkg";

/// Build a trigger body whose `### Packages` section is exactly `pkg`, with the
/// other required sections held valid — so a 422 can only come from `pkg`.
fn body_with_package(pkg: &str) -> String {
    format!("### Session Name\nsess\n### Packages\n{pkg}\n### Work Label\nlabel\n")
}

/// Parse a body carrying the single package line `pkg`, asserting exactly one
/// [`PackageRef`] parses, and return it.
fn parse_one_package(pkg: &str) -> PackageRef {
    let spec = parse_trigger_issue_body(&body_with_package(pkg)).expect("valid package ref parses");
    assert_eq!(spec.packages.len(), 1, "expected exactly one package");
    spec.packages.into_iter().next().expect("one package")
}

fn err_message(body: &str) -> String {
    match parse_trigger_issue_body(body) {
        Err(AppError::Unprocessable(msg)) => msg,
        other => panic!("expected Unprocessable (422), got {other:?}"),
    }
}

// ---- Happy paths ----

/// A fully-populated body parses to the documented [`TriggerSpec`], preserving the
/// package order and reading the optional environment.
#[test]
fn worked_example_parses_all_four_sections() {
    let body = "\
### Session Name

my-session

### Packages

ChronoAIProject/fkst-packages@dev:packages/github-devloop
acme/tools@v1.0.0:pkg/thing

### Work Label

fkst-cloud

### Environment

prod-env
";
    let spec = parse_trigger_issue_body(body).expect("worked example parses");
    assert_eq!(
        spec,
        TriggerSpec {
            name: "my-session".to_string(),
            packages: vec![
                PackageRef {
                    owner: "ChronoAIProject".to_string(),
                    repo: "fkst-packages".to_string(),
                    git_ref: "dev".to_string(),
                    path: "packages/github-devloop".to_string(),
                },
                PackageRef {
                    owner: "acme".to_string(),
                    repo: "tools".to_string(),
                    git_ref: "v1.0.0".to_string(),
                    path: "pkg/thing".to_string(),
                },
            ],
            work_label: "fkst-cloud".to_string(),
            environment: Some("prod-env".to_string()),
        }
    );
}

/// The `@ref` accepts a branch, a semver tag, a bare SHA, and a `/`-namespaced
/// branch — each preserved verbatim in [`PackageRef::git_ref`].
#[test]
fn package_ref_accepts_branch_tag_and_sha_refs() {
    assert_eq!(parse_one_package("owner/repo@dev:pkg/dir").git_ref, "dev");
    assert_eq!(
        parse_one_package("owner/repo@v1.2.3:pkg/dir").git_ref,
        "v1.2.3"
    );
    assert_eq!(
        parse_one_package("owner/repo@a1b2c3d4e5f6:pkg/dir").git_ref,
        "a1b2c3d4e5f6"
    );
    assert_eq!(
        parse_one_package("owner/repo@feature/foo:pkg").git_ref,
        "feature/foo",
        "a `/`-namespaced branch ref is allowed"
    );
}

#[test]
fn absent_environment_section_yields_none() {
    let body =
        format!("### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n");
    let spec = parse_trigger_issue_body(&body).expect("parses without Environment");
    assert!(spec.environment.is_none());
}

#[test]
fn intro_before_first_heading_is_ignored() {
    let body = format!(
        "Form intro the user never edits\n\n### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n"
    );
    let spec = parse_trigger_issue_body(&body).expect("intro ignored");
    assert_eq!(spec.name, "sess");
}

// ---- Session Name (each names the offending section in the 422) ----

#[test]
fn missing_session_name_is_422_naming_the_section() {
    let msg = err_message(&format!(
        "### Packages\n{VALID_PKG}\n### Work Label\nlabel\n"
    ));
    assert!(msg.contains("Session Name"), "must name the section: {msg}");
}

#[test]
fn multiline_session_name_is_422_naming_the_section() {
    let body = format!(
        "### Session Name\nfirst\nsecond\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n"
    );
    let msg = err_message(&body);
    assert!(msg.contains("Session Name"), "must name the section: {msg}");
    assert!(msg.contains("exactly one"), "must flag the count: {msg}");
}

#[test]
fn invalid_session_name_chars_is_422_naming_the_value() {
    let body =
        format!("### Session Name\nMy_Session\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n");
    let msg = err_message(&body);
    assert!(msg.contains("Session Name"), "must name the section: {msg}");
    assert!(msg.contains("My_Session"), "must name the value: {msg}");
}

// ---- Packages (each names the section; malformed lines also name the value) ----

#[test]
fn missing_packages_lines_is_422_naming_the_section() {
    // The heading is present but has zero non-empty lines.
    let body = "### Session Name\nsess\n### Packages\n\n### Work Label\nlabel\n";
    let msg = err_message(body);
    assert!(msg.contains("Packages"), "must name the section: {msg}");
    assert!(
        msg.contains("at least one"),
        "must flag the emptiness: {msg}"
    );
}

#[test]
fn package_ref_missing_at_is_422_naming_the_value_and_form() {
    let msg = err_message(&body_with_package("owner/repo:dev:path"));
    assert!(msg.contains("Packages"), "must name the section: {msg}");
    assert!(
        msg.contains("owner/repo:dev:path"),
        "must echo the value: {msg}"
    );
    assert!(msg.contains('@'), "must flag the missing `@`: {msg}");
    assert!(
        msg.contains("owner/repo@ref:path"),
        "must recall the expected form: {msg}"
    );
}

#[test]
fn package_ref_missing_colon_is_422_naming_the_value_and_form() {
    let msg = err_message(&body_with_package("owner/repo@dev"));
    assert!(msg.contains("Packages"), "must name the section: {msg}");
    assert!(msg.contains("owner/repo@dev"), "must echo the value: {msg}");
    assert!(
        msg.contains("owner/repo@ref:path"),
        "must recall the expected form: {msg}"
    );
}

#[test]
fn package_ref_empty_owner_is_422_naming_the_value() {
    let msg = err_message(&body_with_package("/repo@dev:path"));
    assert!(msg.contains("Packages"), "must name the section: {msg}");
    assert!(msg.contains("/repo@dev:path"), "must echo the value: {msg}");
    assert!(msg.contains("owner"), "must flag which part failed: {msg}");
}

#[test]
fn package_ref_two_slashes_before_at_is_422() {
    let msg = err_message(&body_with_package("a/b/c@dev:path"));
    assert!(msg.contains("Packages"), "must name the section: {msg}");
    assert!(msg.contains("a/b/c@dev:path"), "must echo the value: {msg}");
    assert!(
        msg.contains("single `/`"),
        "must flag the slash count: {msg}"
    );
}

#[test]
fn package_ref_zero_slashes_before_at_is_422() {
    let msg = err_message(&body_with_package("ownerrepo@dev:path"));
    assert!(msg.contains("Packages"), "must name the section: {msg}");
    assert!(
        msg.contains("ownerrepo@dev:path"),
        "must echo the value: {msg}"
    );
    assert!(
        msg.contains("single `/`"),
        "must flag the slash count: {msg}"
    );
}

#[test]
fn package_ref_dotdot_in_ref_is_422_naming_the_value() {
    let msg = err_message(&body_with_package("o/r@foo/../bar:path"));
    assert!(msg.contains("Packages"), "must name the section: {msg}");
    assert!(
        msg.contains("o/r@foo/../bar:path"),
        "must echo the value: {msg}"
    );
    assert!(msg.contains("ref"), "must flag the ref part: {msg}");
    assert!(msg.contains(".."), "must flag the traversal: {msg}");
}

#[test]
fn package_ref_dotdot_in_path_is_422_naming_the_value() {
    let msg = err_message(&body_with_package("o/r@dev:foo/../bar"));
    assert!(msg.contains("Packages"), "must name the section: {msg}");
    assert!(
        msg.contains("o/r@dev:foo/../bar"),
        "must echo the value: {msg}"
    );
    assert!(msg.contains("path"), "must flag the path part: {msg}");
    assert!(msg.contains(".."), "must flag the traversal: {msg}");
}

#[test]
fn package_ref_leading_slash_in_path_is_422_naming_the_value() {
    let msg = err_message(&body_with_package("o/r@dev:/abs/path"));
    assert!(msg.contains("Packages"), "must name the section: {msg}");
    assert!(
        msg.contains("o/r@dev:/abs/path"),
        "must echo the value: {msg}"
    );
    assert!(msg.contains("path"), "must flag the path part: {msg}");
    assert!(
        msg.contains("start with `/`"),
        "must flag the absolute path: {msg}"
    );
}

#[test]
fn package_ref_illegal_space_char_is_422_naming_the_value() {
    let msg = err_message(&body_with_package("o/r@dev:bad path"));
    assert!(msg.contains("Packages"), "must name the section: {msg}");
    assert!(
        msg.contains("o/r@dev:bad path"),
        "must echo the value: {msg}"
    );
    assert!(msg.contains("path"), "must flag the path part: {msg}");
}

// ---- Work Label (each names the offending section in the 422) ----

#[test]
fn missing_work_label_is_422_naming_the_section() {
    let msg = err_message(&format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n"
    ));
    assert!(msg.contains("Work Label"), "must name the section: {msg}");
}

#[test]
fn multiline_work_label_is_422_naming_the_section() {
    let body =
        format!("### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\none\ntwo\n");
    let msg = err_message(&body);
    assert!(msg.contains("Work Label"), "must name the section: {msg}");
    assert!(msg.contains("exactly one"), "must flag the count: {msg}");
}

#[test]
fn work_label_with_comma_is_422_naming_the_section() {
    let body =
        format!("### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nred, blue\n");
    let msg = err_message(&body);
    assert!(msg.contains("Work Label"), "must name the section: {msg}");
    assert!(msg.contains("comma"), "must flag the comma: {msg}");
}

#[test]
fn duplicate_work_label_heading_is_422() {
    let body = format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nx\n### Work Label\ny\n"
    );
    let msg = err_message(&body);
    assert!(msg.contains("duplicate"), "must flag the duplicate: {msg}");
    assert!(msg.contains("Work Label"), "must name the section: {msg}");
}

// ---- Environment (optional; two names is ambiguous) ----

#[test]
fn environment_with_two_lines_is_422_naming_the_section() {
    let body = format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n### Environment\nfirst\nsecond\n"
    );
    let msg = err_message(&body);
    assert!(msg.contains("Environment"), "must name the section: {msg}");
    assert!(
        msg.contains("exactly one"),
        "must flag the ambiguity: {msg}"
    );
}
