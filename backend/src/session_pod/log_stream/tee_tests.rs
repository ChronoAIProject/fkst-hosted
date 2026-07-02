//! Tests for the supervise-stream tee splitter. Split into a sibling file so
//! `tee.rs` stays under the 500-line module cap. The load-bearing assertion is that
//! the inherited stream stays byte-for-byte identical to the child's output.

use super::*;

use std::sync::mpsc::sync_channel;

#[tokio::test]
async fn tee_preserves_the_inherited_stream_byte_for_byte() {
    let input = b"line one\nline two\npartial-no-newline";
    let mut inherited: Vec<u8> = Vec::new();
    let (tx, _rx) = sync_channel(1024);

    tee_reader(&input[..], &mut inherited, tx, LogClass::Supervise)
        .await
        .expect("tee completes");

    // The inherited sink must equal the child's raw bytes exactly — nothing dropped,
    // reordered, or reframed.
    assert_eq!(inherited, input, "inherited stream must be byte-identical");
}

#[tokio::test]
async fn tee_forwards_each_complete_line_plus_the_final_tail() {
    let input = b"alpha\nbeta\ngamma";
    let mut inherited: Vec<u8> = Vec::new();
    let (tx, rx) = sync_channel(1024);

    tee_reader(&input[..], &mut inherited, tx, LogClass::Supervise)
        .await
        .expect("tee completes");

    let forwarded: Vec<(LogClass, String)> = rx.try_iter().collect();
    assert_eq!(
        forwarded,
        vec![
            (LogClass::Supervise, "alpha".to_string()),
            (LogClass::Supervise, "beta".to_string()),
            // The unterminated trailing line is flushed at EOF.
            (LogClass::Supervise, "gamma".to_string()),
        ]
    );
}

#[tokio::test]
async fn tee_tags_lines_with_the_given_class() {
    let input = b"driver record\n";
    let mut inherited: Vec<u8> = Vec::new();
    let (tx, rx) = sync_channel(16);

    tee_reader(&input[..], &mut inherited, tx, LogClass::HostedDriver)
        .await
        .expect("tee completes");

    let first = rx.try_recv().expect("one record");
    assert_eq!(first, (LogClass::HostedDriver, "driver record".to_string()));
}

#[tokio::test]
async fn tee_does_not_block_when_the_channel_is_full() {
    // A capacity-1 channel with no receiver draining: try_send drops, the tee must
    // still drain the reader and keep the inherited stream whole.
    let input = b"one\ntwo\nthree\nfour\n";
    let mut inherited: Vec<u8> = Vec::new();
    let (tx, _rx) = sync_channel(1);

    tee_reader(&input[..], &mut inherited, tx, LogClass::Supervise)
        .await
        .expect("tee completes without blocking");

    assert_eq!(
        inherited, input,
        "inherited stream stays complete under backpressure"
    );
}
