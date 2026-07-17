use super::*;
use crate::github_app::listing::IssueSummary;

fn meta(number: i64, state: &str, closed_at: Option<&str>) -> IssueWithMeta {
    IssueWithMeta {
        summary: IssueSummary {
            number,
            title: format!("issue-{number}"),
            body: "body".to_string(),
            labels: vec!["fkst-substrate-trigger".to_string(), "bug".to_string()],
            state: state.to_string(),
            assignees: Vec::new(),
            user_login: "author".to_string(),
            user_id: 9,
        },
        html_url: format!("https://github.com/acme/site/issues/{number}"),
        created_at: "2026-07-01T00:00:00Z".to_string(),
        updated_at: "2026-07-02T00:00:00Z".to_string(),
        closed_at: closed_at.map(str::to_string),
    }
}

#[test]
fn issue_detail_projects_summary_and_meta() {
    let detail = IssueDetail::from(&meta(5, "closed", Some("2026-07-03T00:00:00Z")));
    assert_eq!(detail.number, 5);
    assert_eq!(detail.title, "issue-5");
    assert_eq!(detail.state, "closed");
    assert_eq!(detail.author, "author");
    assert_eq!(
        detail.labels,
        vec!["fkst-substrate-trigger".to_string(), "bug".to_string()]
    );
    assert_eq!(detail.html_url, "https://github.com/acme/site/issues/5");
    assert_eq!(detail.created_at, "2026-07-01T00:00:00Z");
    assert_eq!(detail.updated_at, "2026-07-02T00:00:00Z");
    assert_eq!(detail.closed_at.as_deref(), Some("2026-07-03T00:00:00Z"));
}

#[test]
fn issue_detail_keeps_open_issue_closed_at_null_in_json() {
    let detail = IssueDetail::from(&meta(6, "open", None));
    let json = serde_json::to_value(&detail).expect("serialize");
    assert_eq!(json["state"], "open");
    assert!(json["closed_at"].is_null(), "open issue renders closed_at null");
}
