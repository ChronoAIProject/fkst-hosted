//! Authoritative BSON document shapes for the `sessions` and `leases`
//! collections. (The `packages` collection shape is owned by
//! [`crate::packages`].)
//!
//! Conventions (load-bearing for downstream queries):
//! - `Option<T>` fields serialize as explicit BSON `null` (no
//!   `skip_serializing_if`) so the document shape is stable.
//! - UUIDs are stored as `bson::Uuid` (BSON Binary subtype 4) — a raw
//!   `uuid::Uuid` would serialize as a *string* and silently never match
//!   `find_one({_id})` lookups. Convert to/from `uuid::Uuid` at the edges.
//! - Timestamps are `bson::DateTime` (millisecond UTC, driver-native) so
//!   round-trips are lossless.

use serde::{Deserialize, Serialize};

/// Lifecycle state of a session. Serializes lowercase on the wire.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Pending,
    Validating,
    Running,
    Stopping,
    Stopped,
    Failed,
}

/// `sessions` collection: `_id` is a UUID stored as BSON Binary subtype 4.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDoc {
    #[serde(rename = "_id")]
    pub id: bson::Uuid,
    pub package_name: String,
    pub status: SessionStatus,
    pub pod_id: Option<String>,
    pub fencing_token: Option<i64>,
    pub pid: Option<i32>,
    pub runtime_dir: Option<String>,
    pub error: Option<String>,
    pub created_at: bson::DateTime,
    pub started_at: Option<bson::DateTime>,
    pub stopped_at: Option<bson::DateTime>,
}

/// `leases` collection: `_id` is the package name (at most one live holder
/// per package).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeaseDoc {
    #[serde(rename = "_id")]
    pub package_name: String,
    pub session_id: bson::Uuid,
    pub holder_pod: String,
    pub fencing_token: i64,
    pub expires_at: bson::DateTime,
    pub renewed_at: bson::DateTime,
}

#[cfg(test)]
mod tests {
    use bson::spec::BinarySubtype;
    use bson::Bson;

    use super::*;

    fn sample_session() -> SessionDoc {
        SessionDoc {
            id: bson::Uuid::new(),
            package_name: "demo-package".to_string(),
            status: SessionStatus::Running,
            pod_id: Some("pod-0".to_string()),
            fencing_token: Some(42),
            pid: Some(4242),
            runtime_dir: Some("/tmp/run".to_string()),
            error: None,
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            started_at: Some(bson::DateTime::from_millis(1_700_000_000_500)),
            stopped_at: None,
        }
    }

    fn sample_lease() -> LeaseDoc {
        LeaseDoc {
            package_name: "demo-package".to_string(),
            session_id: bson::Uuid::new(),
            holder_pod: "pod-0".to_string(),
            fencing_token: 7,
            expires_at: bson::DateTime::from_millis(1_700_000_060_000),
            renewed_at: bson::DateTime::from_millis(1_700_000_030_000),
        }
    }

    #[test]
    fn session_doc_round_trips_losslessly() {
        let doc = sample_session();
        let raw = bson::to_document(&doc).expect("serialize");
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn lease_doc_round_trips_losslessly() {
        let doc = sample_lease();
        let raw = bson::to_document(&doc).expect("serialize");
        let back: LeaseDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn session_id_serializes_as_binary_subtype_uuid_not_string() {
        // Regression guard: a string `_id` would silently never match
        // `find_one({_id: <uuid>})` against driver-written Binary data.
        let raw = bson::to_document(&sample_session()).expect("serialize");
        match raw.get("_id").expect("_id present") {
            Bson::Binary(binary) => assert_eq!(binary.subtype, BinarySubtype::Uuid),
            other => panic!("expected Bson::Binary(subtype Uuid), got {other:?}"),
        }
    }

    #[test]
    fn session_status_serializes_lowercase() {
        let cases = [
            (SessionStatus::Pending, "pending"),
            (SessionStatus::Validating, "validating"),
            (SessionStatus::Running, "running"),
            (SessionStatus::Stopping, "stopping"),
            (SessionStatus::Stopped, "stopped"),
            (SessionStatus::Failed, "failed"),
        ];
        for (status, expected) in cases {
            let bson = bson::to_bson(&status).expect("serialize");
            assert_eq!(bson, Bson::String(expected.to_string()));
        }
    }

    #[test]
    fn lease_doc_id_carries_the_package_name() {
        let raw = bson::to_document(&sample_lease()).expect("serialize");
        assert_eq!(
            raw.get("_id").expect("_id present"),
            &Bson::String("demo-package".to_string())
        );
        assert!(
            !raw.contains_key("package_name"),
            "package_name must map onto _id only"
        );
    }

    #[test]
    fn none_fields_serialize_as_explicit_null() {
        let raw = bson::to_document(&sample_session()).expect("serialize");
        assert_eq!(raw.get("error").expect("error present"), &Bson::Null);
        assert_eq!(
            raw.get("stopped_at").expect("stopped_at present"),
            &Bson::Null
        );
    }
}
