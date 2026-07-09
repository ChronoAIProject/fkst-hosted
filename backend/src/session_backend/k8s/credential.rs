//! The credential-delivery verb (issue #413): rewrite one file key of a live
//! session's mounted Secret in place. Generalized from the token-rotation loop's
//! `github-token`-specific patch to ANY file key; the whole-volume projection
//! propagates the new file to the running container. The value is never logged.

use kube::api::{Patch, PatchParams};
use secrecy::{ExposeSecret, SecretString};

use crate::k8s::session_object_name;

use super::super::{BackendError, DeliveryOutcome};
use super::K8sBackend;

impl K8sBackend {
    pub(super) async fn deliver_credential_impl(
        &self,
        session_id: &str,
        file: &str,
        contents: SecretString,
    ) -> Result<DeliveryOutcome, BackendError> {
        let name = session_object_name(session_id);
        let patch = secret_file_patch(file, contents.expose_secret());
        match self
            .secrets_api()
            .patch(&name, &PatchParams::default(), &Patch::Merge(patch))
            .await
        {
            Ok(_) => Ok(DeliveryOutcome::Delivered),
            Err(kube::Error::Api(e)) if e.code == 404 => Ok(DeliveryOutcome::SessionGone),
            Err(error) => Err(BackendError::Other(anyhow::Error::new(error))),
        }
    }
}

/// The JSON merge patch that rewrites a session Secret's `file` key with `contents`
/// (via `stringData`, which K8s folds into the Secret's data). Pure + unit-tested so
/// the key + shape can't drift. A merge patch leaves every other key/field intact.
fn secret_file_patch(file: &str, contents: &str) -> serde_json::Value {
    let string_data = serde_json::Map::from_iter([(
        file.to_string(),
        serde_json::Value::String(contents.to_string()),
    )]);
    serde_json::json!({ "stringData": serde_json::Value::Object(string_data) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_spec::creds::GITHUB_TOKEN_FILE;

    #[test]
    fn secret_patch_targets_the_github_token_key_via_string_data() {
        let patch = secret_file_patch(
            GITHUB_TOKEN_FILE,
            r#"{"token":"ghs_new","expires_at":"2026-07-01T13:00:00+00:00"}"#,
        );
        let value = &patch["stringData"][GITHUB_TOKEN_FILE];
        assert_eq!(
            value.as_str().unwrap(),
            r#"{"token":"ghs_new","expires_at":"2026-07-01T13:00:00+00:00"}"#
        );
        // Nothing else is touched (a merge patch leaves other keys/fields intact).
        assert!(patch.get("data").is_none());
        assert_eq!(GITHUB_TOKEN_FILE, "github-token");
    }

    #[test]
    fn secret_patch_generalizes_to_any_file_key() {
        // The verb delivers ANY credential file, not just the github token.
        let patch = secret_file_patch("llm-api-key", "sk-rotated");
        assert_eq!(
            patch["stringData"]["llm-api-key"].as_str().unwrap(),
            "sk-rotated"
        );
        assert!(patch["stringData"].get("github-token").is_none());
    }
}
