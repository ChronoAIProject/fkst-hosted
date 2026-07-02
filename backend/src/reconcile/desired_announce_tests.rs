//! Tests for the planner's one-time session-announcement emission plus the
//! determinism / order-independence guarantees. Split out of `desired_plan_tests`
//! to keep each test file under the 500-line limit. Fixtures live in
//! [`super::desired_test_fixtures`].

use std::collections::HashSet;

use super::desired_test_fixtures::*;
use super::{full_config_hash, plan_repo, PodLiveness, ReconcileAction};

// ---- session announcement --------------------------------------------------

#[test]
fn valid_registration_not_yet_announced_is_announced() {
    // A fresh valid registration with no queued work (absent pod, not pending) still
    // announces — the announce is independent of Spawn/pending.
    let regs = vec![reg("s1", 1, "h")];
    let actions = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[("s1", false)]),
        &latched(&[]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::AnnounceSession {
            trigger_issue: 1,
            session_name: "demo".to_string(),
            work_label: "wl".to_string(),
            packages: vec![],
            environment: None,
            auto_merge: false,
            full_config_hash: full_config_hash(&regs[0]),
        }]
    );
}

#[test]
fn valid_registration_announces_alongside_spawn() {
    // With queued work (pending + absent) BOTH Spawn and AnnounceSession fire; the
    // spawn is emitted first (registration-driven pass), the announce second.
    let regs = vec![reg("s1", 1, "h")];
    let actions = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[("s1", true)]),
        &latched(&[]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![
            ReconcileAction::Spawn(regs[0].clone()),
            ReconcileAction::AnnounceSession {
                trigger_issue: 1,
                session_name: "demo".to_string(),
                work_label: "wl".to_string(),
                packages: vec![],
                environment: None,
                auto_merge: false,
                full_config_hash: full_config_hash(&regs[0]),
            },
        ]
    );
}

#[test]
fn already_announced_registration_is_not_reannounced() {
    // Issue 1 already carries the durable announced label -> no second announcement.
    let regs = vec![reg("s1", 1, "h")];
    let actions = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[("s1", false)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(
        actions.is_empty(),
        "a latched-announced issue is not reannounced"
    );
}

#[test]
fn invalid_trigger_is_never_announced() {
    // An invalid trigger (no registration) is flagged, never announced.
    let invalid = vec![(5, "missing `### Work Label`".to_string())];
    let actions = plan_repo(
        &[],
        &invalid,
        &[],
        &pending(&[]),
        &latched(&[]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::FlagInvalid {
            trigger_issue: 5,
            detail: "missing `### Work Label`".to_string(),
        }],
        "no AnnounceSession for an invalid trigger"
    );
}

#[test]
fn announcement_carries_rendered_packages_and_auto_merge() {
    // The action carries the rendered `owner/repo@ref:path` refs (author order) + the
    // per-session auto-merge opt-in, ready for the pure comment renderer.
    let mut r = reg("s1", 1, "h");
    r.def.packages = vec![
        pkg(
            "ChronoAIProject",
            "fkst-packages",
            "dev",
            "packages/github-devloop",
        ),
        pkg("acme", "pkgs", "main", "packages/proxy"),
    ];
    r.def.environment = Some("prod".to_string());
    r.auto_merge = true;
    let regs = vec![r];
    let actions = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[("s1", false)]),
        &latched(&[]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::AnnounceSession {
            trigger_issue: 1,
            session_name: "demo".to_string(),
            work_label: "wl".to_string(),
            packages: vec![
                "ChronoAIProject/fkst-packages@dev:packages/github-devloop".to_string(),
                "acme/pkgs@main:packages/proxy".to_string(),
            ],
            environment: Some("prod".to_string()),
            auto_merge: true,
            full_config_hash: full_config_hash(&regs[0]),
        }]
    );
}

// ---- determinism / order-independence --------------------------------------

#[test]
fn clear_invalid_output_is_order_independent_of_the_set() {
    let regs = vec![reg("s3", 3, "h"), reg("s5", 5, "h"), reg("s8", 8, "h")];
    // Two logically-equal sets built by inserting the ids in different orders.
    let a: HashSet<i64> = [3, 5, 8].into_iter().collect();
    let b: HashSet<i64> = [8, 3, 5].into_iter().collect();
    // Suppress the announces (all three issues) so the assertion stays about the
    // order-independence of the ClearInvalid output.
    let announced = latched(&[3, 5, 8]);
    let plan_a = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[]),
        &a,
        &announced,
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    let plan_b = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[]),
        &b,
        &announced,
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        plan_a, plan_b,
        "set iteration order must not leak into output"
    );
    assert_eq!(
        plan_a,
        vec![
            ReconcileAction::ClearInvalid { trigger_issue: 3 },
            ReconcileAction::ClearInvalid { trigger_issue: 5 },
            ReconcileAction::ClearInvalid { trigger_issue: 8 },
        ],
        "cleared issues are emitted in ascending order"
    );
}

#[test]
fn plan_output_is_order_independent_of_the_pending_map() {
    let regs = vec![reg("s1", 1, "h"), reg("s2", 2, "h")];
    let live = vec![
        pod("s1", 1, PodLiveness::Live, ago(10), Some(ago(1)), Some("h")),
        pod("s2", 2, PodLiveness::Live, ago(10), Some(ago(1)), Some("h")),
    ];
    let m1 = pending(&[("s1", true), ("s2", true)]);
    let m2 = pending(&[("s2", true), ("s1", true)]);
    let announced = latched(&[1, 2]);
    let p1 = plan_repo(
        &regs,
        &[],
        &live,
        &m1,
        &latched(&[]),
        &announced,
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    let p2 = plan_repo(
        &regs,
        &[],
        &live,
        &m2,
        &latched(&[]),
        &announced,
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(p1, p2);
}
