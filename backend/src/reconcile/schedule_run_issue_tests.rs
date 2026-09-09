use k8s_openapi::chrono::TimeZone;

use super::*;

fn slot() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 5, 1, 0, 0)
        .single()
        .expect("valid timestamp")
}

fn request() -> RunIssueRequest {
    RunIssueRequest {
        schedule_issue: 123,
        workflow_id: "github-candidate-sourcing".to_string(),
        slot: slot(),
        arguments: BTreeMap::from([
            (
                "role".to_string(),
                "AI Tools Application Engineer".to_string(),
            ),
            ("min_score".to_string(), "6".to_string()),
        ]),
        work_label: "fkst-dev-chronoai-fkst".to_string(),
        creator_login: "alice".to_string(),
        manual: false,
    }
}

#[test]
fn the_title_is_slot_stamped_so_two_runs_never_collide() {
    assert_eq!(
        request().title(),
        "[scheduled] github-candidate-sourcing — 2026-08-05T01:00:00Z"
    );
}

#[test]
fn the_body_carries_the_dispatch_marker_binding_the_run_to_its_slot() {
    let body = render_run_issue_body(&request());
    assert!(
        body.starts_with(
            "<!-- fkst-cron-dispatch:v1 schedule=\"123\" \
             workflow=\"github-candidate-sourcing\" slot=\"2026-08-05T01:00:00Z\" \
             manual=\"false\" -->"
        ),
        "{body}"
    );
}

#[test]
fn arguments_render_as_quoted_toml_rather_than_shell_text() {
    let body = render_run_issue_body(&request());
    assert!(body.contains("```toml\n"), "{body}");
    assert!(
        body.contains("min_score = \"6\"\n"),
        "even a numeric-looking value stays a string: {body}"
    );
    assert!(
        body.contains("role = \"AI Tools Application Engineer\"\n"),
        "{body}"
    );
}

#[test]
fn a_hostile_argument_value_stays_data() {
    // The run issue is how a public issue body's arguments reach the pod. A value
    // that could terminate its own quoting would be a command-injection channel.
    let mut request = request();
    request.arguments = BTreeMap::from([(
        "role".to_string(),
        "\" ; rm -rf / ; echo \"\\pwned\nsecond line\ttabbed".to_string(),
    )]);
    let body = render_run_issue_body(&request);
    let line = body
        .lines()
        .find(|line| line.starts_with("role = "))
        .expect("the argument renders on one line");
    assert_eq!(
        line, "role = \"\\\" ; rm -rf / ; echo \\\"\\\\pwned\\nsecond line\\ttabbed\"",
        "quotes, backslashes and newlines are escaped, so the value cannot escape \
         its own string"
    );
    // The TOML parser is the contract, so prove a real one reads the value back.
    let parsed: toml::Value = toml::from_str(line).expect("valid TOML");
    assert_eq!(
        parsed["role"].as_str().expect("a string"),
        request.arguments["role"]
    );
}

#[test]
fn an_argument_less_run_says_so_rather_than_emitting_an_empty_block() {
    let mut request = request();
    request.arguments.clear();
    let body = render_run_issue_body(&request);
    assert!(body.contains("_None._"), "{body}");
    assert!(!body.contains("```toml"), "{body}");
}

#[test]
fn the_body_points_the_reader_back_at_the_definition() {
    // A run issue looks editable and is not: the schedule lives on #123.
    let body = render_run_issue_body(&request());
    assert!(body.contains("scheduled workflow #123"), "{body}");
    assert!(body.contains("edit #123 instead"), "{body}");
}

#[test]
fn a_manual_run_is_visibly_manual_in_both_the_marker_and_the_prose() {
    let manual = RunIssueRequest {
        manual: true,
        ..request()
    };
    let body = render_run_issue_body(&manual);
    assert!(body.contains("manual=\"true\""), "{body}");
    assert!(body.contains("started manually"), "{body}");
}
