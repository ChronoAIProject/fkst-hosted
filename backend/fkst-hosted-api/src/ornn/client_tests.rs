//! Unit tests for [`crate::ornn::client`] (issue #114). Split into its own
//! file (referenced via `#[path]`) so `client.rs` stays under 500 lines —
//! mirroring the `codex_provider/{mod,tests}.rs` split convention.

use std::sync::Mutex;

use super::*;

/// A scripted fake transport. Each `proxy_get` pops the next queued reply
/// matching by a path substring; `download_direct` returns a fixed blob and
/// records the URLs it was asked for (to assert the two-hop wiring).
struct FakeTransport {
    /// Queued `(path_contains, status, json_body)` replies, consumed FIFO.
    proxy: Mutex<Vec<(String, u16, serde_json::Value)>>,
    /// Bytes returned by every `download_direct` call.
    download_bytes: Vec<u8>,
    /// URLs `download_direct` was called with (in order).
    downloaded: Mutex<Vec<String>>,
}

impl FakeTransport {
    fn new(download_bytes: Vec<u8>) -> Self {
        Self {
            proxy: Mutex::new(Vec::new()),
            download_bytes,
            downloaded: Mutex::new(Vec::new()),
        }
    }

    fn push(&self, path_contains: &str, status: u16, body: serde_json::Value) {
        self.proxy
            .lock()
            .unwrap()
            .push((path_contains.to_string(), status, body));
    }
}

#[async_trait]
impl OrnnTransport for FakeTransport {
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
            .unwrap_or_else(|| panic!("no fake reply queued for path {path}"));
        let (_, status, body) = queue.remove(idx);
        Ok(ProxyResponse {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            headers: reqwest::header::HeaderMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }

    async fn download_direct(&self, presigned_url: &str) -> Result<Vec<u8>, AppError> {
        self.downloaded
            .lock()
            .unwrap()
            .push(presigned_url.to_string());
        Ok(self.download_bytes.clone())
    }
}

fn token() -> SecretString {
    SecretString::from("user_tok".to_string())
}

#[tokio::test]
async fn skill_detail_decodes_envelope_and_then_two_hop_downloads() {
    let fake = Arc::new(FakeTransport::new(b"ZIPBYTES".to_vec()));
    fake.push(
        "/skills/demo",
        200,
        serde_json::json!({
            "data": { "name": "demo", "version": "1.2",
                      "presignedPackageUrl": "https://storage/demo.zip?sig=x" }
        }),
    );
    let client = OrnnClient::new(fake.clone());

    let detail = client
        .skill_detail(&token(), "demo", "1.2")
        .await
        .expect("hop1");
    assert_eq!(
        detail.presigned_package_url,
        "https://storage/demo.zip?sig=x"
    );

    let bytes = client
        .download_package(&detail.presigned_package_url)
        .await
        .expect("hop2");
    assert_eq!(bytes, b"ZIPBYTES");
    // The download used the pre-signed URL directly (no proxy).
    assert_eq!(
        fake.downloaded.lock().unwrap().as_slice(),
        &["https://storage/demo.zip?sig=x".to_string()]
    );
}

#[tokio::test]
async fn skill_detail_404_maps_to_not_found() {
    let fake = Arc::new(FakeTransport::new(vec![]));
    fake.push("/skills/ghost", 404, serde_json::json!({}));
    let client = OrnnClient::new(fake);
    let err = client
        .skill_detail(&token(), "ghost", "1.0")
        .await
        .expect_err("404");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn skill_detail_403_maps_to_forbidden() {
    let fake = Arc::new(FakeTransport::new(vec![]));
    fake.push("/skills/private", 403, serde_json::json!({}));
    let client = OrnnClient::new(fake);
    let err = client
        .skill_detail(&token(), "private", "1.0")
        .await
        .expect_err("403");
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
}

#[tokio::test]
async fn resolve_pins_expands_a_skillset_closure_and_collects_instructions() {
    let fake = Arc::new(FakeTransport::new(vec![]));
    fake.push(
        "/skillsets/research/closure",
        200,
        serde_json::json!({
            "data": {
                "instructions": "Master prompt.",
                "items": [
                    { "name": "web", "version": "1.0" },
                    { "name": "pdf", "version": "2.0" }
                ]
            }
        }),
    );
    let client = OrnnClient::new(fake);
    let pins = vec![OrnnSkillPin {
        kind: OrnnPinKind::Skillset,
        name: "research".to_string(),
        version: "3.1".to_string(),
    }];
    let resolved = client.resolve_pins(&token(), &pins).await.expect("resolve");
    assert_eq!(resolved.nodes.len(), 2);
    assert_eq!(resolved.nodes[0].name, "web");
    assert_eq!(resolved.skillset_instructions.len(), 1);
    assert_eq!(resolved.skillset_instructions[0].0, "research");
    assert_eq!(resolved.skillset_instructions[0].1, "Master prompt.");
}

#[tokio::test]
async fn resolve_pins_dedupes_same_name_same_version_across_pins() {
    let fake = Arc::new(FakeTransport::new(vec![]));
    // A skillset whose member overlaps a separately-pinned skill at the
    // SAME version: should dedupe to a single node.
    fake.push(
        "/skillsets/bundle/closure",
        200,
        serde_json::json!({
            "data": { "instructions": "", "items": [ { "name": "shared", "version": "1.0" } ] }
        }),
    );
    let client = OrnnClient::new(fake);
    let pins = vec![
        OrnnSkillPin {
            kind: OrnnPinKind::Skillset,
            name: "bundle".to_string(),
            version: "1.0".to_string(),
        },
        OrnnSkillPin {
            kind: OrnnPinKind::Skill,
            name: "shared".to_string(),
            version: "1.0".to_string(),
        },
    ];
    let resolved = client.resolve_pins(&token(), &pins).await.expect("resolve");
    assert_eq!(resolved.nodes.len(), 1, "same name+version must dedupe");
}

#[tokio::test]
async fn resolve_pins_hard_fails_on_cross_selection_version_conflict() {
    let fake = Arc::new(FakeTransport::new(vec![]));
    fake.push(
        "/skillsets/bundle/closure",
        200,
        serde_json::json!({
            "data": { "instructions": "", "items": [ { "name": "shared", "version": "1.0" } ] }
        }),
    );
    let client = OrnnClient::new(fake);
    let pins = vec![
        OrnnSkillPin {
            kind: OrnnPinKind::Skillset,
            name: "bundle".to_string(),
            version: "1.0".to_string(),
        },
        // Same skill name, DIFFERENT version → hard conflict.
        OrnnSkillPin {
            kind: OrnnPinKind::Skill,
            name: "shared".to_string(),
            version: "2.0".to_string(),
        },
    ];
    let err = client
        .resolve_pins(&token(), &pins)
        .await
        .expect_err("conflict");
    assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    assert!(format!("{err}").contains("conflicting"));
}

#[tokio::test]
async fn resolve_pins_propagates_a_closure_404_loudly() {
    let fake = Arc::new(FakeTransport::new(vec![]));
    fake.push("/skillsets/ghost/closure", 404, serde_json::json!({}));
    let client = OrnnClient::new(fake);
    let pins = vec![OrnnSkillPin {
        kind: OrnnPinKind::Skillset,
        name: "ghost".to_string(),
        version: "1.0".to_string(),
    }];
    let err = client.resolve_pins(&token(), &pins).await.expect_err("404");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn versions_unwraps_items_array() {
    let fake = Arc::new(FakeTransport::new(vec![]));
    fake.push(
        "/skills/fmt/versions",
        200,
        serde_json::json!({
            "data": { "items": [ { "version": "2.0" }, { "version": "1.0" } ] }
        }),
    );
    let client = OrnnClient::new(fake);
    let versions = client
        .skill_versions(&token(), "fmt")
        .await
        .expect("versions");
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version, "2.0");
}

#[tokio::test]
async fn skill_search_decodes_page() {
    let fake = Arc::new(FakeTransport::new(vec![]));
    fake.push(
        "/skill-search",
        200,
        serde_json::json!({
            "data": {
                "items": [ { "name": "fmt", "isSystemSkill": true } ],
                "page": 1, "pageSize": 20, "total": 1
            }
        }),
    );
    let client = OrnnClient::new(fake);
    let page = client
        .skill_search(&token(), &[("scope", "public")])
        .await
        .expect("search");
    assert_eq!(page.items.len(), 1);
    assert!(page.items[0].is_system_skill);
    assert_eq!(page.total, 1);
}
