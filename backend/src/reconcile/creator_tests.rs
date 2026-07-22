use super::*;

fn metadata(author: &str, assignees: &[&str]) -> IssueMetadata {
    IssueMetadata {
        number: 7,
        labels: vec!["fkst-substrate-trigger".to_string()],
        state: "open".to_string(),
        assignees: assignees.iter().map(|value| value.to_string()).collect(),
        user_login: author.to_string(),
        user_id: 4242,
    }
}

#[test]
fn human_author_wins_over_assignees() {
    assert_eq!(
        effective_creator(&metadata("alice", &["bob"]), Some("fkst-app")),
        CreatorResolution::Resolved(SessionCreator {
            login: "alice".to_string(),
            id: Some(4242),
        })
    );
}

#[test]
fn bot_with_one_assignee_resolves_to_assignee_login() {
    assert_eq!(
        effective_creator(&metadata("fkst-app[bot]", &["alice"]), Some("fkst-app")),
        CreatorResolution::Resolved(SessionCreator {
            login: "alice".to_string(),
            id: None,
        })
    );
}

#[test]
fn bot_with_zero_or_multiple_assignees_is_unattributable() {
    for assignees in [&[][..], &["alice", "bob"][..]] {
        assert_eq!(
            effective_creator(&metadata("fkst-app[bot]", assignees), Some("fkst-app")),
            CreatorResolution::Unattributable {
                author_login: "fkst-app[bot]".to_string(),
                assignee_count: assignees.len(),
            }
        );
    }
}

#[test]
fn bot_login_normalization_matches_all_github_forms_case_insensitively() {
    for (author, configured) in [
        ("slug", "slug"),
        ("slug[bot]", "slug"),
        ("app/slug", "slug[bot]"),
        ("APP/SLUG[BOT]", "SlUg"),
    ] {
        assert_eq!(
            effective_creator(&metadata(author, &["creator"]), Some(configured)),
            CreatorResolution::Resolved(SessionCreator {
                login: "creator".to_string(),
                id: None,
            }),
            "author={author}, configured={configured}"
        );
    }
}

#[test]
fn absent_bot_login_treats_every_issue_as_human_authored() {
    assert_eq!(
        effective_creator(&metadata("fkst-app[bot]", &["alice"]), None),
        CreatorResolution::Resolved(SessionCreator {
            login: "fkst-app[bot]".to_string(),
            id: Some(4242),
        })
    );
}
