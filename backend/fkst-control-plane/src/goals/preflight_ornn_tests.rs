//! Unit tests for [`crate::goals::preflight_ornn`] (#179). Split into its own
//! file (referenced via `#[path]`) so `preflight_ornn.rs` stays under 500 lines
//! and the Ornn-availability cases live next to the code they exercise — mirroring
//! the `ornn/client_tests.rs` split convention.

use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use secrecy::SecretString;

use super::check_ornn;
use crate::error::AppError;
use crate::nyxid::ProxyResponse;
use crate::ornn::types::{OrnnPinKind, OrnnSkillPin};
use crate::ornn::{OrnnClient, OrnnTransport};

/// Scripted Ornn transport: FIFO replies keyed by a path substring (mirrors the
/// `ornn` module fakes).
struct FakeOrnn {
    proxy: Mutex<Vec<(String, u16, serde_json::Value)>>,
}

impl FakeOrnn {
    fn new() -> Self {
        Self {
            proxy: Mutex::new(Vec::new()),
        }
    }

    fn push(&self, needle: &str, status: u16, body: serde_json::Value) {
        self.proxy
            .lock()
            .unwrap()
            .push((needle.to_string(), status, body));
    }
}

#[async_trait]
impl OrnnTransport for FakeOrnn {
    async fn proxy_get(
        &self,
        path: &str,
        _query: &[(&str, &str)],
        _user_token: &SecretString,
    ) -> Result<ProxyResponse, AppError> {
        let mut queue = self.proxy.lock().unwrap();
        let idx = queue
            .iter()
            .position(|(needle, _, _)| path.contains(needle.as_str()))
            .unwrap_or_else(|| panic!("no fake ornn reply for {path}"));
        let (_, status, body) = queue.remove(idx);
        Ok(ProxyResponse {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            headers: reqwest::header::HeaderMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }

    async fn download_direct(&self, _url: &str) -> Result<Vec<u8>, AppError> {
        unreachable!("preflight never downloads packages")
    }
}

fn skill(name: &str, version: &str) -> OrnnSkillPin {
    OrnnSkillPin {
        kind: OrnnPinKind::Skill,
        name: name.to_string(),
        version: version.to_string(),
    }
}

fn skillset(name: &str, version: &str) -> OrnnSkillPin {
    OrnnSkillPin {
        kind: OrnnPinKind::Skillset,
        name: name.to_string(),
        version: version.to_string(),
    }
}

fn token() -> SecretString {
    SecretString::from("user_tok".to_string())
}

fn ornn_client(fake: FakeOrnn) -> OrnnClient {
    OrnnClient::new(Arc::new(fake))
}

#[tokio::test]
async fn ornn_unknown_skill_is_a_pin_error() {
    let fake = FakeOrnn::new();
    fake.push("/skills/ghost/versions", 404, serde_json::json!({}));
    let client = ornn_client(fake);
    let errors = check_ornn(&client, Some(&token()), &[skill("ghost", "1.0")]).await;
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].name, "ghost");
    assert!(errors[0].reason.contains("not found"));
}

#[tokio::test]
async fn ornn_known_skill_unknown_version_is_a_pin_error() {
    let fake = FakeOrnn::new();
    fake.push(
        "/skills/fmt/versions",
        200,
        serde_json::json!({ "data": { "items": [ { "version": "2.0" } ] } }),
    );
    let client = ornn_client(fake);
    let errors = check_ornn(&client, Some(&token()), &[skill("fmt", "1.0")]).await;
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].reason.contains("not available"),
        "{:?}",
        errors[0]
    );
}

#[tokio::test]
async fn ornn_deprecated_only_version_is_a_pin_error() {
    let fake = FakeOrnn::new();
    fake.push(
        "/skills/fmt/versions",
        200,
        serde_json::json!({ "data": { "items": [ { "version": "1.0", "isDeprecated": true } ] } }),
    );
    let client = ornn_client(fake);
    let errors = check_ornn(&client, Some(&token()), &[skill("fmt", "1.0")]).await;
    assert_eq!(errors.len(), 1);
    assert!(errors[0].reason.contains("deprecated"), "{:?}", errors[0]);
}

#[tokio::test]
async fn ornn_closure_conflict_is_a_pin_error() {
    // Both pins exist at their requested versions, but the skillset closure pulls
    // `shared@1.0` while the skill pin asks `shared@2.0` → resolve conflict.
    let fake = FakeOrnn::new();
    fake.push(
        "/skillsets/bundle/versions",
        200,
        serde_json::json!({ "data": { "items": [ { "version": "1.0" } ] } }),
    );
    fake.push(
        "/skills/shared/versions",
        200,
        serde_json::json!({ "data": { "items": [ { "version": "2.0" } ] } }),
    );
    fake.push(
        "/skillsets/bundle/closure",
        200,
        serde_json::json!({ "data": { "instructions": "",
            "items": [ { "name": "shared", "version": "1.0" } ] } }),
    );
    let client = ornn_client(fake);
    let pins = vec![skillset("bundle", "1.0"), skill("shared", "2.0")];
    let errors = check_ornn(&client, Some(&token()), &pins).await;
    assert!(
        !errors.is_empty(),
        "a closure conflict must produce pin errors"
    );
    assert!(
        errors.iter().any(|e| e.reason.contains("conflicting")),
        "conflict reason must name the clash: {errors:?}"
    );
}

#[tokio::test]
async fn ornn_all_resolvable_passes() {
    let fake = FakeOrnn::new();
    fake.push(
        "/skills/fmt/versions",
        200,
        serde_json::json!({ "data": { "items": [ { "version": "2.0" } ] } }),
    );
    let client = ornn_client(fake);
    let errors = check_ornn(&client, Some(&token()), &[skill("fmt", "2.0")]).await;
    assert!(errors.is_empty(), "all-resolvable must pass: {errors:?}");
}

#[tokio::test]
async fn ornn_empty_pins_is_a_noop() {
    let client = ornn_client(FakeOrnn::new());
    let errors = check_ornn(&client, Some(&token()), &[]).await;
    assert!(errors.is_empty());
}

#[tokio::test]
async fn ornn_without_token_reports_every_pin() {
    let client = ornn_client(FakeOrnn::new());
    let errors = check_ornn(&client, None, &[skill("a", "1.0"), skill("b", "2.0")]).await;
    assert_eq!(errors.len(), 2, "every pin is unverifiable without a token");
}
