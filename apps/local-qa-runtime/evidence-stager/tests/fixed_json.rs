use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use fkst_local_qa_evidence_stager::{
    CleanupResidualReason, EvidenceMediaType, EvidenceRole, EvidenceStager, FixedJsonExportRequest,
    FixedJsonNamespace, StageRequest, StagerError,
};
use fkst_qa_contracts::{
    admit_json, canonical_admitted_bytes, canonical_bytes, sha256_digest,
    validate_local_fixed_json_export, LOCAL_FIXED_JSON_MAX_BYTES,
};
use serde_json::{json, Value};
use tempfile::TempDir;

fn corpus() -> Value {
    serde_json::from_str(include_str!(
        "../../../../packages/qa-contracts/fixtures/qa.local-fixed-json-export/v1/conformance.json"
    ))
    .unwrap()
}

fn raw() -> Vec<u8> {
    corpus()["source_cases"][0]["raw_utf8"]
        .as_str()
        .unwrap()
        .as_bytes()
        .to_vec()
}

fn root(temp: &TempDir) -> PathBuf {
    temp.path().canonicalize().unwrap().join("quarantine")
}

fn request(bytes: &[u8]) -> FixedJsonExportRequest<'_> {
    FixedJsonExportRequest {
        run_id: "run-1",
        attempt: 1,
        observation_id: "observation/0",
        raw_bytes: bytes,
    }
}

fn paths() -> Vec<String> {
    corpus()["crash_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect()
}

fn rehash(mut value: Value) -> Vec<u8> {
    value.as_object_mut().unwrap().remove("content_digest");
    let projection =
        canonical_admitted_bytes(&admit_json(&serde_json::to_vec(&value).unwrap()).unwrap())
            .unwrap();
    value["content_digest"] = sha256_digest(&projection).into();
    canonical_admitted_bytes(&admit_json(&serde_json::to_vec(&value).unwrap()).unwrap()).unwrap()
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn shared_sources_only_export_exact_closed_json_and_errors_never_echo_canaries() {
    let fixture = corpus();
    for case in fixture["source_cases"].as_array().unwrap() {
        let temp = TempDir::new().unwrap();
        let root = root(&temp);
        let stager = EvidenceStager::new(&root);
        let bytes = case["raw_utf8"].as_str().unwrap().as_bytes();
        let result = stager.stage_fixed_json_export(request(bytes));
        if case["accepted"] == true {
            let handle = result.unwrap_or_else(|error| panic!("{}: {error}", case["id"]));
            let exported = stager.read_fixed_json_export(&handle).unwrap();
            assert_eq!(
                exported.bytes(),
                case["canonical_utf8"].as_str().unwrap().as_bytes()
            );
            assert_eq!(
                exported.receipt().value()["source_digest"],
                case["source_digest"]
            );
            assert_eq!(
                exported.receipt().value()["output_digest"],
                case["output_digest"]
            );
            assert_eq!(
                fs::read(root.join("fixed-json/run-1/1/raw/0.json")).unwrap(),
                bytes
            );
            validate_local_fixed_json_export(
                &canonical_bytes(exported.receipt()).unwrap(),
                bytes,
                exported.bytes(),
            )
            .unwrap();
            let debug = format!("{handle:?}");
            assert!(!debug.contains("http://") && !debug.contains(root.to_str().unwrap()));
        } else {
            let error = result.unwrap_err();
            assert_eq!(error, StagerError::InvalidObject, "{}", case["id"]);
            assert!(!format!("{error:?} {error}").contains(fixture["canary"].as_str().unwrap()));
            assert!(!root.exists());
        }
    }
    let temp = TempDir::new().unwrap();
    let stager = EvidenceStager::new(root(&temp));
    let png_hex = fixture["png_hex"].as_str().unwrap();
    let png: Vec<u8> = (0..png_hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&png_hex[index..index + 2], 16).unwrap())
        .collect();
    assert_eq!(
        stager.stage_fixed_json_export(request(&png)).unwrap_err(),
        StagerError::InvalidObject
    );
}

#[test]
fn durable_replay_preserves_original_receipt_timestamp_bytes_and_raw_identity() {
    let temp = TempDir::new().unwrap();
    let root = root(&temp);
    let bytes = raw();
    let stager = EvidenceStager::new(&root);
    let first = stager.stage_fixed_json_export(request(&bytes)).unwrap();
    let original = stager.read_fixed_json_export(&first).unwrap();
    let receipt_bytes = canonical_bytes(original.receipt()).unwrap();
    assert!(
        original.receipt().value()["created_at_unix_ms"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_ne!(
        original.receipt().value()["source_digest"],
        original.receipt().value()["output_digest"]
    );
    drop(stager);
    let restarted = EvidenceStager::new(&root);
    let second = restarted.stage_fixed_json_export(request(&bytes)).unwrap();
    let replay = restarted.read_fixed_json_export(&second).unwrap();
    assert_eq!(receipt_bytes, canonical_bytes(replay.receipt()).unwrap());
    assert_eq!(original.bytes(), replay.bytes());
    assert_eq!(
        fs::read(root.join("fixed-json/run-1/1/export/0.receipt.json")).unwrap(),
        receipt_bytes
    );
    assert!(restarted.read_fixed_json_export(&first).is_ok());
    // Equal canonical output does not erase raw source identity.
    assert_eq!(
        restarted
            .stage_fixed_json_export(request(replay.bytes()))
            .unwrap_err(),
        StagerError::DuplicateIdentity
    );
    let changed_url = String::from_utf8(bytes.clone())
        .unwrap()
        .replace("49152", "49153");
    assert_eq!(
        restarted
            .stage_fixed_json_export(request(changed_url.as_bytes()))
            .unwrap_err(),
        StagerError::DuplicateIdentity
    );
}

#[test]
fn every_incomplete_publication_combination_is_ineligible_and_scoped_cleanup_converges() {
    let temp = TempDir::new().unwrap();
    let root = root(&temp);
    let bytes = raw();
    let stager = EvidenceStager::new(&root);
    stager.stage_fixed_json_export(request(&bytes)).unwrap();
    let paths = paths();
    let originals: Vec<Vec<u8>> = paths
        .iter()
        .map(|path| fs::read(root.join("fixed-json/run-1/1").join(path)).unwrap())
        .collect();
    for mask in 1..15 {
        let incomplete = root.parent().unwrap().join(format!("interrupted-{mask}"));
        for (index, path) in paths.iter().enumerate() {
            if mask & (1 << index) != 0 {
                write(
                    &incomplete.join("fixed-json/run-1/1").join(path),
                    &originals[index],
                );
            }
        }
        let reopened = EvidenceStager::new(&incomplete);
        assert!(
            reopened.stage_fixed_json_export(request(&bytes)).is_err(),
            "mask={mask}"
        );
        for namespace in [FixedJsonNamespace::Raw, FixedJsonNamespace::Export] {
            let status = reopened.fixed_json_status("run-1", 1, namespace).unwrap();
            assert!(status.files <= 2);
            assert!(reopened
                .cleanup_fixed_json("run-1", 1, namespace)
                .unwrap()
                .is_complete());
            assert_eq!(
                reopened
                    .fixed_json_status("run-1", 1, namespace)
                    .unwrap()
                    .files,
                0
            );
        }
    }
}

#[test]
fn missing_or_corrupted_members_revoke_live_handles_and_restart_replay() {
    let temp = TempDir::new().unwrap();
    let root = root(&temp);
    let bytes = raw();
    let stager = EvidenceStager::new(&root);
    let handle = stager.stage_fixed_json_export(request(&bytes)).unwrap();
    for relative in paths() {
        let path = root.join("fixed-json/run-1/1").join(relative);
        let original = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(stager.read_fixed_json_export(&handle).is_err());
        assert!(EvidenceStager::new(&root)
            .stage_fixed_json_export(request(&bytes))
            .is_err());
        fs::write(&path, b"PRIVATE_VALUE_6146_CANARY").unwrap();
        let error = stager.read_fixed_json_export(&handle).err().unwrap();
        assert!(!format!("{error:?} {error}").contains("PRIVATE_VALUE_6146_CANARY"));
        assert!(EvidenceStager::new(&root)
            .stage_fixed_json_export(request(&bytes))
            .is_err());
        fs::write(&path, vec![b' '; LOCAL_FIXED_JSON_MAX_BYTES + 1]).unwrap();
        assert!(stager.read_fixed_json_export(&handle).is_err());
        fs::write(&path, original).unwrap();
        assert!(stager.read_fixed_json_export(&handle).is_ok());
    }
}

#[test]
fn receipt_changes_cannot_rewrite_timestamp_policy_or_identity_after_restart() {
    let temp = TempDir::new().unwrap();
    let root = root(&temp);
    let bytes = raw();
    let stager = EvidenceStager::new(&root);
    let handle = stager.stage_fixed_json_export(request(&bytes)).unwrap();
    let exported = stager.read_fixed_json_export(&handle).unwrap();
    let receipt_path = root.join("fixed-json/run-1/1/export/0.receipt.json");
    let anchor_path = root.join("fixed-json/run-1/1/raw/0.receipt-digest");
    let original_receipt = fs::read(&receipt_path).unwrap();
    let original_anchor = fs::read(&anchor_path).unwrap();
    let mut changed = exported.receipt().value().clone();
    changed["created_at_unix_ms"] = json!(1);
    fs::write(&receipt_path, rehash(changed)).unwrap();
    assert!(stager.read_fixed_json_export(&handle).is_err());
    assert!(EvidenceStager::new(&root)
        .stage_fixed_json_export(request(&bytes))
        .is_err());
    for patch in [
        json!({"run_id":"run-2"}),
        json!({"attempt":2}),
        json!({"object_id":"observation/1"}),
        json!({"role":"runner-log"}),
        json!({"policy_version":2}),
        json!({"media_type":"image/png"}),
        json!({"policy_digest":format!("sha256:{}", "0".repeat(64))}),
    ] {
        let mut changed = exported.receipt().value().clone();
        changed
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let changed = rehash(changed);
        // Also rewrite the local anchor: identity and policy checks must stand independently.
        fs::write(&receipt_path, &changed).unwrap();
        fs::write(&anchor_path, sha256_digest(&changed)).unwrap();
        assert!(stager.read_fixed_json_export(&handle).is_err());
        assert!(EvidenceStager::new(&root)
            .stage_fixed_json_export(request(&bytes))
            .is_err());
    }
    fs::write(receipt_path, original_receipt).unwrap();
    fs::write(anchor_path, original_anchor).unwrap();
    assert!(stager.read_fixed_json_export(&handle).is_ok());
}

#[test]
fn exact_input_bound_and_two_object_quota_do_not_mutate_legacy_local_evidence() {
    let temp = TempDir::new().unwrap();
    let root = root(&temp);
    let original = String::from_utf8(raw()).unwrap();
    let exact = format!(
        "{{{}{}",
        " ".repeat(LOCAL_FIXED_JSON_MAX_BYTES - original.len()),
        &original[1..]
    );
    let stager = EvidenceStager::new(&root);
    stager
        .stage_fixed_json_export(request(exact.as_bytes()))
        .unwrap();
    let mut second = request(exact.as_bytes());
    second.observation_id = "observation/1";
    stager.stage_fixed_json_export(second).unwrap();
    let mut third = request(original.as_bytes());
    third.observation_id = "observation/2";
    assert_eq!(
        stager.stage_fixed_json_export(third).unwrap_err(),
        StagerError::QuotaExceeded
    );
    assert_eq!(
        stager
            .stage_fixed_json_export(request(&vec![255; LOCAL_FIXED_JSON_MAX_BYTES + 1]))
            .unwrap_err(),
        StagerError::ObjectTooLarge
    );
    let raw_log = b"PRIVATE_VALUE_6146_CANARY\nstdout\n";
    let legacy = stager
        .stage(StageRequest {
            run_id: "run-1",
            attempt: 1,
            object_id: "evidence/0",
            role: EvidenceRole::RunnerLog,
            media_type: EvidenceMediaType::PlainTextUtf8,
            bytes: raw_log,
        })
        .unwrap();
    assert_eq!(
        legacy.object().value()["ownership"],
        "local-only:not-uploadable"
    );
    assert_eq!(
        fs::read(root.join("run-1/1/evidence/0.bin")).unwrap(),
        raw_log
    );
    assert!(stager.stage_fixed_json_export(request(raw_log)).is_err());
    assert!(stager.verify(&legacy).is_ok());
}

#[test]
fn cleanup_and_status_are_exactly_run_attempt_and_namespace_scoped() {
    let temp = TempDir::new().unwrap();
    let root = root(&temp);
    let bytes = raw();
    let stager = EvidenceStager::new(&root);
    assert_eq!(
        stager
            .fixed_json_status("run-1", 1, FixedJsonNamespace::Raw)
            .unwrap()
            .files,
        0
    );
    assert!(!root.exists());
    let first = stager.stage_fixed_json_export(request(&bytes)).unwrap();
    let other_run = String::from_utf8(bytes.clone())
        .unwrap()
        .replace("run-1", "run-2");
    let other = stager
        .stage_fixed_json_export(FixedJsonExportRequest {
            run_id: "run-2",
            ..request(other_run.as_bytes())
        })
        .unwrap();
    let other_attempt = String::from_utf8(bytes.clone())
        .unwrap()
        .replace("\"attempt\":1", "\"attempt\":2");
    let attempt = stager
        .stage_fixed_json_export(FixedJsonExportRequest {
            attempt: 2,
            ..request(other_attempt.as_bytes())
        })
        .unwrap();
    let raw_status = stager
        .fixed_json_status("run-1", 1, FixedJsonNamespace::Raw)
        .unwrap();
    assert_eq!(raw_status.files, 2);
    assert_eq!(raw_status.byte_length, bytes.len() as u64 + 71);
    let export_status = stager
        .fixed_json_status("run-1", 1, FixedJsonNamespace::Export)
        .unwrap();
    assert_eq!(export_status.files, 2);
    assert!(stager
        .cleanup_fixed_json("run-1", 1, FixedJsonNamespace::Raw)
        .unwrap()
        .is_complete());
    assert_eq!(
        stager
            .fixed_json_status("run-1", 1, FixedJsonNamespace::Raw)
            .unwrap()
            .files,
        0
    );
    assert_eq!(
        stager
            .fixed_json_status("run-1", 1, FixedJsonNamespace::Export)
            .unwrap(),
        export_status
    );
    assert!(stager.read_fixed_json_export(&first).is_err());
    assert!(stager.read_fixed_json_export(&other).is_ok());
    assert!(stager.read_fixed_json_export(&attempt).is_ok());
    assert!(stager
        .cleanup_fixed_json("run-1", 1, FixedJsonNamespace::Export)
        .unwrap()
        .is_complete());
    assert!(stager
        .cleanup_fixed_json("run-1", 1, FixedJsonNamespace::Export)
        .unwrap()
        .is_complete());
    assert!(stager.read_fixed_json_export(&other).is_ok());
}

#[test]
fn confined_ids_reject_paths_and_cross_stager_handles() {
    let temp = TempDir::new().unwrap();
    let root = root(&temp);
    let bytes = raw();
    let stager = EvidenceStager::new(&root);
    for id in [
        "../observation/0",
        "observation/../0",
        "/observation/0",
        "observation/0.json",
        "evidence/0",
    ] {
        assert!(stager
            .stage_fixed_json_export(FixedJsonExportRequest {
                observation_id: id,
                ..request(&bytes)
            })
            .is_err());
    }
    for run_id in ["../run-2", "run-1/../run-2", "/run-1", "", "run-2"] {
        assert!(stager
            .stage_fixed_json_export(FixedJsonExportRequest {
                run_id,
                ..request(&bytes)
            })
            .is_err());
    }
    assert!(stager
        .stage_fixed_json_export(FixedJsonExportRequest {
            attempt: 2,
            ..request(&bytes)
        })
        .is_err());
    assert!(stager
        .cleanup_fixed_json("../run-1", 1, FixedJsonNamespace::Raw)
        .is_err());
    assert!(!root.exists());
    let handle = stager.stage_fixed_json_export(request(&bytes)).unwrap();
    assert!(EvidenceStager::new(root.with_file_name("other"))
        .read_fixed_json_export(&handle)
        .is_err());
    assert!(EvidenceStager::new(root.join("..").join("quarantine"))
        .stage_fixed_json_export(request(&bytes))
        .is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_hardlinks_and_unrelated_cleanup_entries_never_escape_ownership() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let root = root(&temp);
    let parent = root.parent().unwrap();
    let outside = parent.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, &root).unwrap();
    let bytes = raw();
    let stager = EvidenceStager::new(&root);
    assert_eq!(
        stager.stage_fixed_json_export(request(&bytes)).unwrap_err(),
        StagerError::FilesystemSafety
    );
    assert!(!stager
        .cleanup_fixed_json("run-1", 1, FixedJsonNamespace::Raw)
        .unwrap()
        .is_complete());
    fs::remove_file(&root).unwrap();
    let aliased_parent = parent.join("alias");
    symlink(&outside, &aliased_parent).unwrap();
    assert!(EvidenceStager::new(aliased_parent.join("nested"))
        .stage_fixed_json_export(request(&bytes))
        .is_err());
    let handle = stager.stage_fixed_json_export(request(&bytes)).unwrap();
    let output_path = root.join("fixed-json/run-1/1/export/0.json");
    let output = fs::read(&output_path).unwrap();
    let outside_bytes = outside.join("outside.json");
    fs::write(&outside_bytes, &output).unwrap();
    fs::remove_file(&output_path).unwrap();
    symlink(&outside_bytes, &output_path).unwrap();
    assert!(stager.read_fixed_json_export(&handle).is_err());
    assert!(stager
        .fixed_json_status("run-1", 1, FixedJsonNamespace::Export)
        .is_err());
    assert!(!stager
        .cleanup_fixed_json("run-1", 1, FixedJsonNamespace::Export)
        .unwrap()
        .is_complete());
    assert_eq!(fs::read(&outside_bytes).unwrap(), output);
    fs::remove_file(&output_path).unwrap();
    fs::hard_link(&outside_bytes, &output_path).unwrap();
    assert!(stager.read_fixed_json_export(&handle).is_err());
    assert!(!stager
        .cleanup_fixed_json("run-1", 1, FixedJsonNamespace::Export)
        .unwrap()
        .is_complete());
    fs::remove_file(&output_path).unwrap();
    let unrelated = root.join("fixed-json/run-1/1/export/PRIVATE_VALUE_6146_CANARY");
    fs::write(&unrelated, b"keep").unwrap();
    let residual = stager
        .cleanup_fixed_json("run-1", 1, FixedJsonNamespace::Export)
        .unwrap();
    assert_eq!(
        residual.residuals()[0].reason(),
        CleanupResidualReason::UnrelatedEntry
    );
    assert!(!format!("{residual:?}").contains("PRIVATE_VALUE_6146_CANARY"));
    assert_eq!(fs::read(unrelated).unwrap(), b"keep");
    assert_eq!(fs::read(outside_bytes).unwrap(), output);
}

#[test]
fn temporary_crash_files_are_owned_for_cleanup_but_never_replayed() {
    let temp = TempDir::new().unwrap();
    let root = root(&temp);
    let stager = EvidenceStager::new(&root);
    write(
        &root.join("fixed-json/run-1/1/raw/.0.json.1.1.tmp"),
        b"PRIVATE_VALUE_6146_CANARY",
    );
    write(
        &root.join("fixed-json/run-1/1/export/.0.receipt.json.1.2.tmp"),
        b"incomplete",
    );
    assert!(stager.stage_fixed_json_export(request(&raw())).is_err());
    for namespace in [FixedJsonNamespace::Raw, FixedJsonNamespace::Export] {
        assert_eq!(
            stager
                .fixed_json_status("run-1", 1, namespace)
                .unwrap()
                .files,
            1
        );
        assert!(stager
            .cleanup_fixed_json("run-1", 1, namespace)
            .unwrap()
            .is_complete());
        assert_eq!(
            stager
                .fixed_json_status("run-1", 1, namespace)
                .unwrap()
                .files,
            0
        );
    }
}

#[test]
fn independent_stagers_replay_one_durable_receipt_under_concurrent_requests() {
    let temp = TempDir::new().unwrap();
    let root = root(&temp);
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let stager = EvidenceStager::new(root);
                let bytes = raw();
                barrier.wait();
                let handle = stager.stage_fixed_json_export(request(&bytes)).unwrap();
                canonical_bytes(stager.read_fixed_json_export(&handle).unwrap().receipt()).unwrap()
            })
        })
        .collect();
    let receipts: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(receipts[0], receipts[1]);
}
