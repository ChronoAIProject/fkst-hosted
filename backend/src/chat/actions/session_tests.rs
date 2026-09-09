//! Tests for action-proposal validation and rendering (sibling `#[path]` module).
//!
//! The parity test is the important one: the previewed body must be byte-for-byte what
//! the real create-session path would file. A preview that merely resembles the outcome
//! would make the confirm gate meaningless.

use super::*;

fn draft() -> DraftSessionRequest {
    DraftSessionRequest {
        name: "sitebuilder".to_string(),
        packages: vec!["acme/pkgs@main:packages/site".to_string()],
        manifests: Vec::new(),
        work_label: Some("site-build".to_string()),
        environment: None,
        source_branch: None,
        target_branch: None,
        auto_merge: None,
        log_access: Vec::new(),
        collaborators: Vec::new(),
        output_lang: None,
    }
}

/// A draft exercising every optional section.
fn full_draft() -> DraftSessionRequest {
    DraftSessionRequest {
        name: "docs-refresh".to_string(),
        packages: vec!["acme/pkgs@main:packages/writer".to_string()],
        manifests: vec!["acme/pkgs@main:manifests/default.json".to_string()],
        work_label: Some("docs-work".to_string()),
        environment: Some("my-node-env".to_string()),
        source_branch: Some("main".to_string()),
        target_branch: Some("docs-integration".to_string()),
        auto_merge: Some(true),
        log_access: vec!["alice".to_string()],
        collaborators: vec!["carol".to_string()],
        output_lang: Some("en".to_string()),
    }
}

fn expect_error(result: Result<ActionProposal, ProposalError>, needle: &str) {
    match result {
        Ok(_) => panic!("expected a rejection mentioning {needle:?}"),
        Err(error) => assert!(
            error.to_string().contains(needle),
            "error must mention {needle:?}: {error}"
        ),
    }
}

// ---- create-session: rendering + parity -----------------------------------

#[test]
fn a_full_draft_renders_every_provided_section() {
    let validated =
        validate_create_session("acme", "site", full_draft()).expect("the draft is valid");
    let body = &validated.rendered_issue_body;
    for section in [
        "### Session Name",
        "### Packages",
        "### Manifest",
        "### Work Label",
        "### Environment",
        "### Source Branch",
        "### Target Branch",
        "### Auto-merge",
        "### Session Collaborators",
        "### Output Language",
    ] {
        assert!(body.contains(section), "{section} missing from:\n{body}");
    }
    assert!(body.contains("docs-refresh"));
    assert!(body.contains("acme/pkgs@main:packages/writer"));
}

#[test]
fn the_preview_equals_what_the_real_endpoint_would_file() {
    // The parity guarantee: the same renderer, on the same mapped request, must produce
    // the identical body. If these ever diverge the confirm gate is showing fiction.
    let draft = full_draft();
    let validated =
        validate_create_session("acme", "site", draft.clone()).expect("the draft is valid");
    let direct = crate::routes::canvas::trigger_body::validated_trigger_body(
        &draft_to_request_for_test(&draft),
    )
    .expect("the real renderer accepts the mapped request");
    assert_eq!(validated.rendered_issue_body, direct);
}

/// Map a draft the way the module does, for the parity comparison above.
fn draft_to_request_for_test(
    draft: &DraftSessionRequest,
) -> crate::routes::canvas::trigger_body::CreateSessionRequest {
    draft.to_create_request()
}

#[test]
fn a_minimal_draft_omits_the_optional_sections() {
    let validated = validate_create_session("acme", "site", draft()).expect("valid");
    let body = &validated.rendered_issue_body;
    assert!(!body.contains("### Environment"));
    assert!(!body.contains("### Auto-merge"));
    assert!(!body.contains("### Output Language"));
}

#[test]
fn owner_and_repo_are_trimmed_and_carried_through() {
    let validated = validate_create_session("  acme  ", " site ", draft()).expect("valid");
    assert_eq!(validated.owner, "acme");
    assert_eq!(validated.repo, "site");
}

// ---- create-session: rejections ------------------------------------------

#[test]
fn a_draft_with_no_package_source_is_rejected() {
    let mut draft = draft();
    draft.packages.clear();
    draft.manifests.clear();
    expect_error(
        propose_create_session("acme", "site", draft),
        "at least one package source",
    );
}

#[test]
fn a_manifest_alone_is_a_valid_package_source() {
    let mut draft = draft();
    draft.packages.clear();
    draft.manifests = vec!["acme/pkgs@main:manifests/default.json".to_string()];
    propose_create_session("acme", "site", draft).expect("a manifest is sufficient");
}

#[test]
fn a_malformed_package_reference_is_rejected_by_the_real_grammar() {
    // Reusing the trigger parser means a draft can never be accepted here and rejected
    // by the endpoint.
    for bad in [
        "no-at-sign",
        "acme/pkgs@main",
        "acme@main:path",
        "acme/pkgs@main:../escape",
    ] {
        let mut draft = draft();
        draft.packages = vec![bad.to_string()];
        expect_error(propose_create_session("acme", "site", draft), "packages");
    }
}

#[test]
fn a_malformed_manifest_reference_is_rejected_and_named_as_a_manifest() {
    let mut draft = draft();
    draft.manifests = vec!["not-a-ref".to_string()];
    expect_error(propose_create_session("acme", "site", draft), "manifests");
}

#[test]
fn an_empty_session_name_is_rejected() {
    let mut draft = draft();
    draft.name = "   ".to_string();
    expect_error(
        propose_create_session("acme", "site", draft),
        "session name",
    );
}

#[test]
fn a_blank_owner_or_repo_is_rejected() {
    expect_error(propose_create_session("", "site", draft()), "owner");
    expect_error(propose_create_session("acme", "  ", draft()), "name");
}

#[test]
fn an_over_long_work_label_is_rejected() {
    let mut draft = draft();
    draft.work_label = Some("x".repeat(MAX_WORK_LABEL_CHARS + 1));
    expect_error(propose_create_session("acme", "site", draft), "50");
}

#[test]
fn a_comma_separated_work_label_is_rejected() {
    // The trigger takes exactly one label; a comma means the model tried to pass two.
    let mut draft = draft();
    draft.work_label = Some("a,b".to_string());
    expect_error(
        propose_create_session("acme", "site", draft),
        "single label",
    );
}

#[test]
fn blank_list_entries_are_dropped_rather_than_rendered() {
    let mut draft = draft();
    draft.packages = vec![
        "  ".to_string(),
        "acme/pkgs@main:packages/site".to_string(),
        String::new(),
    ];
    let validated = validate_create_session("acme", "site", draft).expect("valid");
    assert_eq!(validated.request.packages.len(), 1);
}

// ---- the proposal payload ------------------------------------------------

#[test]
fn a_session_proposal_carries_its_target_and_summary() {
    let proposal = propose_create_session("acme", "site", draft()).expect("valid");
    let ActionProposal::CreateSession {
        owner,
        name,
        target,
        summary,
        request,
        ..
    } = &proposal
    else {
        panic!("expected a create-session proposal");
    };
    assert_eq!(owner, "acme");
    assert_eq!(name, "site");
    assert_eq!(target.method, "POST");
    assert_eq!(target.path, "/api/v1/repos/acme/site/sessions");
    assert!(summary.contains("sitebuilder") && summary.contains("site-build"));
    assert_eq!(request.name, "sitebuilder");
}

#[test]
fn a_label_less_session_summary_says_labels_are_discovered() {
    let mut draft = draft();
    draft.work_label = None;
    let proposal = propose_create_session("acme", "site", draft).expect("valid");
    let ActionProposal::CreateSession { summary, .. } = &proposal else {
        panic!("expected a create-session proposal");
    };
    assert!(summary.contains("auto-discovered"), "got {summary}");
}

#[test]
fn a_session_proposal_serializes_with_its_kind_discriminant() {
    // The whole reason a dedicated draft DTO exists: this must be `Serialize`, which
    // `CreateSessionRequest` deliberately is not.
    let proposal = propose_create_session("acme", "site", draft()).expect("valid");
    let json = serde_json::to_value(&proposal).expect("proposals must serialize");
    assert_eq!(json["kind"], "create_session");
    assert_eq!(json["owner"], "acme");
    assert!(json["rendered_issue_body"].is_string());
    assert!(json["request"]["packages"].is_array());
    assert!(
        json["request"].get("disposable_environment").is_none(),
        "a draft has no field for secrets, so none can appear on the wire"
    );
}

// ---- work items ----------------------------------------------------------

#[test]
fn a_work_item_proposal_validates_and_targets_the_work_items_endpoint() {
    let proposal = propose_work_item(
        "acme",
        "site",
        7,
        "Add the footer",
        Some("site-build".to_string()),
        Some("Edit src/footer.tsx".to_string()),
    )
    .expect("valid");
    let ActionProposal::CreateWorkItem {
        trigger_issue_number,
        title,
        label,
        body,
        target,
        summary,
        ..
    } = &proposal
    else {
        panic!("expected a work-item proposal");
    };
    assert_eq!(*trigger_issue_number, 7);
    assert_eq!(title, "Add the footer");
    assert_eq!(label.as_deref(), Some("site-build"));
    assert_eq!(body, "Edit src/footer.tsx");
    assert_eq!(target.method, "POST");
    assert_eq!(target.path, "/api/v1/repos/acme/site/sessions/7/work-items");
    assert!(summary.contains("Add the footer"));
}

#[test]
fn a_work_item_label_is_optional_so_the_endpoint_default_applies() {
    let proposal =
        propose_work_item("acme", "site", 7, "Task", None, None).expect("valid without a label");
    let ActionProposal::CreateWorkItem { label, body, .. } = &proposal else {
        panic!("expected a work-item proposal");
    };
    assert_eq!(*label, None);
    assert_eq!(body, "");
}

#[test]
fn work_item_caps_are_enforced() {
    expect_error(
        propose_work_item("acme", "site", 7, &"x".repeat(201), None, None),
        "200",
    );
    expect_error(
        propose_work_item(
            "acme",
            "site",
            7,
            "Task",
            None,
            Some("x".repeat(20 * 1024 + 1)),
        ),
        "20480",
    );
    expect_error(
        propose_work_item("acme", "site", 7, "  ", None, None),
        "title",
    );
    expect_error(
        propose_work_item("acme", "site", 0, "Task", None, None),
        "positive",
    );
}

// ---- stop ---------------------------------------------------------------

#[test]
fn a_stop_proposal_requires_a_reason_and_targets_the_delete_endpoint() {
    let proposal = propose_stop_session("acme", "site", 7, "the work is finished").expect("valid");
    let ActionProposal::StopSession {
        trigger_issue_number,
        reason,
        target,
        ..
    } = &proposal
    else {
        panic!("expected a stop proposal");
    };
    assert_eq!(*trigger_issue_number, 7);
    assert_eq!(reason, "the work is finished");
    assert_eq!(target.method, "DELETE");
    assert_eq!(target.path, "/api/v1/repos/acme/site/sessions/7");
}

#[test]
fn a_reasonless_stop_is_rejected() {
    // Stopping is irreversible; the user must see WHY it is being suggested.
    expect_error(propose_stop_session("acme", "site", 7, "  "), "stop reason");
}

#[test]
fn an_over_long_stop_reason_is_rejected() {
    expect_error(
        propose_stop_session("acme", "site", 7, &"x".repeat(MAX_STOP_REASON_CHARS + 1)),
        "500",
    );
}

#[test]
fn every_proposal_kind_serializes_with_a_distinct_discriminant() {
    let kinds: Vec<String> = vec![
        propose_create_session("acme", "site", draft()).expect("valid"),
        propose_work_item("acme", "site", 7, "Task", None, None).expect("valid"),
        propose_stop_session("acme", "site", 7, "done").expect("valid"),
    ]
    .into_iter()
    .map(|proposal| {
        serde_json::to_value(&proposal).expect("serializes")["kind"]
            .as_str()
            .expect("kind is a string")
            .to_string()
    })
    .collect();
    assert_eq!(
        kinds,
        vec!["create_session", "create_work_item", "stop_session"]
    );
}
