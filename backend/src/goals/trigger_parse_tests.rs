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
            work_label: Some("fkst-cloud".to_string()),
            environment: Some("prod-env".to_string()),
            auto_merge: false,
            log_access: vec![],
            collaborators: vec![],
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
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

/// Build a body with the four required sections held valid plus an `### Auto-merge`
/// section carrying `val`, so a parse's `auto_merge` reflects only that value.
fn body_with_auto_merge(val: &str) -> String {
    format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n### Auto-merge\n{val}\n"
    )
}

#[test]
fn auto_merge_true_variants() {
    for val in ["true", "YES", "On", "enabled", "1"] {
        let spec = parse_trigger_issue_body(&body_with_auto_merge(val)).expect("parses");
        assert!(spec.auto_merge, "{val:?} must enable auto-merge");
    }
}

#[test]
fn auto_merge_false_variants() {
    for val in ["false", "no", "off", "", "maybe"] {
        let spec = parse_trigger_issue_body(&body_with_auto_merge(val)).expect("parses");
        assert!(!spec.auto_merge, "{val:?} must leave auto-merge off");
    }
}

#[test]
fn auto_merge_absent_defaults_false() {
    // The minimal body carries no `### Auto-merge` section at all.
    let spec = parse_trigger_issue_body(&body_with_package(VALID_PKG)).expect("parses");
    assert!(!spec.auto_merge, "an absent section defaults off");
}

// ---- Log Access Allowlist (optional allow-list; lenient, never a 422) ----

/// Build a body with the four required sections held valid plus a `### Log Access Allowlist`
/// section carrying `val`, so a parse's `log_access` reflects only that value.
fn body_with_log_access(val: &str) -> String {
    format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n### Log Access Allowlist\n{val}\n"
    )
}

#[test]
fn log_access_absent_defaults_empty() {
    let spec = parse_trigger_issue_body(&body_with_package(VALID_PKG)).expect("parses");
    assert!(
        spec.log_access.is_empty(),
        "an absent `### Log Access Allowlist` section defaults to an empty allow-list"
    );
}

#[test]
fn log_access_parses_comma_whitespace_and_newline_separated_tokens() {
    // A mix of commas, spaces, and newlines all separate tokens; a leading `@` is
    // stripped; numeric ids are kept verbatim.
    let spec = parse_trigger_issue_body(&body_with_log_access("@alice, bob   carol\n12345"))
        .expect("parses");
    assert_eq!(
        spec.log_access,
        vec![
            "alice".to_string(),
            "bob".to_string(),
            "carol".to_string(),
            "12345".to_string(),
        ]
    );
}

#[test]
fn log_access_blank_section_is_empty() {
    let spec = parse_trigger_issue_body(&body_with_log_access("   \n\n")).expect("parses");
    assert!(
        spec.log_access.is_empty(),
        "a blank `### Log Access Allowlist` section yields an empty allow-list"
    );
}

// ---- Session Collaborators (optional work-item authority list; lenient) ----

/// Build a body with the required sections held valid plus a
/// `### Session Collaborators` section carrying `val`, so a parse's
/// `collaborators` reflects only that value.
fn body_with_collaborators(val: &str) -> String {
    format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n### Session Collaborators\n{val}\n"
    )
}

#[test]
fn collaborators_absent_defaults_empty() {
    let spec = parse_trigger_issue_body(&body_with_package(VALID_PKG)).expect("parses");
    assert!(
        spec.collaborators.is_empty(),
        "an absent `### Session Collaborators` section defaults to an empty list"
    );
}

#[test]
fn collaborators_parse_comma_whitespace_and_newline_separated_tokens() {
    // Commas, spaces, and newlines all separate tokens; a leading `@` is stripped.
    let spec = parse_trigger_issue_body(&body_with_collaborators("@alice, bob   carol\ndave"))
        .expect("parses");
    assert_eq!(
        spec.collaborators,
        vec![
            "alice".to_string(),
            "bob".to_string(),
            "carol".to_string(),
            "dave".to_string(),
        ]
    );
}

#[test]
fn collaborators_are_deduped_case_insensitively_first_spelling_wins() {
    let spec = parse_trigger_issue_body(&body_with_collaborators("Alice, alice, @ALICE, bob"))
        .expect("parses");
    assert_eq!(
        spec.collaborators,
        vec!["Alice".to_string(), "bob".to_string()],
        "case-insensitive dedupe keeps the first spelling"
    );
}

#[test]
fn collaborators_blank_section_is_empty() {
    let spec = parse_trigger_issue_body(&body_with_collaborators("   \n\n")).expect("parses");
    assert!(
        spec.collaborators.is_empty(),
        "a blank `### Session Collaborators` section yields an empty list"
    );
}

#[test]
fn collaborators_and_log_access_are_independent_lists() {
    // The two trusted-user lists are parsed separately and never bleed into each
    // other — collaborators come only from `### Session Collaborators`.
    let body = format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n\
         ### FKST Contributors\nlogviewer\n### Session Collaborators\n@worker\n"
    );
    let spec = parse_trigger_issue_body(&body).expect("both sections parse");
    assert_eq!(spec.log_access, vec!["logviewer".to_string()]);
    assert_eq!(spec.collaborators, vec!["worker".to_string()]);
}

#[test]
fn duplicate_collaborators_heading_is_422() {
    // Matches every other section: a duplicate `### ` heading is a 422 naming it.
    let body = format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n\
         ### Session Collaborators\nx\n### Session Collaborators\ny\n"
    );
    let msg = err_message(&body);
    assert!(msg.contains("duplicate"), "must flag the duplicate: {msg}");
    assert!(
        msg.contains("Session Collaborators"),
        "must name the section: {msg}"
    );
}

#[test]
fn collaborators_comment_only_section_parses_to_empty() {
    // A PRISTINE section (comment-only, even one whose prose MENTIONS @logins) must
    // yield an EMPTY list: the HTML comment is stripped before tokenizing, so no
    // garbage/@-mention word-tokens leak into the frozen authority list.
    let spec = parse_trigger_issue_body(&body_with_collaborators(
        "<!--\nOptional. Add @alice, @bob — one per line. Delete to trust only yourself.\n-->",
    ))
    .expect("comment-only section parses");
    assert!(
        spec.collaborators.is_empty(),
        "a comment-only Session Collaborators section must parse to an empty list"
    );
}

#[test]
fn collaborators_comment_plus_value_parses_only_the_value() {
    // The comment (including its @mention prose) is stripped; only the real value
    // lines survive.
    let spec = parse_trigger_issue_body(&body_with_collaborators(
        "<!-- Optional. @ignored-in-comment -->\n@worker, @second",
    ))
    .expect("comment + value parses");
    assert_eq!(
        spec.collaborators,
        vec!["worker".to_string(), "second".to_string()]
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
fn missing_work_label_is_optional_and_parses_to_none() {
    // The `### Work Label` section is OPTIONAL since auto-discovery: absent → the
    // trigger parses with `work_label: None` (wake labels come from the packages'
    // `[github].work_labels`). (Previously this was a 422.)
    let spec = parse_trigger_issue_body(&format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n"
    ))
    .expect("a trigger with no Work Label section is valid");
    assert_eq!(spec.work_label, None);
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

// ---- Output Language (optional but STRICT; comment-tolerant) ----

/// Build a body with the three required sections held valid plus a
/// `### Output Language` section carrying `val`.
fn body_with_output_language(val: &str) -> String {
    format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n### Output Language\n{val}\n"
    )
}

#[test]
fn output_language_absent_defaults_none() {
    let spec = parse_trigger_issue_body(&body_with_package(VALID_PKG)).expect("parses");
    assert_eq!(spec.output_lang, None, "an absent section defaults to None");
}

#[test]
fn output_language_accepts_conservative_locale_tags() {
    for lang in ["en", "zh", "cmn", "zh-CN", "zh_TW", "pt-br20"] {
        let spec =
            parse_trigger_issue_body(&body_with_output_language(lang)).expect("valid locale");
        assert_eq!(spec.output_lang.as_deref(), Some(lang), "{lang}");
    }
}

#[test]
fn output_language_blank_or_comment_only_is_none() {
    // Blank section.
    let spec = parse_trigger_issue_body(&body_with_output_language("   \n")).expect("parses");
    assert_eq!(spec.output_lang, None);
    // The PRISTINE template shape: a multi-line explanatory HTML comment and no
    // value. Stripping the comment must leave nothing — not a 422.
    let spec = parse_trigger_issue_body(&body_with_output_language(
        "<!--\nOptional. One locale tag, e.g. zh.\n-->",
    ))
    .expect("pristine template section parses");
    assert_eq!(spec.output_lang, None);
}

#[test]
fn output_language_comment_plus_value_parses_the_value() {
    let spec = parse_trigger_issue_body(&body_with_output_language(
        "<!--\nOptional. One locale tag.\n-->\nzh",
    ))
    .expect("comment + value parses");
    assert_eq!(spec.output_lang.as_deref(), Some("zh"));
}

#[test]
fn output_language_invalid_locale_is_422_naming_the_section() {
    for bad in ["ZH", "zh cn", "zh/../x", "a", "toolong-abcdefghij"] {
        let msg = err_message(&body_with_output_language(bad));
        assert!(
            msg.contains("Output Language"),
            "{bad}: must name the section: {msg}"
        );
    }
}

#[test]
fn output_language_two_values_is_422() {
    let msg = err_message(&body_with_output_language("en\nzh"));
    assert!(msg.contains("Output Language"), "names the section: {msg}");
    assert!(msg.contains("at most one"), "flags the ambiguity: {msg}");
}

// ---- Engine Config (optional but STRICT; allowlisted; comment-tolerant) ----

#[test]
fn engine_config_absent_defaults_empty() {
    let spec = parse_trigger_issue_body(&body_with_package(VALID_PKG)).expect("parses");
    assert!(spec.engine_config.is_empty());
}

#[test]
fn engine_config_section_parses_allowlisted_pairs() {
    let body = format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n\
         ### Engine Config\n<!--\nOne KEY=value per line.\n-->\nFKST_CODEX_PERMIT_SLOTS=8\nFKST_RATE_POOL_GH=10,10\n"
    );
    let spec = parse_trigger_issue_body(&body).expect("parses");
    assert_eq!(spec.engine_config.len(), 2);
    assert_eq!(spec.engine_config["FKST_CODEX_PERMIT_SLOTS"], "8");
    assert_eq!(spec.engine_config["FKST_RATE_POOL_GH"], "10,10");
}

#[test]
fn engine_config_violations_are_422_from_the_whole_body_parse() {
    let body = format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n\
         ### Engine Config\nFKST_RUNTIME_ROOT=/tmp/x\n"
    );
    let msg = err_message(&body);
    assert!(msg.contains("Engine Config"), "names the section: {msg}");
    assert!(msg.contains("FKST_RUNTIME_ROOT"), "names the key: {msg}");
}

#[test]
fn the_full_pristine_bundled_template_parses_with_its_sample_values() {
    // The bug (#480): an author who fills the form values but KEEPS the
    // template's explanatory comments must never 422 — previously the strict
    // required sections counted their own template comment as a content line.
    let body = include_str!("../github_app/templates_assets/fkst-substrate-session.md");
    let spec = parse_trigger_issue_body(body).expect("the pristine template must parse");
    assert_eq!(spec.name, "my-first-session");
    assert_eq!(spec.work_label.as_deref(), Some("fkst-work"));
    assert_eq!(spec.packages.len(), 1);
    assert_eq!(spec.packages[0].repo, "fkst-packages");
    assert_eq!(spec.environment, None, "comment-only section is unset");
    assert!(!spec.auto_merge, "the template ships `false`");
    // The pristine `### Session Collaborators` section is comment-only; the parser
    // strips the comment before tokenizing, so it parses to an EMPTY list — no
    // comment prose (or its @mentions) leaks into the frozen authority list.
    assert!(
        spec.collaborators.is_empty(),
        "the template's comment-only Session Collaborators section must parse to empty"
    );
    assert_eq!(spec.output_lang, None);
    assert!(spec.engine_config.is_empty());
}

#[test]
fn strict_sections_parse_comment_plus_value() {
    // Comment + value in each previously-sharp strict section.
    let body = "### Session Name
<!-- one line -->
sess
### Packages
<!--
refs
-->
acme/tools@main:pkg/a
### Work Label
<!-- label -->
label
### Environment
<!-- optional -->
staging
";
    let spec = parse_trigger_issue_body(body).expect("comment + value parses");
    assert_eq!(spec.name, "sess");
    assert_eq!(spec.packages.len(), 1);
    assert_eq!(spec.work_label.as_deref(), Some("label"));
    assert_eq!(spec.environment.as_deref(), Some("staging"));
}

#[test]
fn the_bundled_templates_new_sections_parse_unset_verbatim() {
    // The EXACT bundled text of the two NEW sections (explanatory comments and
    // all) must parse to "not set" — never a 422 that punishes an author who
    // kept the template's comments. The tail of the bundled asset (from
    // `### Output Language` onward) is spliced verbatim onto valid required
    // sections, so this test breaks if the asset's new-section text ever stops
    // being comment-only or stops stripping cleanly.
    let template = include_str!("../github_app/templates_assets/fkst-substrate-session.md");
    let new_sections_start = template
        .find("### Output Language")
        .expect("the bundled template carries the Output Language section");
    let body = format!(
        "### Session Name
sess
### Packages
{VALID_PKG}
### Work Label
label
{}",
        &template[new_sections_start..]
    );
    let spec = parse_trigger_issue_body(&body).expect("the bundled new-section text must parse");
    assert_eq!(spec.output_lang, None, "comment-only section is unset");
    assert!(
        spec.engine_config.is_empty(),
        "comment-only section is an empty map"
    );
}

#[test]
fn fkst_contributors_heading_parses_and_legacy_heading_stays_an_alias() {
    // New heading.
    let body = format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n### FKST Contributors\n@alice, bob\n"
    );
    let spec = parse_trigger_issue_body(&body).expect("parses");
    assert_eq!(
        spec.log_access,
        vec!["alice".to_string(), "bob".to_string()]
    );
    // Legacy heading parses BYTE-IDENTICALLY (load-bearing: live issue bodies
    // re-parse every tick — a changed result would flip their full hash).
    let spec = parse_trigger_issue_body(&body_with_log_access("@alice, bob")).expect("parses");
    assert_eq!(
        spec.log_access,
        vec!["alice".to_string(), "bob".to_string()]
    );
}

#[test]
fn fkst_contributors_and_legacy_sections_merge_deduped() {
    let body = format!(
        "### Session Name\nsess\n### Packages\n{VALID_PKG}\n### Work Label\nlabel\n\
         ### FKST Contributors\nalice\nBob\n### Log Access Allowlist\nbob, carol\n"
    );
    let spec = parse_trigger_issue_body(&body).expect("both headings parse");
    // Current heading first; `bob` deduped case-insensitively (first spelling wins).
    assert_eq!(
        spec.log_access,
        vec!["alice".to_string(), "Bob".to_string(), "carol".to_string()]
    );
}

#[test]
fn work_label_section_is_optional() {
    // Absent `### Work Label` → None (labels auto-discovered from packages).
    let body = "### Session Name\nsess\n### Packages\no/r@dev:pkg\n";
    let spec = parse_trigger_issue_body(body).expect("no Work Label section is valid");
    assert_eq!(spec.work_label, None);

    // Present-but-blank section is also None, not a 422.
    let blank = "### Session Name\nsess\n### Packages\no/r@dev:pkg\n### Work Label\n\n";
    let spec = parse_trigger_issue_body(blank).expect("blank Work Label is valid");
    assert_eq!(spec.work_label, None);

    // Present + named still parses to Some.
    let named = "### Session Name\nsess\n### Packages\no/r@dev:pkg\n### Work Label\nfkst-x\n";
    let spec = parse_trigger_issue_body(named).expect("named Work Label is valid");
    assert_eq!(spec.work_label.as_deref(), Some("fkst-x"));
}
