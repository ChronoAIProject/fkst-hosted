//! Shared test fixtures for the journaler tests scattered across `mod.rs`,
//! `flush_mech_tests.rs`, and `flush_bootstrap_tests.rs`. A `cfg(test)`
//! sibling module so all three test files share one fixture set (#139).

use secrecy::SecretString;
use serde_json::json;

use crate::journal::config::JournalConfig;
use crate::journal::model::{CompletedEntry, ProgressRecord};
use crate::journal::{ProgressSignal, SessionCtx};

pub(crate) fn ctx(token: i64) -> SessionCtx {
    SessionCtx {
        session_id: "11111111-1111-4111-8111-111111111111".to_string(),
        package_name: "demo".to_string(),
        package_fingerprint: "fp".to_string(),
        pod_id: Some("pod-0".to_string()),
        fencing_token: token,
    }
}

pub(crate) fn github_cfg(server_uri: &str) -> JournalConfig {
    JournalConfig {
        github_repo: Some("owner/name".to_string()),
        github_api_base: server_uri.to_string(),
        github_token: Some(SecretString::from("test-token".to_string())),
        cas_max_retries: 3,
        // Bootstrap retries default to 3; tests that exercise the 404
        // eventual-consistency loop override this explicitly.
        ..JournalConfig::default()
    }
}

pub(crate) fn mongo_only_cfg() -> JournalConfig {
    JournalConfig {
        github_enabled: false,
        ..JournalConfig::default()
    }
}

pub(crate) fn raised(department: &str, name: &str) -> ProgressSignal {
    ProgressSignal::Raised {
        event_json: json!({
            "department": department, "source": "raiser", "name": name, "corr": "c-1"
        }),
    }
}

pub(crate) fn completed(idem: &str, at: &str) -> CompletedEntry {
    CompletedEntry {
        idem_key: idem.to_string(),
        event: json!({"department": "d"}),
        at: at.to_string(),
    }
}

/// Contents-API GET body for a record.
pub(crate) fn contents_body(record: &ProgressRecord, sha: &str) -> serde_json::Value {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    json!({
        "content": STANDARD.encode(serde_json::to_vec(record).expect("json")),
        "sha": sha,
        "encoding": "base64"
    })
}
