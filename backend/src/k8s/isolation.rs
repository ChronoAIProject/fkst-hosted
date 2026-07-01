//! Shared pod hard-isolation builder (issue #338 R3).
//!
//! R3 is the security-critical rule that a session (or, in a later PR, a
//! validation) pod MUST be unable to touch ANY other cluster resource: no
//! Kubernetes API credential, no in-cluster service discovery, no host
//! namespaces, and DNS restricted to external resolvers only. The pod runs as
//! ROOT — deliberately — because the session installs OS packages (`apt`,
//! `dpkg`, `pip`) which need to write `/usr`, `/var`, `/etc` and run dpkg
//! postinst hooks that `chown`/`setuid`. Root is not a hole here: it is BOXED
//! by the controls below, so a compromised session gains root only inside its
//! own throwaway pod and can reach nothing else on the cluster.
//!
//! The box, control by control:
//! - `automount_service_account_token: false` — the always-on floor. The pod
//!   carries NO ServiceAccount token, so it cannot call the Kubernetes API at
//!   all, regardless of RBAC.
//! - `enable_service_links: false` — no `*_SERVICE_HOST/PORT` env leaking the
//!   in-cluster service topology into the pod.
//! - `dnsPolicy: None` + explicit external nameservers — the pod cannot resolve
//!   in-cluster (`*.svc.cluster.local`) names; only the configured external
//!   resolvers answer.
//! - host namespaces off (`hostNetwork/hostPID/hostIPC: false`) — no sharing the
//!   node's network, process, or IPC namespace.
//! - container caps dropped to `ALL`, adding back only the handful dpkg postinst
//!   needs; `allowPrivilegeEscalation: false`; `privileged: false`.
//!
//! The one deliberate relaxation is `readOnlyRootFilesystem: false`: package
//! installs must write the root filesystem. Every other lever is set to its most
//! restrictive value so this relaxation stays contained.

use k8s_openapi::api::core::v1::{
    Capabilities, PodDNSConfig, PodSecurityContext, PodSpec, SeccompProfile, SecurityContext,
};

/// Pod-level security context for an isolated pod: runs as root (uid/gid 0) with
/// the default seccomp profile. Root is required ONLY so `apt`/`dpkg`/`pip`
/// installs work; it is boxed by [`apply_isolation`]'s other controls.
pub(crate) fn isolated_pod_security() -> PodSecurityContext {
    PodSecurityContext {
        run_as_user: Some(0),
        run_as_group: Some(0),
        run_as_non_root: Some(false),
        fs_group: None,
        seccomp_profile: Some(SeccompProfile {
            type_: "RuntimeDefault".to_string(),
            localhost_profile: None,
        }),
        ..Default::default()
    }
}

/// Container-level security context for an isolated pod: not privileged, no
/// privilege escalation, all capabilities dropped except the six dpkg postinst
/// hooks need. The root filesystem stays writable — the ONE deliberate
/// relaxation — because package installs write `/usr`, `/var`, `/etc`.
pub(crate) fn isolated_container_security() -> SecurityContext {
    SecurityContext {
        privileged: Some(false),
        allow_privilege_escalation: Some(false),
        // Installs must write /usr, /var, /etc; the ONE deliberate relaxation.
        read_only_root_filesystem: Some(false),
        run_as_non_root: Some(false),
        capabilities: Some(Capabilities {
            drop: Some(vec!["ALL".to_string()]),
            add: Some(vec![
                "CHOWN".to_string(),
                "DAC_OVERRIDE".to_string(),
                "FOWNER".to_string(),
                "FSETID".to_string(),
                "SETUID".to_string(),
                "SETGID".to_string(),
            ]),
        }),
        ..Default::default()
    }
}

/// Apply the #338 R3 hard-isolation box to `pod_spec` in place so the pod is
/// isolated on EVERY CNI. Drops the ServiceAccount token, disables service-link
/// env, pins DNS to the given external `dns_nameservers`, turns off host
/// namespaces, and stamps the isolated pod/container security contexts onto the
/// spec and every container.
///
/// `runtime_class` selects the pod `runtimeClassName`: `None` leaves it unset —
/// the cluster default runtime (runc) — which is all local/docker-desktop can
/// do, while a value like `kata` selects a sandboxed RuntimeClass (Kata
/// Containers). Kata is the real isolation boundary layered on top of the K8s
/// hardening above; the nodes must provide the named RuntimeClass.
pub(crate) fn apply_isolation(
    pod_spec: &mut PodSpec,
    dns_nameservers: &[String],
    runtime_class: Option<&str>,
) {
    // The always-on floor: no API credential mounted into the pod.
    pod_spec.automount_service_account_token = Some(false);
    // No `*_SERVICE_HOST/PORT` env exposing the in-cluster service topology.
    pod_spec.enable_service_links = Some(false);
    // External DNS only — the pod cannot resolve in-cluster service names.
    pod_spec.dns_policy = Some("None".to_string());
    pod_spec.dns_config = Some(PodDNSConfig {
        nameservers: Some(dns_nameservers.to_vec()),
        options: None,
        searches: None,
    });
    // No host namespace sharing.
    pod_spec.host_network = Some(false);
    pod_spec.host_pid = Some(false);
    pod_spec.host_ipc = Some(false);
    // Sandbox runtime: None = the cluster default runtime (runc), a value like
    // `kata` selects the sandboxed RuntimeClass — the strongest isolation tier,
    // layered on top of the K8s hardening above.
    pod_spec.runtime_class_name = runtime_class.map(|s| s.to_string());
    // Root, boxed (see module docs).
    pod_spec.security_context = Some(isolated_pod_security());
    for container in &mut pod_spec.containers {
        container.security_context = Some(isolated_container_security());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::Container;

    fn pod_with_one_container() -> PodSpec {
        PodSpec {
            containers: vec![Container {
                name: "runner".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn apply_isolation_sets_the_pod_level_floor() {
        let mut pod = pod_with_one_container();
        apply_isolation(
            &mut pod,
            &["1.1.1.1".to_string(), "8.8.8.8".to_string()],
            None,
        );

        assert_eq!(pod.automount_service_account_token, Some(false));
        assert_eq!(pod.enable_service_links, Some(false));
        assert_eq!(pod.dns_policy.as_deref(), Some("None"));
        assert_eq!(pod.host_network, Some(false));
        assert_eq!(pod.host_pid, Some(false));
        assert_eq!(pod.host_ipc, Some(false));
    }

    #[test]
    fn apply_isolation_pins_external_dns_nameservers() {
        let mut pod = pod_with_one_container();
        let servers = vec!["9.9.9.9".to_string(), "1.0.0.1".to_string()];
        apply_isolation(&mut pod, &servers, None);

        let dns = pod.dns_config.as_ref().expect("dns config");
        assert_eq!(dns.nameservers.as_deref(), Some(&servers[..]));
        assert_eq!(dns.options, None);
        assert_eq!(dns.searches, None);
    }

    #[test]
    fn apply_isolation_runs_the_pod_as_boxed_root() {
        let mut pod = pod_with_one_container();
        apply_isolation(&mut pod, &["1.1.1.1".to_string()], None);

        let sc = pod.security_context.as_ref().expect("pod security context");
        assert_eq!(sc.run_as_user, Some(0));
        assert_eq!(sc.run_as_group, Some(0));
        assert_eq!(sc.run_as_non_root, Some(false));
        assert_eq!(sc.fs_group, None);
        let seccomp = sc.seccomp_profile.as_ref().expect("seccomp profile");
        assert_eq!(seccomp.type_, "RuntimeDefault");
        assert_eq!(seccomp.localhost_profile, None);
    }

    #[test]
    fn apply_isolation_boxes_every_container() {
        let mut pod = pod_with_one_container();
        apply_isolation(&mut pod, &["1.1.1.1".to_string()], None);

        let csc = pod.containers[0]
            .security_context
            .as_ref()
            .expect("container security context");
        assert_eq!(csc.privileged, Some(false));
        assert_eq!(csc.allow_privilege_escalation, Some(false));
        assert_eq!(csc.read_only_root_filesystem, Some(false));
        assert_eq!(csc.run_as_non_root, Some(false));

        let caps = csc.capabilities.as_ref().expect("capabilities");
        assert_eq!(caps.drop.as_deref(), Some(&["ALL".to_string()][..]));
        assert_eq!(
            caps.add.as_deref(),
            Some(
                &[
                    "CHOWN".to_string(),
                    "DAC_OVERRIDE".to_string(),
                    "FOWNER".to_string(),
                    "FSETID".to_string(),
                    "SETUID".to_string(),
                    "SETGID".to_string(),
                ][..]
            )
        );
    }

    #[test]
    fn apply_isolation_sets_the_runtime_class_when_given() {
        let mut pod = pod_with_one_container();
        apply_isolation(&mut pod, &["1.1.1.1".to_string()], Some("kata"));
        assert_eq!(pod.runtime_class_name.as_deref(), Some("kata"));
    }

    #[test]
    fn apply_isolation_leaves_the_runtime_class_unset_by_default() {
        // None keeps the cluster default runtime (runc) — required for
        // local/docker-desktop, which has no Kata RuntimeClass.
        let mut pod = pod_with_one_container();
        apply_isolation(&mut pod, &["1.1.1.1".to_string()], None);
        assert_eq!(pod.runtime_class_name, None);
    }
}
