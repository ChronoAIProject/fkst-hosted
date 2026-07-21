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
        manifests: vec!["acme/manifests@main:bundles/site".to_string()],
        work_label: Some("site-build".to_string()),
        environment: Some("prod-env".to_string()),
        auto_merge: Some(true),
        log_access: vec!["reviewer".to_string(), "12345".to_string()],
        collaborators: vec!["worker".to_string(), "helper".to_string()],
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
    assert_eq!(
        spec.manifest_refs.len(),
        1,
        "the `### Manifest` reference round-trips through the parser"
    );
    assert_eq!(spec.manifest_refs[0].owner, "acme");
    assert_eq!(spec.manifest_refs[0].repo, "manifests");
    assert_eq!(spec.manifest_refs[0].git_ref, "main");
    assert_eq!(spec.manifest_refs[0].path, "bundles/site");
    // The `### Manifest` section renders right after `### Packages` (template order).
    let packages_at = body.find("### Packages").expect("packages section");
    let manifest_at = body.find("### Manifest").expect("manifest section");
    assert!(
        packages_at < manifest_at,
        "### Manifest must follow ### Packages"
    );
    assert_eq!(spec.work_label.as_deref(), Some("site-build"));
    assert_eq!(spec.environment.as_deref(), Some("prod-env"));
    assert!(spec.auto_merge);
    assert_eq!(
        spec.log_access,
        vec!["reviewer".to_string(), "12345".to_string()]
    );
    assert_eq!(
        spec.collaborators,
        vec!["worker".to_string(), "helper".to_string()],
        "the `### Session Collaborators` grantees round-trip through the parser"
    );
    assert_eq!(spec.output_lang.as_deref(), Some("zh-CN"));
    assert!(spec.engine_config.is_empty());
}

#[test]
fn minimal_request_omits_every_optional_section() {
    let req = CreateSessionRequest {
        name: "site".to_string(),
        packages: vec!["acme/pkgs@main:packages/devloop".to_string()],
        manifests: Vec::new(),
        work_label: None,
        environment: None,
        auto_merge: None,
        log_access: Vec::new(),
        collaborators: Vec::new(),
        output_lang: None,
    };
    let body = validated_trigger_body(&req).expect("valid");
    assert!(!body.contains("### Manifest"));
    assert!(!body.contains("### Work Label"));
    assert!(!body.contains("### Environment"));
    assert!(!body.contains("### Auto-merge"));
    assert!(!body.contains("### Log Access Allowlist"));
    assert!(!body.contains("### Session Collaborators"));
    assert!(!body.contains("### Output Language"));
    let spec = parse_trigger_issue_body(&body).expect("parses");
    assert_eq!(spec.work_label, None);
    assert!(!spec.auto_merge);
    assert!(spec.log_access.is_empty());
    assert!(spec.collaborators.is_empty());
}

#[test]
fn auto_merge_false_and_blank_optionals_render_like_absent() {
    let req = CreateSessionRequest {
        name: "site".to_string(),
        packages: vec!["acme/pkgs@main:packages/devloop".to_string()],
        manifests: vec!["  ".to_string()],
        work_label: Some("   ".to_string()),
        environment: Some(String::new()),
        auto_merge: Some(false),
        log_access: vec!["  ".to_string()],
        collaborators: vec!["  ".to_string()],
        output_lang: None,
    };
    let body = validated_trigger_body(&req).expect("valid");
    assert!(
        !body.contains("### Manifest"),
        "a blank-only manifest list renders no section"
    );
    assert!(!body.contains("### Auto-merge"), "false renders no section");
    assert!(
        !body.contains("### Work Label"),
        "blank collapses to absent"
    );
    assert!(
        !body.contains("### Session Collaborators"),
        "a blank-only collaborators list renders no section"
    );
    let spec = parse_trigger_issue_body(&body).expect("parses");
    assert!(!spec.auto_merge);
    assert_eq!(spec.work_label, None);
    assert!(spec.collaborators.is_empty());
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
fn no_package_source_at_all_is_rejected_before_rendering() {
    // Neither `### Packages` nor `### Manifest` names a source — a 400 before any
    // rendering (mirrors the trigger parser's "≥1 package source" rule).
    let req = CreateSessionRequest {
        packages: Vec::new(),
        manifests: Vec::new(),
        ..full_request()
    };
    let err = validated_trigger_body(&req).expect_err("must reject");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

#[test]
fn a_manifest_only_request_renders_no_packages_section_and_round_trips() {
    // Since I7 a `### Manifest` reference can supply the packages, so a request
    // with no explicit packages is valid — it renders `### Manifest` but no
    // `### Packages` section, and round-trips through the parser.
    let req = CreateSessionRequest {
        packages: Vec::new(),
        manifests: vec!["acme/manifests@main:bundles/site".to_string()],
        ..full_request()
    };
    let body = validated_trigger_body(&req).expect("valid");
    assert!(
        !body.contains("### Packages"),
        "no explicit packages renders no `### Packages` section"
    );
    assert!(body.contains("### Manifest"));
    let spec = parse_trigger_issue_body(&body).expect("parses");
    assert!(spec.packages.is_empty());
    assert_eq!(spec.manifest_refs.len(), 1);
    assert_eq!(spec.manifest_refs[0].repo, "manifests");
}

#[test]
fn an_invalid_manifest_ref_is_a_400() {
    let req = CreateSessionRequest {
        manifests: vec!["not-a-manifest-ref".to_string()],
        ..full_request()
    };
    let err = validated_trigger_body(&req).expect_err("must reject");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

#[test]
fn a_manifest_entry_hiding_multiple_refs_is_a_400() {
    // Two refs on one rendered `### Manifest` line: the parser reads the whole
    // line as ONE reference (a space is not in the grammar), so it fails to
    // parse rather than silently fanning out — a fail-closed 400 either way.
    let req = CreateSessionRequest {
        manifests: vec!["acme/manifests@main:a acme/manifests@main:b".to_string()],
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
            "collaborators",
            CreateSessionRequest {
                collaborators: vec!["### Auto-merge".to_string()],
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
        (
            "manifests",
            CreateSessionRequest {
                manifests: vec!["acme/m@main:p\n### Engine Config\nFKST_X=1".to_string()],
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
fn a_log_access_entry_hiding_multiple_grantees_is_a_400() {
    // "alice bob" renders as ONE allowlist line but the parser splits on any
    // whitespace/comma, so the created trigger would grant log download (and
    // FKST_GITHUB_AUTHORIZED_LOGINS trust) to grantees the request never
    // listed as separate entries. The round-trip check must fail closed.
    for entry in ["alice bob", "alice,bob"] {
        let req = CreateSessionRequest {
            log_access: vec![entry.to_string()],
            ..full_request()
        };
        let err = validated_trigger_body(&req).expect_err(entry);
        assert!(
            matches!(err, AppError::Validation(_)),
            "{entry:?}: expected Validation, got {err:?}"
        );
    }
}

#[test]
fn a_collaborators_entry_hiding_multiple_grantees_is_a_400() {
    // "worker helper" renders as ONE collaborators line but the parser splits on
    // any whitespace/comma, so the created trigger would grant work-item
    // authority to grantees the request never listed as separate entries. Like
    // log_access, the round-trip check must fail closed.
    for entry in ["worker helper", "worker,helper"] {
        let req = CreateSessionRequest {
            collaborators: vec![entry.to_string()],
            ..full_request()
        };
        let err = validated_trigger_body(&req).expect_err(entry);
        assert!(
            matches!(err, AppError::Validation(_)),
            "{entry:?}: expected Validation, got {err:?}"
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
