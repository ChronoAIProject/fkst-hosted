//! Pure builders for the env-validation Pod, its spec ConfigMap, and the pod
//! name (issue #338 §3.1/§3.3). Split out of [`super`] so the orchestration
//! (which needs a live cluster) and the spec-shaping (which does not) stay in
//! separate files, each testable in isolation and under the 500-line limit.
//!
//! The validation pod is the SAME hard-isolation box as a session pod: it runs
//! [`crate::k8s::isolation::apply_isolation`], so a compromised install script
//! is boxed inside a throwaway pod with no API token, no service discovery,
//! external DNS only, and host namespaces off (see the isolation module docs).

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    ConfigMap, ConfigMapVolumeSource, Container, Pod, PodSpec, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};

use crate::config::PodConfig;
use crate::error::AppError;
use crate::install::ValidateSpec;

/// Name prefix shared by the validation Pod + its spec ConfigMap.
const POD_NAME_PREFIX: &str = "fkst-env-val-";
/// DNS-1123 label ceiling: Pod (and ConfigMap) names must be <= 63 chars.
const POD_NAME_MAX_LEN: usize = 63;
/// Length of the random collision-breaking suffix appended to the pod name.
const RANDOM_SUFFIX_LEN: usize = 8;

/// `app.kubernetes.io/component` value stamped on every validation object. The
/// GC sweep + any operator query select on exactly this.
pub(crate) const COMPONENT_LABEL_VALUE: &str = "env-validation";

/// Directory the spec ConfigMap is mounted at (read-only). Concatenated with
/// [`SPEC_FILE_KEY`] it MUST equal [`crate::install::VALIDATE_SPEC_PATH`] — a
/// drift guard in the tests enforces that.
const SPEC_MOUNT_DIR: &str = "/var/run/fkst/validate";
/// ConfigMap data key (and mounted filename) carrying the serialized spec.
const SPEC_FILE_KEY: &str = "validate-spec.json";
/// Volume name shared by the ConfigMap volume + its mount.
const SPEC_VOLUME: &str = "validate-spec";
/// Container name inside the validation pod.
const VALIDATOR_CONTAINER: &str = "validator";

/// Common labels stamped on the validation Pod + its ConfigMap so a sweep (or an
/// operator) can find them by selector and attribute them to a GitHub user.
fn validation_labels(id: i64) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "app.kubernetes.io/part-of".to_string(),
            "fkst-hosted".to_string(),
        ),
        (
            "app.kubernetes.io/component".to_string(),
            COMPONENT_LABEL_VALUE.to_string(),
        ),
        (
            "fkst.chrono-ai.fun/github-user-id".to_string(),
            id.to_string(),
        ),
    ])
}

/// A random lowercase-alphanumeric suffix (`a-z0-9`) that breaks name collisions
/// between concurrent validations of the same environment.
fn random_suffix() -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..RANDOM_SUFFIX_LEN)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect()
}

/// Fold an arbitrary environment name into DNS-1123 label characters: ASCII
/// alphanumerics are lowercased, everything else becomes `-`. Leading/trailing
/// dashes are trimmed by the caller after truncation.
fn sanitize_label_segment(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Deterministic name from `(id, name, suffix)`, truncating the `name` segment so
/// the whole label stays a valid DNS-1123 name (<= 63 chars). Split from
/// [`validation_pod_name`] so tests can pin a known suffix.
fn validation_object_name(id: i64, name: &str, suffix: &str) -> String {
    let id_str = id.to_string();
    let sanitized = sanitize_label_segment(name);
    // Fixed cost = prefix + id + the two `-` separators + suffix. Whatever is
    // left is the budget for the (truncated) name segment.
    let fixed = POD_NAME_PREFIX.len() + id_str.len() + 2 + suffix.len();
    let budget = POD_NAME_MAX_LEN.saturating_sub(fixed);
    let name_seg: String = sanitized.chars().take(budget).collect();
    let name_seg = name_seg.trim_matches('-');
    if name_seg.is_empty() {
        // No usable name characters survived truncation: drop the segment (and
        // its extra `-`) rather than emit a `--` — the suffix keeps it unique.
        format!("{POD_NAME_PREFIX}{id_str}-{suffix}")
    } else {
        format!("{POD_NAME_PREFIX}{id_str}-{name_seg}-{suffix}")
    }
}

/// A fresh, collision-resistant validation Pod/ConfigMap name for `(id, name)`.
pub(crate) fn validation_pod_name(id: i64, name: &str) -> String {
    validation_object_name(id, name, &random_suffix())
}

/// Build the isolated validation Pod (pure; no API calls). A bare `core/v1` Pod
/// (not a Job): `restartPolicy: Never`, `activeDeadlineSeconds` set, one
/// `validate-env` container mounting the spec ConfigMap read-only, then the
/// shared #338 R3 hard-isolation box applied — identical to a session pod.
///
/// The container references the ConfigMap by `name` (same as the pod name); the
/// caller creates the ConfigMap right after the Pod, and the kubelet retries the
/// mount until it exists, within the deadline.
pub(crate) fn build_validation_pod(
    name: &str,
    id: i64,
    pod_config: &PodConfig,
    deadline_secs: i64,
) -> Result<Pod, AppError> {
    // Config guarantees an image when dispatch is on; guard here too (a pod with
    // no image is unspawnable). Rendered to the client as a generic 500.
    let image = pod_config
        .image
        .clone()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("FKST_POD_IMAGE is not configured")))?;

    let container = Container {
        name: VALIDATOR_CONTAINER.to_string(),
        image: Some(image),
        args: Some(vec!["validate-env".to_string()]),
        volume_mounts: Some(vec![VolumeMount {
            name: SPEC_VOLUME.to_string(),
            mount_path: SPEC_MOUNT_DIR.to_string(),
            read_only: Some(true),
            ..Default::default()
        }]),
        ..Default::default()
    };

    let volume = Volume {
        name: SPEC_VOLUME.to_string(),
        config_map: Some(ConfigMapVolumeSource {
            name: name.to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let mut pod_spec = PodSpec {
        restart_policy: Some("Never".to_string()),
        service_account_name: Some(pod_config.service_account.clone()),
        // A bare Pod has no Job wrapper, so its own activeDeadlineSeconds is the
        // in-cluster backstop that fails a stuck pod (the poll loop + GC sweep
        // are the control-plane-side backstops).
        active_deadline_seconds: Some(deadline_secs),
        containers: vec![container],
        volumes: Some(vec![volume]),
        ..Default::default()
    };
    // The SAME hard-isolation box as a session pod (no API token, no service
    // discovery, external DNS only, host namespaces off, root boxed).
    crate::k8s::isolation::apply_isolation(&mut pod_spec, &pod_config.dns_nameservers);

    Ok(Pod {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(pod_config.namespace.clone()),
            labels: Some(validation_labels(id)),
            ..Default::default()
        },
        spec: Some(pod_spec),
        ..Default::default()
    })
}

/// Build the spec ConfigMap (pure; no API calls). Carries the serialized
/// [`ValidateSpec`] (`install` + non-secret `variables` + `deadline_secs`);
/// owner-referenced to the created Pod so cascade-GC removes it. Secrets are
/// NEVER written here — only user-declared plaintext variables.
pub(crate) fn build_spec_configmap(
    name: &str,
    namespace: &str,
    id: i64,
    install: &[String],
    variables: &BTreeMap<String, String>,
    deadline_secs: i64,
    owner: Option<OwnerReference>,
) -> Result<ConfigMap, AppError> {
    let spec = ValidateSpec {
        install: install.to_vec(),
        variables: variables.clone(),
        // Config validates deadline >= 1, so the conversion never truncates; a
        // pathological non-positive value degrades to 0 rather than panicking.
        deadline_secs: u64::try_from(deadline_secs).unwrap_or(0),
    };
    let json = serde_json::to_string(&spec)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize validate spec: {e}")))?;

    Ok(ConfigMap {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(validation_labels(id)),
            owner_references: owner.map(|o| vec![o]),
            ..Default::default()
        },
        data: Some(BTreeMap::from([(SPEC_FILE_KEY.to_string(), json)])),
        ..Default::default()
    })
}

/// The OwnerReference a created validation Pod presents to its ConfigMap, so the
/// ConfigMap cascade-deletes when the Pod is removed. `None` if the created Pod
/// is missing a name or UID (it never is post-create).
pub(crate) fn pod_owner_reference(pod: &Pod) -> Option<OwnerReference> {
    let name = pod.metadata.name.clone()?;
    let uid = pod.metadata.uid.clone()?;
    Some(OwnerReference {
        api_version: "v1".to_string(),
        kind: "Pod".to_string(),
        name,
        uid,
        controller: Some(true),
        block_owner_deletion: Some(true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod_config() -> PodConfig {
        PodConfig {
            dispatch: true,
            namespace: "fkst-sessions".to_string(),
            image: Some("registry/fkst-control-plane:1.0".to_string()),
            service_account: "fkst-session-runner".to_string(),
            run_ttl_secs: 600,
            active_deadline_secs: 3600,
            llm_base_url: "https://llm.example/p".to_string(),
            llm_model: "gpt-5-codex".to_string(),
            llm_wire_api: "chat".to_string(),
            dns_nameservers: vec!["1.1.1.1".to_string(), "8.8.8.8".to_string()],
        }
    }

    #[test]
    fn build_validation_pod_is_isolated_and_runs_validate_env() {
        let pod = build_validation_pod("fkst-env-val-1-web-abcd1234", 1, &pod_config(), 300)
            .expect("pod builds");
        let meta = &pod.metadata;
        assert_eq!(meta.name.as_deref(), Some("fkst-env-val-1-web-abcd1234"));
        assert_eq!(meta.namespace.as_deref(), Some("fkst-sessions"));
        let labels = meta.labels.as_ref().expect("labels");
        assert_eq!(
            labels["app.kubernetes.io/component"].as_str(),
            "env-validation"
        );
        assert_eq!(labels["fkst.chrono-ai.fun/github-user-id"].as_str(), "1");

        let spec = pod.spec.as_ref().expect("spec");
        assert_eq!(spec.restart_policy.as_deref(), Some("Never"));
        assert_eq!(spec.active_deadline_seconds, Some(300));
        assert_eq!(
            spec.service_account_name.as_deref(),
            Some("fkst-session-runner")
        );

        let c = &spec.containers[0];
        assert_eq!(c.args.as_deref(), Some(&["validate-env".to_string()][..]));
        let mount = &c.volume_mounts.as_ref().expect("mounts")[0];
        assert_eq!(mount.mount_path, "/var/run/fkst/validate");
        assert_eq!(mount.read_only, Some(true));
        let vol = &spec.volumes.as_ref().expect("volumes")[0];
        assert_eq!(
            vol.config_map.as_ref().expect("cm source").name,
            "fkst-env-val-1-web-abcd1234"
        );

        // The isolation box is applied identically to a session pod.
        assert_eq!(spec.automount_service_account_token, Some(false));
        assert_eq!(spec.enable_service_links, Some(false));
        assert_eq!(spec.dns_policy.as_deref(), Some("None"));
        let sc = spec
            .security_context
            .as_ref()
            .expect("pod security context");
        assert_eq!(sc.run_as_user, Some(0));
        let csc = spec.containers[0]
            .security_context
            .as_ref()
            .expect("container security context");
        assert_eq!(
            csc.capabilities.as_ref().and_then(|c| c.drop.as_deref()),
            Some(&["ALL".to_string()][..])
        );
    }

    #[test]
    fn build_validation_pod_requires_an_image() {
        let mut cfg = pod_config();
        cfg.image = None;
        assert!(matches!(
            build_validation_pod("n", 1, &cfg, 300),
            Err(AppError::Internal(_))
        ));
    }

    #[test]
    fn build_spec_configmap_round_trips_install_variables_and_deadline() {
        let install = vec!["apt-get update".to_string(), "pip install x".to_string()];
        let mut variables = BTreeMap::new();
        variables.insert("FOO".to_string(), "bar".to_string());
        let owner = OwnerReference {
            api_version: "v1".to_string(),
            kind: "Pod".to_string(),
            name: "fkst-env-val-1-web-abcd1234".to_string(),
            uid: "pod-uid-9".to_string(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        };
        let cm = build_spec_configmap(
            "fkst-env-val-1-web-abcd1234",
            "fkst-sessions",
            1,
            &install,
            &variables,
            300,
            Some(owner),
        )
        .expect("configmap builds");

        let data = cm.data.as_ref().expect("data");
        let back: ValidateSpec =
            serde_json::from_str(&data["validate-spec.json"]).expect("spec round-trips");
        assert_eq!(back.install, install);
        assert_eq!(back.variables, variables);
        assert_eq!(back.deadline_secs, 300);

        // Owner reference points at the pod we passed, so cascade-GC works.
        let owners = cm.metadata.owner_references.as_ref().expect("owners");
        assert_eq!(owners[0].kind, "Pod");
        assert_eq!(owners[0].name, "fkst-env-val-1-web-abcd1234");
        assert_eq!(owners[0].uid, "pod-uid-9");
    }

    #[test]
    fn build_spec_configmap_without_owner_leaves_references_unset() {
        let cm = build_spec_configmap("n", "ns", 1, &[], &BTreeMap::new(), 300, None)
            .expect("configmap builds");
        assert!(cm.metadata.owner_references.is_none());
    }

    #[test]
    fn validation_object_name_has_prefix_id_suffix_and_fits_a_label() {
        let name = validation_object_name(42, "web", "abcd1234");
        assert_eq!(name, "fkst-env-val-42-web-abcd1234");
        assert!(name.len() <= POD_NAME_MAX_LEN);
    }

    #[test]
    fn validation_object_name_truncates_a_long_name_to_stay_within_63() {
        let long = "a".repeat(200);
        let name = validation_object_name(1234567, &long, "abcd1234");
        assert!(name.len() <= POD_NAME_MAX_LEN, "len {}", name.len());
        assert!(name.starts_with(POD_NAME_PREFIX));
        assert!(name.ends_with("-abcd1234"));
    }

    #[test]
    fn validation_object_name_sanitizes_and_drops_an_all_invalid_name() {
        // A name of only invalid chars sanitizes to dashes, trims empty, and the
        // segment is dropped rather than emitting a `--`.
        let name = validation_object_name(7, "***", "abcd1234");
        assert_eq!(name, "fkst-env-val-7-abcd1234");
        // Mixed case + symbols are lowercased and dashed.
        let mixed = validation_object_name(7, "My_Env.1", "zzzz0000");
        assert_eq!(mixed, "fkst-env-val-7-my-env-1-zzzz0000");
    }

    #[test]
    fn validation_pod_name_is_a_valid_length_label_with_a_random_suffix() {
        let a = validation_pod_name(99, "web");
        let b = validation_pod_name(99, "web");
        assert!(a.len() <= POD_NAME_MAX_LEN);
        assert!(a.starts_with("fkst-env-val-99-web-"));
        // Overwhelmingly likely to differ (8 chars over a 36-symbol alphabet).
        assert_ne!(a, b);
    }

    #[test]
    fn pod_owner_reference_is_none_without_a_uid() {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("n".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(pod_owner_reference(&pod).is_none());
    }

    #[test]
    fn spec_mount_path_matches_the_in_pod_reader_path() {
        // Drift guard: mount dir + filename MUST equal the path the in-pod
        // runner reads, so the two never diverge.
        assert_eq!(
            format!("{SPEC_MOUNT_DIR}/{SPEC_FILE_KEY}"),
            crate::install::VALIDATE_SPEC_PATH
        );
    }
}
