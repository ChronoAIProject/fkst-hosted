//! Tests for [`ChatLimits`] (sibling `#[path]` module).

use super::*;

/// Assert the error is a 429-shaped rate limit carrying a retry hint.
fn assert_rate_limited(error: AppError) {
    match error {
        AppError::RateLimited {
            retry_after_secs, ..
        } => assert!(retry_after_secs > 0, "a retry hint must be advertised"),
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[tokio::test]
async fn a_second_concurrent_turn_from_the_same_user_is_rejected() {
    let limits = ChatLimits::new(4);
    let _first = limits
        .admit(1001)
        .await
        .expect("the first turn is admitted");
    let error = limits
        .admit(1001)
        .await
        .expect_err("a double-submit must be rejected");
    assert_rate_limited(error);
}

#[tokio::test]
async fn distinct_users_run_concurrently() {
    let limits = ChatLimits::new(4);
    let _a = limits.admit(1).await.expect("user 1 admitted");
    let _b = limits.admit(2).await.expect("user 2 admitted");
    let _c = limits.admit(3).await.expect("user 3 admitted");
}

#[tokio::test]
async fn dropping_the_guard_lets_the_same_user_start_again() {
    let limits = ChatLimits::new(4);
    {
        let _turn = limits.admit(1001).await.expect("admitted");
    }
    limits
        .admit(1001)
        .await
        .expect("the slot must be released when the turn ends");
}

#[tokio::test(start_paused = true)]
async fn a_saturated_ceiling_is_rejected_after_the_grace() {
    // `start_paused` makes the admission grace elapse instantly instead of the test
    // sleeping for it.
    let limits = ChatLimits::new(2);
    let _a = limits.admit(1).await.expect("first slot");
    let _b = limits.admit(2).await.expect("second slot");
    let error = limits
        .admit(3)
        .await
        .expect_err("a third turn must be told to retry");
    assert_rate_limited(error);
}

#[tokio::test(start_paused = true)]
async fn a_capacity_rejection_does_not_lock_the_user_out() {
    // The regression this guards: taking the per-user entry, then failing on
    // capacity, would leave the id in the in-flight set forever — that account
    // could never chat again until the process restarted.
    let limits = ChatLimits::new(1);
    let held = limits.admit(1).await.expect("first slot");
    limits
        .admit(2)
        .await
        .expect_err("capacity is saturated for user 2");
    drop(held);
    limits
        .admit(2)
        .await
        .expect("user 2 must be admitted once a slot frees");
}

#[tokio::test]
async fn a_freed_slot_admits_a_waiting_turn() {
    let limits = ChatLimits::new(1);
    let held = limits.admit(1).await.expect("first slot");
    drop(held);
    limits.admit(2).await.expect("the freed slot is reusable");
}
