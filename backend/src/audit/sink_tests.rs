//! The sink boundary: the disabled no-op and the recording test double.

use super::*;
use crate::audit::test_support::human_event;

#[tokio::test]
async fn the_disabled_sink_accepts_discards_and_reports_itself() {
    let sink = DisabledSink;
    sink.submit(human_event())
        .expect("the no-op sink never refuses");
    assert_eq!(sink.queue_depth(), 0);
    assert!(!sink.is_delivering());
    assert_eq!(sink.drain().await, DrainReport::default());
}

#[tokio::test]
async fn the_recording_sink_keeps_every_event_in_order() {
    let sink = RecordingSink::new(8);
    assert!(sink.is_empty());

    let first = human_event();
    let mut second = human_event();
    second.request_id = "req-0002".to_string();
    sink.submit(first.clone()).expect("accepted");
    sink.submit(second.clone()).expect("accepted");

    assert_eq!(sink.len(), 2);
    assert_eq!(sink.queue_depth(), 2);
    assert!(sink.is_delivering());
    let recorded = sink.events();
    assert_eq!(recorded[0].request_id, first.request_id);
    assert_eq!(recorded[1].request_id, second.request_id);
    assert_eq!(sink.drain().await.remaining, 0);
}

#[test]
fn the_recording_sink_is_bounded_like_the_real_queue() {
    let sink = RecordingSink::new(1);
    sink.submit(human_event()).expect("first fits");
    assert_eq!(sink.submit(human_event()), Err(SubmitError::QueueFull));
    assert_eq!(sink.len(), 1);
}

#[test]
fn recording_sink_clones_share_one_buffer() {
    // Clones must observe the same events, or a handle held by the application
    // and a handle held by a test would disagree.
    let sink = RecordingSink::new(4);
    let clone = sink.clone();
    sink.submit(human_event()).expect("accepted");
    assert_eq!(clone.len(), 1);
}
