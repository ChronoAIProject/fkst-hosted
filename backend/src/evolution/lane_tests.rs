//! Lane naming, markers, duplicate repair, and the auto-merge exclusion.

use super::*;

const INPUT: &str = "sha256:c0ceeffb6f8cb4b312745f79be2ce6a8616f7c09700237fb8a1d9c8b012a5fb8";

fn marker() -> SyncPrMarker {
    SyncPrMarker {
        issue: 417,
        input: INPUT.to_string(),
        source_head: "a".repeat(40),
        generator: "sha256:beef".to_string(),
        verification: "passed".to_string(),
    }
}

// ---- short hashes and names ------------------------------------------------

#[test]
fn short_hash_takes_sixteen_hex_characters() {
    assert_eq!(short_hash(INPUT).unwrap(), "c0ceeffb6f8cb4b3");
    assert_eq!(short_hash(INPUT).unwrap().len(), 16);
}

#[test]
fn short_hash_rejects_a_malformed_fingerprint() {
    assert!(
        short_hash("sha1:abcdef0123456789").is_none(),
        "wrong algorithm"
    );
    assert!(short_hash("c0ceeffb6f8cb4b3").is_none(), "missing prefix");
    assert!(short_hash("sha256:tooshort").is_none());
    assert!(short_hash("sha256:zzzzzzzzzzzzzzzz").is_none(), "not hex");
}

#[test]
fn the_sync_branch_is_keyed_on_the_input_fingerprint_alone() {
    // The property the lane lock depends on: two racing reconciliations at the
    // same input MUST derive the same ref name, or `POST /git/refs` never
    // collides and the lock silently does nothing.
    let a = sync_branch_name(INPUT).unwrap();
    let b = sync_branch_name(INPUT).unwrap();
    assert_eq!(a, b);
    assert_eq!(a, "fkst/evolution/c0ceeffb6f8cb4b3");
}

#[test]
fn a_different_input_yields_a_different_branch() {
    let other = format!("sha256:{}", "9".repeat(64));
    assert_ne!(sync_branch_name(INPUT), sync_branch_name(&other));
}

#[test]
fn the_release_tag_shares_the_input_short_hash() {
    assert_eq!(
        release_tag(INPUT).unwrap(),
        "fkst-evolution/c0ceeffb6f8cb4b3"
    );
}

#[test]
fn evolution_owned_tags_are_recognised_by_exact_prefix() {
    assert!(is_evolution_tag("fkst-evolution/c0ceeffb6f8cb4b3"));
    assert!(!is_evolution_tag("v1.2.0"));
    // A tag that merely mentions the name is not ours.
    assert!(!is_evolution_tag("v1-fkst-evolution/x"));
}

// ---- markers ---------------------------------------------------------------

#[test]
fn a_rendered_marker_round_trips() {
    let body = format!(
        "Some human text.\n\n{}",
        render_marker(PR_MARKER_TAG, &marker()).unwrap()
    );
    let parsed: SyncPrMarker = parse_marker(&body, PR_MARKER_TAG).expect("parsed");
    assert_eq!(parsed, marker());
}

#[test]
fn user_text_outside_the_marker_is_never_interpreted() {
    let body = format!(
        "{{\"issue\": 1, \"input\": \"sha256:forged\"}}\n{}",
        render_marker(PR_MARKER_TAG, &marker()).unwrap()
    );
    let parsed: SyncPrMarker = parse_marker(&body, PR_MARKER_TAG).expect("parsed");
    assert_eq!(
        parsed.issue, 417,
        "the marker, not the surrounding text, is state"
    );
}

#[test]
fn an_absent_or_malformed_marker_yields_none() {
    assert!(parse_marker::<SyncPrMarker>("no marker here", PR_MARKER_TAG).is_none());
    assert!(parse_marker::<SyncPrMarker>(
        &format!("<!-- {PR_MARKER_TAG}\nnot json\n-->"),
        PR_MARKER_TAG
    )
    .is_none());
    // Opened but never closed: must not scan to the end of the body.
    assert!(parse_marker::<SyncPrMarker>(
        &format!("<!-- {PR_MARKER_TAG}\n{{\"issue\":1}}"),
        PR_MARKER_TAG
    )
    .is_none());
}

#[test]
fn a_marker_of_another_tag_is_not_matched() {
    let body = render_marker(SYNC_MARKER_TAG, &marker()).unwrap();
    assert!(parse_marker::<SyncPrMarker>(&body, PR_MARKER_TAG).is_none());
    assert!(parse_marker::<SyncPrMarker>(&body, SYNC_MARKER_TAG).is_some());
    // The preview tag is a third, distinct namespace.
    assert!(parse_marker::<SyncPrMarker>(&body, PREVIEW_MARKER_TAG).is_none());
}

#[test]
fn an_oversized_marker_is_refused_on_both_sides() {
    let huge = SyncPrMarker {
        generator: "x".repeat(8192),
        ..marker()
    };
    assert!(
        render_marker(PR_MARKER_TAG, &huge).is_err(),
        "must not emit"
    );

    let body = format!(
        "<!-- {PR_MARKER_TAG}\n{}\n-->",
        serde_json::to_string(&huge).unwrap()
    );
    assert!(
        parse_marker::<SyncPrMarker>(&body, PR_MARKER_TAG).is_none(),
        "must not parse"
    );
}

// ---- auto-merge exclusion --------------------------------------------------

#[test]
fn a_sync_pull_request_is_recognised() {
    let body = format!(
        "docs(evolution): synchronize product artifacts\n\n{}",
        render_marker(PR_MARKER_TAG, &marker()).unwrap()
    );
    assert!(is_sync_pull_request(&body));
}

#[test]
fn an_ordinary_bot_pull_request_is_not_a_sync_pull_request() {
    // The generic auto-merge hook must keep merging these exactly as before.
    assert!(!is_sync_pull_request("feat: add a thing\n\nCloses #12"));
    assert!(!is_sync_pull_request(""));
}

#[test]
fn a_sync_branch_is_recognised_only_in_the_base_repository() {
    let base = "acme/site";
    assert!(is_sync_branch(
        "fkst/evolution/c0ceeffb6f8cb4b3",
        Some(base),
        base
    ));

    // A FORK head ref is a bare branch name in the fork, which a contributor
    // controls. Without the repository check, opening a fork pull request from a
    // branch named like a lane would get it treated as Evolution-owned — and so
    // silently exempted from the generic auto-merge hook.
    assert!(!is_sync_branch(
        "fkst/evolution/c0ceeffb6f8cb4b3",
        Some("attacker/site"),
        base
    ));
    // A deleted head repository is likewise not the base repository.
    assert!(!is_sync_branch(
        "fkst/evolution/c0ceeffb6f8cb4b3",
        None,
        base
    ));
}

#[test]
fn a_branch_under_the_prefix_with_the_wrong_shape_is_not_a_lane() {
    let base = "acme/site";
    for head in [
        "fkst/evolution/not-hex-at-all!",
        "fkst/evolution/c0ceeffb",              // too short
        "fkst/evolution/c0ceeffb6f8cb4b3extra", // too long
        "fkst/evolution/",
        "fkst/evolutionary/c0ceeffb6f8cb4b3",
        "feature/x",
    ] {
        assert!(!is_sync_branch(head, Some(base), base), "{head}");
    }
}

#[test]
fn the_branch_a_lane_computes_is_recognised_as_one() {
    // Round-trip: whatever `sync_branch_name` emits must satisfy the predicate
    // the auto-merge exclusion uses, or the exclusion silently stops working.
    let base = "acme/site";
    let branch = sync_branch_name(INPUT).unwrap();
    assert!(is_sync_branch(&branch, Some(base), base));
}

#[test]
fn the_intent_proposal_pull_request_is_not_a_sync_pull_request() {
    // The one pull request that deliberately touches `intent/**` carries NO sync
    // marker, which is exactly how it is identified — and it must never be
    // merged by autonomous policy.
    let body = "docs(evolution): propose an intent change\n\nFor human review.";
    assert!(!is_sync_pull_request(body));
}

// ---- duplicate repair ------------------------------------------------------

fn candidates(numbers: &[i64]) -> Vec<LaneCandidate> {
    numbers
        .iter()
        .map(|n| LaneCandidate { number: *n })
        .collect()
}

#[test]
fn the_lowest_numbered_resource_is_canonical() {
    // Duplicates of one lane carry identical markers and the same author, so
    // creation order is the only totally-ordered signal available.
    let resolved = resolve_lane(&candidates(&[431, 417, 425]));
    assert_eq!(resolved.canonical, Some(417));
    assert_eq!(resolved.duplicates, vec![425, 431]);
    assert!(resolved.is_duplicated());
}

#[test]
fn a_single_resource_is_canonical_with_no_duplicates() {
    let resolved = resolve_lane(&candidates(&[417]));
    assert_eq!(resolved.canonical, Some(417));
    assert!(resolved.duplicates.is_empty());
    assert!(!resolved.is_duplicated(), "the normal case must not report");
}

#[test]
fn no_candidates_yields_no_canonical() {
    let resolved = resolve_lane(&[]);
    assert_eq!(resolved.canonical, None);
    assert!(!resolved.is_duplicated());
}

#[test]
fn a_repeated_observation_is_not_a_duplicate() {
    // The same issue seen twice in one listing pass is not duplication; treating
    // it as such would close the canonical resource against itself.
    let resolved = resolve_lane(&candidates(&[417, 417]));
    assert_eq!(resolved.canonical, Some(417));
    assert!(resolved.duplicates.is_empty());
}

#[test]
fn resolution_is_stable_regardless_of_observation_order() {
    let a = resolve_lane(&candidates(&[431, 417, 425]));
    let b = resolve_lane(&candidates(&[417, 431, 425]));
    assert_eq!(a, b);
}
