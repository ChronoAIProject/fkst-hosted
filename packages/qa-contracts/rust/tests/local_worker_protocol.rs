use fkst_qa_contracts::{
    canonical_bytes, contract_registry, encode_local_worker_frame, validate_local_worker_frame,
    LocalWorkerFrameDecoder,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    frames: Vec<FixtureFrame>,
}

#[derive(Deserialize)]
struct FixtureFrame {
    value: Value,
    canonical_utf8: String,
    wire_hex: String,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../fixtures/qa.local-worker-protocol/v1/happy-path.json"
    ))
    .expect("parse shared worker fixture")
}

fn decode_hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("hex pair"), 16).expect("hex byte")
        })
        .collect()
}

#[test]
fn registers_and_round_trips_the_shared_worker_transcript() {
    assert_eq!(
        contract_registry().expect("registry")["schemas"]["qa.local-worker-protocol/v1"]["major"],
        1
    );
    for frame in fixture().frames {
        let source = serde_json::to_vec(&frame.value).expect("serialize logical frame");
        let validated = validate_local_worker_frame(&source).expect("validate worker frame");
        assert_eq!(
            canonical_bytes(&validated).expect("canonical bytes"),
            frame.canonical_utf8.as_bytes()
        );
        assert_eq!(
            encode_local_worker_frame(&validated).expect("encode frame"),
            decode_hex(&frame.wire_hex)
        );
    }
}

#[test]
fn decodes_coalesced_and_fragmented_binary_frames() {
    let combined: Vec<u8> = fixture()
        .frames
        .iter()
        .flat_map(|frame| decode_hex(&frame.wire_hex))
        .collect();
    let mut coalesced = LocalWorkerFrameDecoder::default();
    assert_eq!(coalesced.push(&combined).expect("decode combined").len(), 16);
    coalesced.finish().expect("complete combined input");

    let mut fragmented = LocalWorkerFrameDecoder::default();
    let mut count = 0;
    count += fragmented
        .push(&combined[..2])
        .expect("prefix fragment")
        .len();
    count += fragmented
        .push(&combined[2..9])
        .expect("payload fragment")
        .len();
    count += fragmented
        .push(&combined[9..])
        .expect("remaining frames")
        .len();
    assert_eq!(count, 16);
    fragmented.finish().expect("complete fragmented input");
}
