//! Tests for card projection (sibling `#[path]` module).
//!
//! The two invariants worth pinning: a card is built from the RESULT (never from
//! anything the model authored), and only a 200 projects — an error body must not
//! produce a card that implies the lookup worked.

use super::*;

fn ok(body: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "status": 200, "body": body })
}

// ---- the shared guards ---------------------------------------------------

#[test]
fn a_non_200_never_projects_a_card() {
    for status in [403, 404, 409, 500, 504] {
        let result = serde_json::json!({
            "status": status,
            "body": { "environment_profiles": [] },
        });
        assert_eq!(
            project("list_environment_profiles", &serde_json::json!({}), &result),
            None,
            "status {status} must not render a card"
        );
    }
}

#[test]
fn a_tool_with_no_card_shape_projects_nothing() {
    // These answer in prose, or already contribute a SessionRef.
    for tool in [
        "search_manual",
        "get_overview",
        "list_repo_sessions",
        "tail_log_file",
        "draft_trigger_session",
    ] {
        assert_eq!(
            project(tool, &serde_json::json!({}), &ok(serde_json::json!({}))),
            None,
            "{tool} must not project a card"
        );
    }
}

#[test]
fn an_unrecognized_payload_yields_no_card_rather_than_a_blank_one() {
    // A card with empty fields reads as "there is nothing there"; None is honest.
    let result = ok(serde_json::json!({ "unexpected": true }));
    assert_eq!(
        project("list_environment_profiles", &serde_json::json!({}), &result),
        None
    );
    assert_eq!(
        project("get_session_outcomes", &serde_json::json!({}), &result),
        None
    );
    assert_eq!(project("list_log_runs", &serde_json::json!({}), &result), None);
}

// ---- environments --------------------------------------------------------

#[test]
fn the_environment_list_projects_every_summary_field() {
    let result = ok(serde_json::json!({
        "environment_profiles": [{
            "name": "video-studio",
            "status": "ready",
            "validated_at": "2026-07-18T21:57:56Z",
            "install_command_count": 2,
            "variable_count": 2,
            "secret_count": 1,
        }],
    }));
    let Some(DataCard::Environments { profiles, omitted }) =
        project("list_environment_profiles", &serde_json::json!({}), &result)
    else {
        panic!("expected an Environments card");
    };
    assert_eq!(omitted, 0);
    assert_eq!(
        profiles,
        vec![EnvironmentSummaryCard {
            name: "video-studio".to_string(),
            status: "ready".to_string(),
            validated_at: "2026-07-18T21:57:56Z".to_string(),
            install_command_count: 2,
            variable_count: 2,
            secret_count: 1,
        }]
    );
}

#[test]
fn an_empty_environment_list_still_renders() {
    // "You have none yet" is a real answer, and the card states it better than prose.
    let result = ok(serde_json::json!({ "environment_profiles": [] }));
    let card = project("list_environment_profiles", &serde_json::json!({}), &result);
    assert!(matches!(
        card,
        Some(DataCard::Environments { ref profiles, omitted: 0 }) if profiles.is_empty()
    ));
}

#[test]
fn a_long_list_is_bounded_and_says_how_many_it_dropped() {
    let rows: Vec<_> = (0..MAX_ROWS + 5)
        .map(|i| serde_json::json!({ "name": format!("env-{i}"), "status": "ready" }))
        .collect();
    let result = ok(serde_json::json!({ "environment_profiles": rows }));
    let Some(DataCard::Environments { profiles, omitted }) =
        project("list_environment_profiles", &serde_json::json!({}), &result)
    else {
        panic!("expected an Environments card");
    };
    assert_eq!(profiles.len(), MAX_ROWS);
    // Silently truncating would read as "that is all of them".
    assert_eq!(omitted, 5);
}

#[test]
fn the_environment_detail_keeps_secret_names_and_never_invents_values() {
    let result = ok(serde_json::json!({
        "name": "video-studio",
        "status": "ready",
        "validated_at": "2026-07-18T21:57:56Z",
        "install": ["apt-get install -y ffmpeg", "pip install yt-dlp"],
        "variables": { "FFMPEG_PRESET": "veryfast" },
        "secret_keys": ["YT_API_KEY"],
    }));
    let Some(DataCard::EnvironmentDetail {
        name,
        install,
        variables,
        secret_keys,
        ..
    }) = project("get_environment_profile", &serde_json::json!({}), &result)
    else {
        panic!("expected an EnvironmentDetail card");
    };
    assert_eq!(name, "video-studio");
    assert_eq!(install.len(), 2);
    assert_eq!(
        variables,
        vec![CardVariable {
            key: "FFMPEG_PRESET".to_string(),
            value: "veryfast".to_string(),
        }]
    );
    // Names only — the endpoint never returns a secret VALUE, and the card has no field
    // that could hold one.
    assert_eq!(secret_keys, vec!["YT_API_KEY".to_string()]);
}

#[test]
fn an_environment_detail_without_optional_sections_still_projects() {
    let result = ok(serde_json::json!({ "name": "bare", "status": "ready" }));
    let Some(DataCard::EnvironmentDetail {
        install,
        variables,
        secret_keys,
        ..
    }) = project("get_environment_profile", &serde_json::json!({}), &result)
    else {
        panic!("expected an EnvironmentDetail card");
    };
    assert!(install.is_empty() && variables.is_empty() && secret_keys.is_empty());
}

// ---- outcomes ------------------------------------------------------------

#[test]
fn outcomes_project_pull_requests_and_count_the_merged_ones() {
    let result = ok(serde_json::json!({
        "owner": "acme",
        "name": "site",
        "trigger_issue": 12,
        "prs": [
            { "number": 20, "title": "Add the hero", "html_url": "https://x/20",
              "state": "closed", "merged": true, "work_issue": 15,
              "files": [{ "filename": "a" }, { "filename": "b" }] },
            { "number": 21, "title": "Add the footer", "html_url": "https://x/21",
              "state": "open", "merged": false, "work_issue": null, "files": [] },
        ],
    }));
    let Some(DataCard::Outcomes {
        owner,
        trigger_issue,
        pull_requests,
        merged,
        omitted,
        ..
    }) = project("get_session_outcomes", &serde_json::json!({}), &result)
    else {
        panic!("expected an Outcomes card");
    };
    assert_eq!(owner, "acme");
    assert_eq!(trigger_issue, 12);
    assert_eq!(merged, 1);
    assert_eq!(omitted, 0);
    assert_eq!(pull_requests[0].files_changed, 2);
    assert_eq!(pull_requests[0].work_issue, Some(15));
    assert_eq!(pull_requests[1].work_issue, None);
    assert!(!pull_requests[1].merged);
}

// ---- logs ----------------------------------------------------------------

#[test]
fn log_runs_take_the_session_id_from_the_arguments() {
    // The runs endpoint does not echo the session id, so the argument is the ONLY
    // source — every other field still comes from the response.
    let result = ok(serde_json::json!({
        "runs": [
            { "run_id": "20260718T2157Z-abc", "started_at": "2026-07-18T21:57:00Z",
              "ended_at": "2026-07-18T22:10:00Z" },
            { "run_id": "20260719T0900Z-def", "started_at": "2026-07-19T09:00:00Z" },
        ],
    }));
    let args = serde_json::json!({ "session_id": "sess-1" });
    let Some(DataCard::LogRuns {
        session_id, runs, ..
    }) = project("list_log_runs", &args, &result)
    else {
        panic!("expected a LogRuns card");
    };
    assert_eq!(session_id, "sess-1");
    assert_eq!(runs[0].ended_at.as_deref(), Some("2026-07-18T22:10:00Z"));
    // A live run has no end time, and the card must show that rather than a blank.
    assert_eq!(runs[1].ended_at, None);
}

#[test]
fn a_log_manifest_prefers_the_response_run_over_the_requested_one() {
    // "Omit `run` for the latest" means the response names the run that was actually
    // read; showing the request's (absent) value would leave the card unlabelled.
    let result = ok(serde_json::json!({
        "run": "20260719T0900Z-def",
        "files": [{ "path": "codex/run.log", "size_bytes": 4096 }],
    }));
    let args = serde_json::json!({ "session_id": "sess-1" });
    let Some(DataCard::LogManifest {
        session_id,
        run,
        files,
        ..
    }) = project("get_log_manifest", &args, &result)
    else {
        panic!("expected a LogManifest card");
    };
    assert_eq!(session_id, "sess-1");
    assert_eq!(run.as_deref(), Some("20260719T0900Z-def"));
    assert_eq!(files[0].size_bytes, 4096);
}

#[test]
fn a_log_manifest_falls_back_to_the_requested_run() {
    let result = ok(serde_json::json!({ "files": [] }));
    let args = serde_json::json!({ "session_id": "s", "run": "requested-run" });
    let Some(DataCard::LogManifest { run, .. }) = project("get_log_manifest", &args, &result)
    else {
        panic!("expected a LogManifest card");
    };
    assert_eq!(run.as_deref(), Some("requested-run"));
}
