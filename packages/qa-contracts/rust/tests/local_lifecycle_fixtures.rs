use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use fkst_qa_contracts::{canonical_bytes, sha256_digest, validate_local_state};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct LifecycleFixture {
    schema_version: String,
    valid_cases: Vec<LifecycleValidCase>,
}

#[derive(Deserialize)]
struct LifecycleValidCase {
    case_id: String,
    source: Value,
    expected_canonical_utf8_hex: String,
    expected_canonical_utf8_base64: String,
    expected_sha256: String,
}

#[test]
fn local_lifecycle_fixture_walks_the_production_path() {
    let fixture: LifecycleFixture =
        load_json(&repo_root().join("fixtures/qa/local-lifecycle-v1.json"));
    assert_eq!(
        fixture.schema_version,
        "qa.local-lifecycle-fixtures/v1"
    );

    for fixture_case in fixture.valid_cases {
        println!("case_id={}", fixture_case.case_id);
        let raw = serde_json::to_vec(&fixture_case.source).expect("serialize fixture source");
        assert_eq!(raw, b"\"accepted\"");
        let validated = validate_local_state(&raw).expect("validate LocalState");
        assert_eq!(validated.value().as_str(), Some("accepted"));
        let canonical = canonical_bytes(&validated).expect("canonical LocalState bytes");
        assert_eq!(hex(&canonical), fixture_case.expected_canonical_utf8_hex);
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(&canonical),
            fixture_case.expected_canonical_utf8_base64
        );
        assert_eq!(sha256_digest(&canonical), fixture_case.expected_sha256);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repository root")
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}
