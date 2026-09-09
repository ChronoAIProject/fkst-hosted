use super::*;

const BODY: &str = "### Workflow\ngithub-candidate-sourcing\n\n### Run Mode\ncron: 0 1 * * 1-5\n\n### Arguments\nrole: AI Tools Application Engineer\nmin_score: 6\n";

fn reject(body: &str) -> String {
    match parse_scheduled_workflow(body) {
        Err(AppError::Unprocessable(message)) => message,
        other => panic!("expected an unprocessable parse error, got {other:?}"),
    }
}

#[test]
fn a_valid_body_parses_with_its_arguments() {
    let spec = parse_scheduled_workflow(BODY).expect("valid body");
    assert_eq!(spec.workflow_id, "github-candidate-sourcing");
    assert_eq!(spec.run_mode.render(), "cron: 0 1 * * 1-5");
    assert_eq!(
        spec.arguments.get("role").map(String::as_str),
        Some("AI Tools Application Engineer")
    );
    assert_eq!(
        spec.arguments.get("min_score").map(String::as_str),
        Some("6")
    );
}

#[test]
fn both_run_modes_round_trip() {
    let once = BODY.replace("cron: 0 1 * * 1-5", "once");
    assert_eq!(
        parse_scheduled_workflow(&once)
            .expect("once parses")
            .run_mode,
        RunMode::Once
    );
    assert_eq!(
        parse_scheduled_workflow(&once)
            .expect("once parses")
            .run_mode
            .render(),
        "once"
    );
    let cron = parse_scheduled_workflow(BODY)
        .expect("cron parses")
        .run_mode;
    assert_eq!(cron.render(), "cron: 0 1 * * 1-5");
    // Spelling tolerance: the prefix is case-insensitive and the space optional.
    for spelling in [
        "CRON: 0 1 * * 1-5",
        "cron:0 1 * * 1-5",
        "Cron:  0 1 * * 1-5",
    ] {
        let body = BODY.replace("cron: 0 1 * * 1-5", spelling);
        assert_eq!(
            parse_scheduled_workflow(&body)
                .expect("spelling parses")
                .run_mode,
            cron
        );
    }
}

#[test]
fn arguments_are_optional() {
    let body = "### Workflow\nsourcing\n\n### Run Mode\nonce\n";
    let spec = parse_scheduled_workflow(body).expect("no arguments is valid");
    assert!(spec.arguments.is_empty());
}

#[test]
fn a_duplicate_heading_is_rejected_by_the_shared_section_contract() {
    let body = format!("{BODY}\n### Workflow\nsecond\n");
    assert!(reject(&body).contains("duplicate"));
}

#[test]
fn an_unknown_section_is_rejected_naming_it() {
    // `### Schedule` is the trigger-era heading an author is most likely to reach
    // for; silently ignoring it would leave the job running on the wrong cadence.
    let body = format!("{BODY}\n### Schedule\ncron: 0 2 * * *\n");
    let message = reject(&body);
    assert!(message.contains("### Schedule"), "{message}");
    assert!(
        message.contains("### Run Mode"),
        "names the right one: {message}"
    );
}

#[test]
fn the_required_sections_are_required() {
    for heading in ["### Workflow", "### Run Mode"] {
        let body = BODY.replace(heading, "### Arguments");
        let message = reject(&body);
        assert!(
            message.contains(heading) || message.contains("duplicate"),
            "removing {heading} must be rejected: {message}"
        );
    }
}

#[test]
fn an_invalid_cron_names_the_field() {
    let body = BODY.replace("0 1 * * 1-5", "0 1 * * 9");
    let message = reject(&body);
    assert!(message.contains("day-of-week"), "{message}");
}

#[test]
fn an_unrecognised_run_mode_is_rejected() {
    let body = BODY.replace("cron: 0 1 * * 1-5", "every monday");
    let message = reject(&body);
    assert!(message.contains("### Run Mode"), "{message}");
    assert!(
        message.contains("once"),
        "states the accepted values: {message}"
    );
}

#[test]
fn a_workflow_id_must_be_path_safe() {
    for id in [
        "../escape",
        "..",
        ".hidden",
        "trailing-",
        "with space",
        "with/slash",
        &"a".repeat(MAX_WORKFLOW_ID_BYTES + 1),
    ] {
        let body = BODY.replace("github-candidate-sourcing", id);
        let message = reject(&body);
        assert!(
            message.contains("### Workflow"),
            "id {id:?} must be rejected by the workflow section: {message}"
        );
    }
    // The legitimate shapes stay accepted.
    for id in ["sourcing", "github-candidate-sourcing", "v2.report_daily"] {
        let body = BODY.replace("github-candidate-sourcing", id);
        assert_eq!(
            parse_scheduled_workflow(&body)
                .expect("valid id")
                .workflow_id,
            id
        );
    }
}

#[test]
fn argument_keys_and_values_are_shape_checked() {
    let cases = [
        ("role: AI\n9bad: x", "invalid key"),
        ("role: AI\nbad-key: x", "invalid key"),
        ("role: AI\nempty:", "empty"),
        ("role: AI\nrole: again", "more than once"),
        ("no separator here", "`:` separator"),
    ];
    for (arguments, expected) in cases {
        let body = BODY.replace(
            "role: AI Tools Application Engineer\nmin_score: 6",
            arguments,
        );
        let message = reject(&body);
        assert!(
            message.contains(expected),
            "arguments {arguments:?} should mention {expected:?}: {message}"
        );
    }
}

#[test]
fn the_argument_count_and_size_caps_are_enforced() {
    let many: String = (0..=MAX_ARGUMENTS)
        .map(|index| format!("key_{index}: value\n"))
        .collect();
    let body = BODY.replace("role: AI Tools Application Engineer\nmin_score: 6", &many);
    assert!(reject(&body).contains("more than"));

    let long = format!("role: {}", "word ".repeat(MAX_ARGUMENT_VALUE_BYTES));
    let body = BODY.replace("role: AI Tools Application Engineer\nmin_score: 6", &long);
    let message = reject(&body);
    assert!(message.contains("limit is"), "{message}");
    assert!(
        !message.contains("word word"),
        "the value must never be echoed back: {message}"
    );
}

#[test]
fn credential_shaped_values_are_rejected_without_echoing_them() {
    let secrets = [
        "ghp_0123456789abcdefghijklmnopqrstuvwxyz",
        "github_pat_11ABCDEFG0abcdefghijkl",
        "nyx_ag_livekey",
        "sk-proj-abcdef0123456789",
        "xoxb-1234-5678-abcdefghijkl",
        "AKIAIOSFODNN7EXAMPLE",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        // No known prefix, but a long unbroken token-alphabet run mixing letters
        // and digits — the shape of a base64/hex secret someone pasted.
        "aGVsbG8xMjM0NTY3ODkwYWJjZGVmZ2hpamts",
    ];
    for secret in secrets {
        let body = BODY.replace("min_score: 6", &format!("token: {secret}"));
        let message = reject(&body);
        assert!(
            message.contains("credential"),
            "{secret} must be refused: {message}"
        );
        assert!(
            message.contains("environment profile"),
            "the message must point at the supported alternative: {message}"
        );
        assert!(
            !message.contains(secret),
            "the offending value must never appear in the error: {message}"
        );
    }
}

#[test]
fn ordinary_prose_arguments_are_not_mistaken_for_credentials() {
    // Long, but broken by spaces — the shape of a real search parameter, which is
    // exactly the kind of argument this workload is built around.
    let prose = "Senior AI Tools Application Engineer, remote, Europe or North America";
    let body = BODY.replace("min_score: 6", &format!("role_query: {prose}"));
    let spec = parse_scheduled_workflow(&body).expect("prose is not a credential");
    assert_eq!(
        spec.arguments.get("role_query").map(String::as_str),
        Some(prose)
    );
}

#[test]
fn template_guidance_comments_do_not_count_as_content() {
    // The shipped issue template carries its explanation as HTML comments; a
    // pristine filled-in template must parse.
    let body = "### Workflow\n<!-- the id under .fkst/workflows/ -->\nsourcing\n\n### Run Mode\n<!-- `once` or `cron: <expr>` -->\nonce\n\n### Arguments\n<!-- optional key: value lines -->\n";
    let spec = parse_scheduled_workflow(body).expect("commented template parses");
    assert_eq!(spec.workflow_id, "sourcing");
    assert_eq!(spec.run_mode, RunMode::Once);
    assert!(spec.arguments.is_empty());
}
