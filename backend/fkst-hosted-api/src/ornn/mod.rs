//! Ornn skill-registry integration (issue #114).
//!
//! Lets a user pin Ornn **skills / skillsets** (name + concrete version) at
//! session trigger; fkst-hosted fetches the full package(s) as that user — via
//! the session's NyxID token (#111) through the NyxID proxy slug `ornn-api` —
//! and writes them into the per-session `$CODEX_HOME` (#112) so the session's
//! `codex` loads them. Also backs the "list available skills" catalog API.
//!
//! No fkst-substrate engine change: Ornn enforces ALL permission/visibility,
//! and the injected files land in the existing per-session CODEX_HOME.
//!
//! Layering:
//! - [`types`] — the Ornn DTOs + the user-facing [`OrnnSkillPin`] types.
//! - [`client`] — [`OrnnClient`]: typed registry access over an INJECTED
//!   transport (testable with a fake), plus pin resolution / conflict logic.
//! - [`inject`] — the on-disk seam: unzip into `$CODEX_HOME/skills/<name>/`
//!   (exec bits preserved, path-traversal rejected) and append a skillset's
//!   instructions to `$CODEX_HOME/AGENTS.md`.
//! - This module's [`inject_pins`] orchestrates the two-hop fetch + install for
//!   every resolved node and the AGENTS.md append, for the session driver.

pub mod client;
pub mod inject;
pub mod types;

use std::path::Path;

use secrecy::SecretString;

use crate::error::AppError;

pub use client::{ConflictError, OrnnClient, OrnnTransport, ResolvedPins, DEFAULT_ORNN_SLUG};
pub use types::{OrnnPinKind, OrnnSkillPin, ResolvedNode};

/// Resolve `pins`, then for every leaf skill run the TWO-hop fetch
/// (`skill_detail` → `download_package`) and install it into
/// `<codex_home>/skills/<name>/`, and append each skillset's instructions to
/// `<codex_home>/AGENTS.md`.
///
/// `user_token` is the session's NyxID token (#111) — exposed only to the proxy
/// calls, NEVER logged. Any Ornn `404`/`403` (missing or forbidden pin) and any
/// download/unzip failure propagates as an `AppError`, so the session driver
/// aborts the start loudly rather than running with a partially-applied pin set.
///
/// A no-op (`Ok`) when `pins` is empty, so a session with no pins is unchanged.
pub async fn inject_pins(
    client: &OrnnClient,
    user_token: &SecretString,
    codex_home: &Path,
    pins: &[OrnnSkillPin],
) -> Result<(), AppError> {
    if pins.is_empty() {
        return Ok(());
    }

    let resolved = client.resolve_pins(user_token, pins).await?;
    tracing::info!(
        node_count = resolved.nodes.len(),
        skillset_count = resolved.skillset_instructions.len(),
        "resolved ornn pins for session"
    );

    for node in &resolved.nodes {
        // Hop 1 (proxied): fetch the pre-signed package URL as the user.
        let detail = client
            .skill_detail(user_token, &node.name, &node.version)
            .await?;
        // Hop 2 (DIRECT, no auth): download the verbatim zip. The URL is
        // sensitive and is never logged.
        let zip_bytes = client
            .download_package(&detail.presigned_package_url)
            .await?;
        inject::install_skill(codex_home, &node.name, &zip_bytes)?;
    }

    for (skillset_name, instructions) in &resolved.skillset_instructions {
        inject::append_instructions(codex_home, skillset_name, instructions)?;
    }

    Ok(())
}

/// Validate one pin at the trust boundary (trigger request): `name` matches
/// `^[a-z0-9][a-z0-9-]*$` and is ≤ [`types::MAX_PIN_NAME_BYTES`]; `version`
/// matches `^(0|[1-9]\d*)\.(0|[1-9]\d*)$` (the `<major>.<minor>` grammar Ornn
/// enforces — no leading zeros, no patch component, no `@latest`).
pub fn validate_pin(pin: &OrnnSkillPin) -> Result<(), AppError> {
    if pin.name.is_empty() || pin.name.len() > types::MAX_PIN_NAME_BYTES {
        return Err(AppError::Validation(format!(
            "invalid ornn pin name: must be 1..={} bytes",
            types::MAX_PIN_NAME_BYTES
        )));
    }
    if !is_valid_pin_name(&pin.name) {
        return Err(AppError::Validation(format!(
            "invalid ornn pin name {:?}: must match ^[a-z0-9][a-z0-9-]*$",
            pin.name
        )));
    }
    if !is_valid_pin_version(&pin.version) {
        return Err(AppError::Validation(format!(
            "invalid ornn pin version {:?}: must match <major>.<minor> (no leading zeros)",
            pin.version
        )));
    }
    Ok(())
}

/// Validate a whole pin selection: each pin individually, then the
/// cross-selection version conflict (the same pre-trigger check the picker UI
/// must run). For each `name` that appears as a direct SKILL pin twice with
/// different versions this fails fast; skillset closures are checked again at
/// resolve time (which needs Ornn), so this catches the cheap local case.
pub fn validate_pins(pins: &[OrnnSkillPin]) -> Result<(), AppError> {
    use std::collections::HashMap;
    let mut chosen: HashMap<&str, &str> = HashMap::new();
    for pin in pins {
        validate_pin(pin)?;
        if pin.kind == OrnnPinKind::Skill {
            if let Some(existing) = chosen.get(pin.name.as_str()) {
                if *existing != pin.version.as_str() {
                    return Err(AppError::Unprocessable(format!(
                        "conflicting ornn skill versions for {:?}: {} vs {}",
                        pin.name, existing, pin.version
                    )));
                }
            } else {
                chosen.insert(&pin.name, &pin.version);
            }
        }
    }
    Ok(())
}

/// `^[a-z0-9][a-z0-9-]*$` without pulling in a compiled regex (the rule is a
/// fixed ASCII allow-list, cheaper and clearer hand-rolled here).
fn is_valid_pin_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `^(0|[1-9]\d*)\.(0|[1-9]\d*)$`: exactly two dot-separated components, each a
/// non-negative integer with no leading zeros (`0` itself is allowed).
fn is_valid_pin_version(version: &str) -> bool {
    let mut parts = version.split('.');
    let (Some(major), Some(minor), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    is_valid_version_component(major) && is_valid_version_component(minor)
}

fn is_valid_version_component(component: &str) -> bool {
    match component {
        "" => false,
        "0" => true,
        other => other.bytes().all(|b| b.is_ascii_digit()) && !other.starts_with('0'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::nyxid::ProxyResponse;
    use async_trait::async_trait;

    fn pin(kind: OrnnPinKind, name: &str, version: &str) -> OrnnSkillPin {
        OrnnSkillPin {
            kind,
            name: name.to_string(),
            version: version.to_string(),
        }
    }

    // ---- validation ---------------------------------------------------------

    #[test]
    fn validate_pin_accepts_well_formed_pins() {
        for (name, version) in [("web-research", "1.0"), ("a", "0.0"), ("x9", "12.34")] {
            validate_pin(&pin(OrnnPinKind::Skill, name, version)).expect(name);
        }
    }

    #[test]
    fn validate_pin_rejects_bad_names() {
        for name in [
            "",
            "-leading",
            "Upper",
            "has_underscore",
            "has space",
            "dot.name",
        ] {
            assert!(
                validate_pin(&pin(OrnnPinKind::Skill, name, "1.0")).is_err(),
                "must reject name {name:?}"
            );
        }
    }

    #[test]
    fn validate_pin_rejects_overlong_names() {
        let long = "a".repeat(types::MAX_PIN_NAME_BYTES + 1);
        assert!(validate_pin(&pin(OrnnPinKind::Skill, &long, "1.0")).is_err());
    }

    #[test]
    fn validate_pin_rejects_bad_versions() {
        for version in ["1", "1.0.0", "01.0", "1.01", "1.", ".1", "v1.0", "1.x", ""] {
            assert!(
                validate_pin(&pin(OrnnPinKind::Skill, "ok", version)).is_err(),
                "must reject version {version:?}"
            );
        }
    }

    #[test]
    fn validate_pins_rejects_duplicate_skill_with_conflicting_versions() {
        let pins = vec![
            pin(OrnnPinKind::Skill, "shared", "1.0"),
            pin(OrnnPinKind::Skill, "shared", "2.0"),
        ];
        let err = validate_pins(&pins).expect_err("conflict");
        assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    }

    #[test]
    fn validate_pins_allows_duplicate_skill_with_same_version() {
        let pins = vec![
            pin(OrnnPinKind::Skill, "shared", "1.0"),
            pin(OrnnPinKind::Skill, "shared", "1.0"),
        ];
        validate_pins(&pins).expect("same version is fine");
    }

    // ---- inject_pins end-to-end (fake transport) ----------------------------

    use std::io::Write;
    use std::sync::Mutex;

    struct FakeTransport {
        proxy: Mutex<Vec<(String, u16, serde_json::Value)>>,
        zip: Vec<u8>,
    }

    impl FakeTransport {
        fn new(zip: Vec<u8>) -> Self {
            Self {
                proxy: Mutex::new(Vec::new()),
                zip,
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
                .unwrap_or_else(|| panic!("no fake reply for {path}"));
            let (_, status, body) = queue.remove(idx);
            Ok(ProxyResponse {
                status: reqwest::StatusCode::from_u16(status).unwrap(),
                headers: reqwest::header::HeaderMap::new(),
                body: serde_json::to_vec(&body).unwrap(),
            })
        }
        async fn download_direct(&self, _url: &str) -> Result<Vec<u8>, AppError> {
            Ok(self.zip.clone())
        }
    }

    fn one_file_zip() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut writer = zip::ZipWriter::new(cursor);
            let opts: zip::write::FileOptions<()> =
                zip::write::FileOptions::default().unix_permissions(0o644);
            writer.start_file("SKILL.md", opts).unwrap();
            writer.write_all(b"# Demo").unwrap();
            writer.finish().unwrap();
        }
        buf
    }

    #[tokio::test]
    async fn inject_pins_installs_a_skill_and_a_skillset_with_instructions() {
        let fake = Arc::new(FakeTransport::new(one_file_zip()));
        // Skillset closure → one member skill + instructions.
        fake.push(
            "/skillsets/research/closure",
            200,
            serde_json::json!({
                "data": { "instructions": "Master prompt.",
                          "items": [ { "name": "web", "version": "1.0" } ] }
            }),
        );
        // Direct skill pin.
        // skill_detail for the closure member `web` and the direct pin `fmt`.
        fake.push(
            "/skills/web",
            200,
            serde_json::json!({ "data": { "name": "web", "version": "1.0",
                "presignedPackageUrl": "https://storage/web.zip" } }),
        );
        fake.push(
            "/skills/fmt",
            200,
            serde_json::json!({ "data": { "name": "fmt", "version": "2.0",
                "presignedPackageUrl": "https://storage/fmt.zip" } }),
        );

        let client = OrnnClient::new(fake);
        let home = tempfile::tempdir().unwrap();
        let pins = vec![
            pin(OrnnPinKind::Skillset, "research", "3.0"),
            pin(OrnnPinKind::Skill, "fmt", "2.0"),
        ];
        let token = SecretString::from("tok".to_string());
        inject_pins(&client, &token, home.path(), &pins)
            .await
            .expect("inject");

        // Both skills installed.
        assert!(home.path().join("skills/web/SKILL.md").is_file());
        assert!(home.path().join("skills/fmt/SKILL.md").is_file());
        // The skillset instructions landed in AGENTS.md.
        let agents = std::fs::read_to_string(home.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("ornn-skillset:research BEGIN"));
        assert!(agents.contains("Master prompt."));
    }

    #[tokio::test]
    async fn inject_pins_is_a_noop_for_empty_pins() {
        let fake = Arc::new(FakeTransport::new(vec![]));
        let client = OrnnClient::new(fake);
        let home = tempfile::tempdir().unwrap();
        let token = SecretString::from("tok".to_string());
        inject_pins(&client, &token, home.path(), &[])
            .await
            .expect("noop");
        // No skills dir, no AGENTS.md created.
        assert!(!home.path().join("skills").exists());
        assert!(!home.path().join("AGENTS.md").exists());
    }

    #[tokio::test]
    async fn inject_pins_aborts_loudly_on_a_404_pin() {
        let fake = Arc::new(FakeTransport::new(one_file_zip()));
        fake.push("/skills/ghost", 404, serde_json::json!({}));
        let client = OrnnClient::new(fake);
        let home = tempfile::tempdir().unwrap();
        let pins = vec![pin(OrnnPinKind::Skill, "ghost", "1.0")];
        let token = SecretString::from("tok".to_string());
        let err = inject_pins(&client, &token, home.path(), &pins)
            .await
            .expect_err("404 aborts");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }
}
