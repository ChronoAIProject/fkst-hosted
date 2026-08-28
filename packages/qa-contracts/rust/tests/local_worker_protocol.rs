use fkst_qa_contracts::{
    canonical_bytes, contract_registry, encode_local_worker_frame, validate_local_worker_frame,
    ContractError, LocalWorkerFrameDecoder, LocalWorkerInputSequence, Rejection, ValidatedValue,
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

#[derive(Deserialize)]
struct NegativeFixture {
    frame_cases: Vec<FrameCase>,
    framing_cases: Vec<FramingCase>,
    sequence_cases: Vec<SequenceCase>,
}

#[derive(Deserialize)]
struct ExpectedRejection {
    category: String,
    code: Option<String>,
    reason: String,
    path: String,
}

#[derive(Clone, Deserialize)]
struct Mutation {
    path: Vec<Value>,
    value: Value,
}

#[derive(Deserialize)]
struct FrameCase {
    case_id: String,
    happy_index: Option<usize>,
    source_utf8: Option<String>,
    source_hex: Option<String>,
    replacement: Option<Value>,
    mutation: Option<Mutation>,
    expected: ExpectedRejection,
}

#[derive(Deserialize)]
struct FramingCase {
    case_id: String,
    wire_hex: Option<String>,
    happy_index: Option<usize>,
    suffix_hex: Option<String>,
    phase: String,
    expected: ExpectedRejection,
}

#[derive(Deserialize)]
struct SequenceFrame {
    happy_index: usize,
    mutation: Option<Mutation>,
}

#[derive(Deserialize)]
struct SequenceCase {
    case_id: String,
    frames: Vec<SequenceFrame>,
    #[serde(default)]
    finish: bool,
    expected: ExpectedRejection,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../fixtures/qa.local-worker-protocol/v1/happy-path.json"
    ))
    .expect("parse shared worker fixture")
}

fn negative_fixture() -> NegativeFixture {
    serde_json::from_str(include_str!(
        "../../fixtures/qa.local-worker-protocol/v1/negative.json"
    ))
    .expect("parse shared negative worker fixture")
}

fn decode_hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
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
fn rejects_shared_negative_worker_frames() {
    let fixture = fixture();
    for fixture_case in negative_fixture().frame_cases {
        let raw = frame_case_bytes(&fixture, &fixture_case);
        let error = validate_local_worker_frame(&raw).expect_err(&fixture_case.case_id);
        assert_rejection(&error, &fixture_case.expected, &fixture_case.case_id);
    }
}

#[test]
fn rejects_shared_negative_worker_framing() {
    let fixture = fixture();
    for fixture_case in negative_fixture().framing_cases {
        let mut decoder = LocalWorkerFrameDecoder::default();
        let wire = framing_case_bytes(&fixture, &fixture_case);
        let error = match decoder.push(&wire) {
            Err(error) => error,
            Ok(_) if fixture_case.phase == "finish" => {
                decoder.finish().expect_err(&fixture_case.case_id)
            }
            Ok(_) => panic!("{} unexpectedly passed", fixture_case.case_id),
        };
        assert_rejection(&error, &fixture_case.expected, &fixture_case.case_id);
    }
}

#[test]
fn rejects_shared_negative_worker_sequences() {
    let fixture = fixture();
    for fixture_case in negative_fixture().sequence_cases {
        let mut sequence = LocalWorkerInputSequence::default();
        let mut observed = None;
        for frame in &fixture_case.frames {
            let validated =
                validated_fixture_frame(&fixture, frame.happy_index, frame.mutation.as_ref());
            if let Err(error) = sequence.accept(&validated) {
                observed = Some(error);
                break;
            }
        }
        if observed.is_none() && fixture_case.finish {
            observed = sequence.finish().err();
        }
        let error = observed.expect(&fixture_case.case_id);
        assert_rejection(&error, &fixture_case.expected, &fixture_case.case_id);
    }
}

#[test]
fn accepts_exact_inbound_sequence_and_clean_eof() {
    let fixture = fixture();
    let mut sequence = LocalWorkerInputSequence::default();
    for index in [0, 2, 4, 6, 8, 10, 12, 14] {
        let frame = validated_fixture_frame(&fixture, index, None);
        sequence.accept(&frame).expect("accept exact inbound frame");
    }
    sequence.finish().expect("accept clean EOF");
}

#[test]
fn decodes_coalesced_and_fragmented_binary_frames() {
    let combined: Vec<u8> = fixture()
        .frames
        .iter()
        .flat_map(|frame| decode_hex(&frame.wire_hex))
        .collect();
    let mut coalesced = LocalWorkerFrameDecoder::default();
    assert_eq!(
        coalesced.push(&combined).expect("decode combined").len(),
        16
    );
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

fn frame_case_bytes(fixture: &Fixture, fixture_case: &FrameCase) -> Vec<u8> {
    if let Some(source_hex) = &fixture_case.source_hex {
        return decode_hex(source_hex);
    }
    if let Some(source_utf8) = &fixture_case.source_utf8 {
        return source_utf8.as_bytes().to_vec();
    }
    let value = fixture_case.replacement.clone().unwrap_or_else(|| {
        mutated_fixture_value(
            fixture,
            fixture_case.happy_index.expect("happy index"),
            fixture_case.mutation.as_ref(),
        )
    });
    serde_json::to_vec(&value).expect("serialize negative frame")
}

fn framing_case_bytes(fixture: &Fixture, fixture_case: &FramingCase) -> Vec<u8> {
    if let Some(wire_hex) = &fixture_case.wire_hex {
        return decode_hex(wire_hex);
    }
    let mut wire =
        decode_hex(&fixture.frames[fixture_case.happy_index.expect("happy index")].wire_hex);
    if let Some(suffix_hex) = &fixture_case.suffix_hex {
        wire.extend_from_slice(&decode_hex(suffix_hex));
    }
    wire
}

fn validated_fixture_frame(
    fixture: &Fixture,
    index: usize,
    mutation: Option<&Mutation>,
) -> ValidatedValue {
    let value = mutated_fixture_value(fixture, index, mutation);
    validate_local_worker_frame(&serde_json::to_vec(&value).expect("serialize fixture frame"))
        .expect("validate fixture frame")
}

fn mutated_fixture_value(fixture: &Fixture, index: usize, mutation: Option<&Mutation>) -> Value {
    let mut value = fixture.frames[index].value.clone();
    if let Some(mutation) = mutation {
        set_path(&mut value, &mutation.path, mutation.value.clone());
    }
    value
}

fn set_path(root: &mut Value, path: &[Value], value: Value) {
    let (last, parents) = path.split_last().expect("non-empty mutation path");
    let mut current = root;
    for token in parents {
        current = match token {
            Value::String(key) => current.get_mut(key).expect("object mutation path"),
            Value::Number(index) => current
                .get_mut(index.as_u64().expect("array index") as usize)
                .expect("array mutation path"),
            _ => panic!("invalid mutation path token"),
        };
    }
    match last {
        Value::String(key) => {
            current
                .as_object_mut()
                .expect("object mutation target")
                .insert(key.clone(), value);
        }
        Value::Number(index) => {
            current.as_array_mut().expect("array mutation target")
                [index.as_u64().expect("array index") as usize] = value;
        }
        _ => panic!("invalid mutation path token"),
    }
}

fn assert_rejection(error: &ContractError, expected: &ExpectedRejection, case_id: &str) {
    let Rejection {
        category,
        code,
        reason,
        path,
    } = &error.0;
    assert_eq!(*category, expected.category, "{case_id}: category");
    assert_eq!(code.map(str::to_owned), expected.code, "{case_id}: code");
    assert_eq!(*reason, expected.reason, "{case_id}: reason");
    assert_eq!(*path, expected.path, "{case_id}: path");
}
