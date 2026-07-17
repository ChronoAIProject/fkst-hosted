//! Render → parse round-trip tests for the create-session trigger body: the
//! rendered text must mean, to the reconciler's parser, exactly what was
//! requested — and anything that could mean something else must 400.

use super::*;

fn full_request() -> CreateSessionRequest {
    CreateSessionRequest {
        name: "site".to_string(),
        packages: vec![
            "acme/pkgs@main:packages/devloop".to_string(),
            "acme/pkgs@main:packages/triage".to_string(),
        ],
        work_label: Some("site-build".to_string()),
        environment: Some("prod-env".to_string()),
        auto_merge: Some(true),
        log_access: vec!["reviewer".to_string(), "12345".to_string()],
        output_lang: Some("zh-CN".to_string()),
    }
}

#[test]
fn full_request_round_trips_through_the_trigger_parser() {
    let body = validated_trigger_body(&full_request()).expect("valid");
    let spec = parse_trigger_issue_body(&body).expect("parses");
    assert_eq!(spec.name, "site");
    assert_eq!(spec.packages.len(), 2);
    assert_eq!(spec.packages[0].owner, "acme");
    assert_eq!(spec.packages[0].repo, "pkgs");
    assert_eq!(spec.packages[0].git_ref, "main");
    assert_eq!(spec.packages[0].path, "packages/devloop");
    assert_eq!(spec.work_label.as_deref(), Some("site-build"));
    assert_eq!(spec.environment.as_deref(), Some("prod-env"));
    assert!(spec.auto_merge);
    assert_eq!(
        spec.log_access,
        vec!["reviewer".to_string(), "12345".to_string()]
    );
    assert_eq!(spec.output_lang.as_deref(), Some("zh-CN"));
    assert!(spec.engine_config.is_empty());
}

#[test]
fn minimal_request_omits_every_optional_section() {
    let req = CreateSessionRequest {
        name: "site".to_string(),
        packages: vec!["acme/pkgs@main:packages/devloop".to_string()],
        work_label: None,
        environment: None,
        auto_merge: None,
        log_access: Vec::new(),
        output_lang: None,
    };
    let body = validated_trigger_body(&req).expect("valid");
    assert!(!body.contains("### Work Label"));
    assert!(!body.contains("### Environment"));
    assert!(!body.contains("### Auto-merge"));
    assert!(!body.contains("### Log Access Allowlist"));
    assert!(!body.contains("### Output Language"));
    let spec = parse_trigger_issue_body(&body).expect("parses");
    assert_eq!(spec.work_label, None);
    assert!(!spec.auto_merge);
    assert!(spec.log_access.is_empty());
}

#[test]
fn auto_merge_false_and_blank_optionals_render_like_absent() {
    let req = CreateSessionRequest {
        name: "site".to_string(),
        packages: vec!["acme/pkgs@main:packages/devloop".to_string()],
        work_label: Some("   ".to_string()),
        environment: Some(String::new()),
        auto_merge: Some(false),
        log_access: vec!["  ".to_string()],
        output_lang: None,
    };
    let body = validated_trigger_body(&req).expect("valid");
    assert!(!body.contains("### Auto-merge"), "false renders no section");
    assert!(
        !body.contains("### Work Label"),
        "blank collapses to absent"
    );
    let spec = parse_trigger_issue_body(&body).expect("parses");
    assert!(!spec.auto_merge);
    assert_eq!(spec.work_label, None);
}

#[test]
fn an_invalid_package_ref_is_a_400_carrying_the_parsers_message() {
    let req = CreateSessionRequest {
        packages: vec!["not-a-package-ref".to_string()],
        ..full_request()
    };
    let err = validated_trigger_body(&req).expect_err("must reject");
    match err {
        AppError::Validation(message) => {
            assert!(
                message.contains("### Packages"),
                "names the section: {message}"
            );
            assert!(
                message.contains("not-a-package-ref"),
                "echoes the value: {message}"
            );
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[test]
fn empty_packages_are_rejected_before_rendering() {
    let req = CreateSessionRequest {
        packages: Vec::new(),
        ..full_request()
    };
    let err = validated_trigger_body(&req).expect_err("must reject");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

#[test]
fn multi_line_and_heading_shaped_values_are_rejected() {
    // A newline could smuggle an extra heading into the body; a leading '#'
    // could BE a heading. Both must 400 for every field.
    let injections = [
        (
            "name",
            CreateSessionRequest {
                name: "site\n\n### Engine Config\nFKST_X=1".to_string(),
                ..full_request()
            },
        ),
        (
            "work_label",
            CreateSessionRequest {
                work_label: Some("### Engine Config".to_string()),
                ..full_request()
            },
        ),
        (
            "environment",
            CreateSessionRequest {
                environment: Some("env\r\nFKST_X=1".to_string()),
                ..full_request()
            },
        ),
        (
            "log_access",
            CreateSessionRequest {
                log_access: vec!["### Auto-merge".to_string()],
                ..full_request()
            },
        ),
        (
            "packages",
            CreateSessionRequest {
                packages: vec!["acme/pkgs@main:p\n### Output Language\nzh".to_string()],
                ..full_request()
            },
        ),
    ];
    for (field, req) in injections {
        let err = validated_trigger_body(&req).expect_err(field);
        assert!(
            matches!(err, AppError::Validation(_)),
            "{field}: expected Validation, got {err:?}"
        );
    }
}

#[test]
fn an_invalid_session_name_is_a_400_naming_the_section() {
    let req = CreateSessionRequest {
        name: "Not A Valid Name".to_string(),
        ..full_request()
    };
    let err = validated_trigger_body(&req).expect_err("must reject");
    match err {
        AppError::Validation(message) => {
            assert!(
                message.contains("### Session Name"),
                "names the section: {message}"
            );
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}
