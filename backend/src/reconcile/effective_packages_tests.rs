//! Tests for the I7 effective-package resolution: expand each session's `### Manifest`
//! references and merge them (deduped, explicit-first) with its explicit `### Packages`,
//! over a mocked GitHub contents API. Covers the effective-set rule, fail-closed demotion
//! (expansion failure / empty union), the per-pass fetch-once cache, byte-identical
//! behavior for a manifest-free session, and that a manifest-only package contributes its
//! `[github].work_labels` to work-label discovery.

use std::collections::BTreeMap;

use secrecy::SecretString;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{resolve_effective_packages, MAX_EFFECTIVE_PACKAGE_WORKSPACES, NO_PACKAGES_DETAIL};
use crate::goals::trigger_parse::PackageRef;
use crate::models::RepoRef;
use crate::reconcile::desired::{SessionDef, SessionRegistration};
use crate::reconcile::work_labels::resolve_work_labels;

fn pkg(owner: &str, repo: &str, git_ref: &str, path: &str) -> PackageRef {
    PackageRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        git_ref: git_ref.to_string(),
        path: path.to_string(),
    }
}

fn tok() -> SecretString {
    SecretString::from("ghs_effpkg_secret".to_string())
}

/// Build a registration with the given explicit packages + manifest refs. Every other
/// field is inert for these tests — only `session_id`, `trigger_issue`, and `def` matter.
fn reg(
    session_id: &str,
    trigger_issue: i64,
    packages: Vec<PackageRef>,
    manifest_refs: Vec<PackageRef>,
) -> SessionRegistration {
    SessionRegistration {
        installation_id: 1,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        trigger_issue,
        trigger_author_id: 7,
        trigger_author_login: "author".to_string(),
        creator_login: "author".to_string(),
        creator_id: Some(7),
        def: SessionDef {
            name: "sess".to_string(),
            packages,
            manifest_refs,
            work_label: Some("wl".to_string()),
            environment: None,
            output_lang: None,
            engine_config: BTreeMap::new(),
            source_branch: None,
            target_branch: None,
            package_env: crate::goals::package_env::PackageEnv::new(),
        },
        effective_packages: Vec::new(),
        session_id: session_id.to_string(),
        config_hash: "h".to_string(),
        auto_merge: false,
        log_access: vec![],
        collaborators: vec![],
        effective_package_env: crate::goals::package_env::PackageEnv::new(),
    }
}

fn manifest_body(schema_version: i64, packages: &[&str]) -> String {
    json!({
        "schemaVersion": schema_version,
        "name": "m",
        "description": "",
        "packages": packages,
    })
    .to_string()
}

fn distinct_workspace_packages(count: usize) -> Vec<PackageRef> {
    (0..count)
        .map(|i| pkg(&format!("org{i}"), "pkgs", "main", "packages/workflow"))
        .collect()
}

fn same_workspace_packages(count: usize) -> Vec<PackageRef> {
    (0..count)
        .map(|i| pkg("acme", "pkgs", "main", &format!("packages/workflow-{i}")))
        .collect()
}

/// Mount a manifest JSON body at its contents path (the path IS the .json file).
async fn mount_manifest_json(server: &MockServer, m: &PackageRef, body: String) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/{}/{}/contents/{}",
            m.owner, m.repo, m.path
        )))
        .and(query_param("ref", m.git_ref.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

/// Mount a non-2xx status at a manifest's contents path.
async fn mount_status(server: &MockServer, m: &PackageRef, status: u16) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/{}/{}/contents/{}",
            m.owner, m.repo, m.path
        )))
        .and(query_param("ref", m.git_ref.as_str()))
        .respond_with(ResponseTemplate::new(status))
        .mount(server)
        .await;
}

async fn resolve(server: &MockServer, regs: &[SessionRegistration]) -> super::EffectivePackages {
    resolve_effective_packages(&reqwest::Client::new(), &server.uri(), &tok(), regs, &[]).await
}

/// Same, with a mandatory baseline prepended to every session.
async fn resolve_with_mandatory(
    server: &MockServer,
    regs: &[SessionRegistration],
    mandatory: &[PackageRef],
) -> super::EffectivePackages {
    resolve_effective_packages(
        &reqwest::Client::new(),
        &server.uri(),
        &tok(),
        regs,
        mandatory,
    )
    .await
}

#[tokio::test]
async fn effective_is_explicit_then_manifest_expanded() {
    let server = MockServer::start().await;
    let manifest = pkg("acme", "manifests", "main", "m.json");
    mount_manifest_json(
        &server,
        &manifest,
        manifest_body(1, &["acme/pkgs@main:packages/from-manifest"]),
    )
    .await;

    let explicit = pkg("acme", "pkgs", "main", "packages/explicit");
    let regs = vec![reg("s1", 1, vec![explicit.clone()], vec![manifest])];
    let out = resolve(&server, &regs).await;

    assert!(
        out.demotions.is_empty(),
        "a clean expansion demotes nothing"
    );
    assert_eq!(
        out.by_session.get("s1"),
        Some(&vec![
            explicit,
            pkg("acme", "pkgs", "main", "packages/from-manifest"),
        ]),
        "effective = explicit first, then the manifest expansion, in order"
    );
}

#[tokio::test]
async fn dedup_keeps_the_explicit_occurrence_first() {
    let server = MockServer::start().await;
    let manifest = pkg("acme", "manifests", "main", "m.json");
    // The manifest re-declares the explicit package, plus a new one.
    mount_manifest_json(
        &server,
        &manifest,
        manifest_body(
            1,
            &[
                "acme/pkgs@main:packages/shared",
                "acme/pkgs@main:packages/manifest-only",
            ],
        ),
    )
    .await;

    let shared = pkg("acme", "pkgs", "main", "packages/shared");
    let regs = vec![reg("s1", 1, vec![shared.clone()], vec![manifest])];
    let out = resolve(&server, &regs).await;

    assert_eq!(
        out.by_session.get("s1"),
        Some(&vec![
            shared,
            pkg("acme", "pkgs", "main", "packages/manifest-only"),
        ]),
        "the shared package survives ONCE, in its explicit position (explicit-first)"
    );
}

#[tokio::test]
async fn manifest_only_session_runs_on_the_manifest_packages() {
    let server = MockServer::start().await;
    let manifest = pkg("acme", "manifests", "main", "m.json");
    mount_manifest_json(
        &server,
        &manifest,
        manifest_body(
            1,
            &["acme/pkgs@main:packages/a", "acme/pkgs@main:packages/b"],
        ),
    )
    .await;

    // No explicit `### Packages` — the manifest supplies the whole set.
    let regs = vec![reg("s1", 1, vec![], vec![manifest])];
    let out = resolve(&server, &regs).await;

    assert!(out.demotions.is_empty());
    assert_eq!(
        out.by_session.get("s1"),
        Some(&vec![
            pkg("acme", "pkgs", "main", "packages/a"),
            pkg("acme", "pkgs", "main", "packages/b"),
        ]),
    );
}

#[tokio::test]
async fn multiple_manifests_expand_in_manifest_order() {
    let server = MockServer::start().await;
    let m1 = pkg("acme", "manifests", "main", "one.json");
    let m2 = pkg("acme", "manifests", "main", "two.json");
    mount_manifest_json(
        &server,
        &m1,
        manifest_body(1, &["acme/pkgs@main:packages/one"]),
    )
    .await;
    mount_manifest_json(
        &server,
        &m2,
        manifest_body(1, &["acme/pkgs@main:packages/two"]),
    )
    .await;

    let regs = vec![reg("s1", 1, vec![], vec![m1, m2])];
    let out = resolve(&server, &regs).await;

    assert_eq!(
        out.by_session.get("s1"),
        Some(&vec![
            pkg("acme", "pkgs", "main", "packages/one"),
            pkg("acme", "pkgs", "main", "packages/two"),
        ]),
        "manifests expand in manifest order",
    );
}

#[tokio::test]
async fn expansion_failure_demotes_with_a_reason() {
    let server = MockServer::start().await;
    let manifest = pkg("acme", "manifests", "main", "missing.json");
    mount_status(&server, &manifest, 404).await;

    let regs = vec![reg("s7", 7, vec![], vec![manifest])];
    let out = resolve(&server, &regs).await;

    assert!(
        !out.by_session.contains_key("s7"),
        "a failed session has no effective set (it is fail-closed)"
    );
    assert_eq!(out.demotions.len(), 1);
    let (issue, reason) = &out.demotions[0];
    assert_eq!(*issue, 7);
    assert!(
        reason.contains("acme/manifests@main:missing.json"),
        "the reason names the offending manifest: {reason}"
    );
    assert!(
        reason.contains("could not be expanded"),
        "the reason states the failure: {reason}"
    );
}

#[tokio::test]
async fn one_bad_manifest_does_not_taint_a_sibling_session() {
    let server = MockServer::start().await;
    let good = pkg("acme", "manifests", "main", "good.json");
    let bad = pkg("acme", "manifests", "main", "bad.json");
    mount_manifest_json(
        &server,
        &good,
        manifest_body(1, &["acme/pkgs@main:packages/ok"]),
    )
    .await;
    mount_status(&server, &bad, 500).await;

    let regs = vec![
        reg("s1", 1, vec![], vec![good]),
        reg("s2", 2, vec![], vec![bad]),
    ];
    let out = resolve(&server, &regs).await;

    assert_eq!(
        out.by_session.get("s1"),
        Some(&vec![pkg("acme", "pkgs", "main", "packages/ok")]),
        "the healthy session resolves cleanly"
    );
    assert!(!out.by_session.contains_key("s2"));
    assert_eq!(
        out.demotions.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        vec![2],
        "only the session with the bad manifest is demoted"
    );
}

#[tokio::test]
async fn empty_effective_union_demotes() {
    let server = MockServer::start().await;
    // A defensive path: no explicit packages AND no manifests → an empty union. The
    // trigger parser normally forbids this, but the resolver must still fail closed.
    let regs = vec![reg("s3", 3, vec![], vec![])];
    let out = resolve(&server, &regs).await;

    assert!(!out.by_session.contains_key("s3"));
    assert_eq!(out.demotions, vec![(3, NO_PACKAGES_DETAIL.to_string())]);
}

#[tokio::test]
async fn manifest_free_session_effective_equals_explicit() {
    let server = MockServer::start().await; // never queried — no manifest to fetch
    let packages = vec![
        pkg("acme", "pkgs", "main", "packages/a"),
        pkg("o", "r", "dev", "packages/b"),
    ];
    let regs = vec![reg("s1", 1, packages.clone(), vec![])];
    let out = resolve(&server, &regs).await;

    assert!(out.demotions.is_empty());
    assert_eq!(
        out.by_session.get("s1"),
        Some(&packages),
        "no manifest → effective is EXACTLY the explicit packages, order preserved (byte-identical)"
    );
}

#[tokio::test]
async fn exactly_at_the_effective_workspace_limit_is_allowed() {
    let server = MockServer::start().await; // never queried — no manifest to fetch
    let packages = distinct_workspace_packages(MAX_EFFECTIVE_PACKAGE_WORKSPACES);
    let regs = vec![reg("s1", 1, packages.clone(), vec![])];
    let out = resolve(&server, &regs).await;

    assert!(out.demotions.is_empty());
    assert_eq!(
        out.by_session.get("s1").map(Vec::len),
        Some(MAX_EFFECTIVE_PACKAGE_WORKSPACES),
        "the boundary itself is accepted"
    );
}

#[tokio::test]
async fn one_over_the_effective_workspace_limit_demotes_before_pod_creation() {
    let server = MockServer::start().await; // never queried — no manifest to fetch
    let packages = distinct_workspace_packages(MAX_EFFECTIVE_PACKAGE_WORKSPACES + 1);
    let regs = vec![reg("s1", 7, packages, vec![])];
    let out = resolve(&server, &regs).await;

    assert!(
        !out.by_session.contains_key("s1"),
        "an over-limit session must not produce a spawnable effective set"
    );
    assert_eq!(out.demotions.len(), 1);
    let (issue, reason) = &out.demotions[0];
    assert_eq!(*issue, 7);
    assert!(
        reason.contains("package workspaces")
            && reason.contains(&(MAX_EFFECTIVE_PACKAGE_WORKSPACES + 1).to_string())
            && reason.contains(&MAX_EFFECTIVE_PACKAGE_WORKSPACES.to_string()),
        "reason should name the workspace limit: {reason}"
    );
}

#[tokio::test]
async fn many_packages_sharing_one_workspace_do_not_trip_the_workspace_limit() {
    let server = MockServer::start().await; // never queried — no manifest to fetch
    let packages = same_workspace_packages(MAX_EFFECTIVE_PACKAGE_WORKSPACES + 1);
    let regs = vec![reg("s1", 1, packages.clone(), vec![])];
    let out = resolve(&server, &regs).await;

    assert!(
        out.demotions.is_empty(),
        "the cap is on distinct cloned workspaces, not paths inside one workspace"
    );
    assert_eq!(out.by_session.get("s1").map(Vec::len), Some(packages.len()));
}

#[tokio::test]
async fn a_shared_manifest_is_fetched_once_per_pass() {
    let server = MockServer::start().await;
    let manifest = pkg("acme", "manifests", "main", "m.json");
    // `.expect(1)` (verified on server drop) proves the per-pass cache: two sessions
    // reference the same manifest, but it is fetched exactly once.
    Mock::given(method("GET"))
        .and(path("/repos/acme/manifests/contents/m.json"))
        .and(query_param("ref", "main"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(manifest_body(1, &["acme/pkgs@main:packages/a"])),
        )
        .expect(1)
        .mount(&server)
        .await;

    let regs = vec![
        reg("s1", 1, vec![], vec![manifest.clone()]),
        reg("s2", 2, vec![], vec![manifest]),
    ];
    let out = resolve(&server, &regs).await;

    assert!(out.demotions.is_empty());
    assert!(out.by_session.contains_key("s1"));
    assert!(out.by_session.contains_key("s2"));
}

#[tokio::test]
async fn a_manifest_package_contributes_its_work_labels() {
    let server = MockServer::start().await;
    let manifest = pkg("acme", "manifests", "main", "m.json");
    // The manifest expands to one package...
    mount_manifest_json(
        &server,
        &manifest,
        manifest_body(1, &["acme/pkgs@main:packages/labeled"]),
    )
    .await;
    // ...whose fkst.toml declares a work label (reachable ONLY via the manifest).
    Mock::given(method("GET"))
        .and(path("/repos/acme/pkgs/contents/packages/labeled/fkst.toml"))
        .and(query_param("ref", "main"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[github]\nwork_labels = [\"fkst-from-manifest\"]\n"),
        )
        .mount(&server)
        .await;

    // A manifest-only session: resolve the effective set, then discover labels over it —
    // mirroring how the reconcile driver feeds `effective_packages` into label discovery.
    let regs = vec![reg("s1", 1, vec![], vec![manifest])];
    let out = resolve(&server, &regs).await;
    let effective = out.by_session.get("s1").expect("resolved").clone();

    let labels =
        resolve_work_labels(&reqwest::Client::new(), &server.uri(), &tok(), &effective).await;
    assert!(
        labels.contains("fkst-from-manifest"),
        "a package reachable only via the manifest contributes its work labels: {labels:?}"
    );
}

/// Per-package configuration merges manifest defaults with the trigger's own
/// settings. The precedence is per KEY, not per block: a trigger overriding one
/// setting must not silently discard the manifest's other settings for that
/// package, which is the failure mode a whole-block replace would have.
#[tokio::test]
async fn trigger_package_env_overrides_the_manifest_per_key() {
    let server = MockServer::start().await;
    let m = pkg("acme", "manifests", "main", "m.json");
    mount_manifest_json(
        &server,
        &m,
        json!({
            "schemaVersion": 1,
            "packages": ["acme/tools@main:pkg/a"],
            "packageEnv": {
                "github-devloop": {
                    "FKST_DEVLOOP_AUTO_REFINE_MAX": "2",
                    "FKST_DEVLOOP_ROLLUP_MERGE": "auto"
                }
            }
        })
        .to_string(),
    )
    .await;

    let mut reg = reg("s1", 1, vec![], vec![m]);
    reg.def.package_env.insert(
        "github-devloop".to_string(),
        [("FKST_DEVLOOP_AUTO_REFINE_MAX".to_string(), "0".to_string())]
            .into_iter()
            .collect(),
    );

    let resolved =
        resolve_effective_packages(&reqwest::Client::new(), &server.uri(), &tok(), &[reg], &[])
            .await;

    let env = &resolved.package_env_by_session["s1"]["github-devloop"];
    assert_eq!(
        env["FKST_DEVLOOP_AUTO_REFINE_MAX"], "0",
        "the trigger must win for the key it sets"
    );
    assert_eq!(
        env["FKST_DEVLOOP_ROLLUP_MERGE"], "auto",
        "a manifest key the trigger did not touch must survive"
    );
}

/// A manifest-free session's effective configuration is exactly its trigger's.
#[tokio::test]
async fn a_manifest_free_session_keeps_its_trigger_package_env() {
    let server = MockServer::start().await;
    let mut reg = reg("s1", 1, vec![pkg("acme", "tools", "main", "pkg/a")], vec![]);
    reg.def.package_env.insert(
        "github-devloop".to_string(),
        [("FKST_DEVLOOP_AUTO_REFINE_MAX".to_string(), "2".to_string())]
            .into_iter()
            .collect(),
    );

    let resolved =
        resolve_effective_packages(&reqwest::Client::new(), &server.uri(), &tok(), &[reg], &[])
            .await;

    assert_eq!(
        resolved.package_env_by_session["s1"]["github-devloop"]["FKST_DEVLOOP_AUTO_REFINE_MAX"],
        "2"
    );
}

/// A key configured under two different package blocks reaches the pod as one
/// flat environment, where the in-pod reader errors at boot and the session never
/// starts. The trigger's own section rejects this and each manifest is checked in
/// isolation, but their UNION was not — so it had to be caught here, where the
/// author can see it.
#[tokio::test]
async fn a_manifest_and_trigger_key_conflict_demotes_instead_of_killing_the_pod() {
    let server = MockServer::start().await;
    let m = pkg("acme", "manifests", "main", "m.json");
    mount_manifest_json(
        &server,
        &m,
        json!({
            "schemaVersion": 1,
            "packages": ["acme/tools@main:pkg/a"],
            "packageEnv": { "alpha": { "FKST_DEVLOOP_TEST_COMMAND": "from-manifest" } }
        })
        .to_string(),
    )
    .await;

    let mut reg = reg("s1", 1, vec![], vec![m]);
    reg.def.package_env.insert(
        "beta".to_string(),
        [(
            "FKST_DEVLOOP_TEST_COMMAND".to_string(),
            "from-trigger".to_string(),
        )]
        .into_iter()
        .collect(),
    );

    let resolved =
        resolve_effective_packages(&reqwest::Client::new(), &server.uri(), &tok(), &[reg], &[])
            .await;

    assert!(
        !resolved.package_env_by_session.contains_key("s1"),
        "a conflicting session must not resolve"
    );
    let (_, reason) = resolved
        .demotions
        .first()
        .expect("the conflict must demote the session");
    assert!(reason.contains("FKST_DEVLOOP_TEST_COMMAND"), "{reason}");
    assert!(reason.contains("alpha"), "{reason}");
    assert!(reason.contains("beta"), "{reason}");
}

/// Manifest-over-manifest keeps the FIRST value, matching how the effective
/// package list keeps its first occurrence. An unconditional insert gave
/// last-wins here, contradicting the documented precedence.
#[tokio::test]
async fn the_first_manifest_wins_a_cross_manifest_key_collision() {
    let server = MockServer::start().await;
    let first = pkg("acme", "manifests", "main", "first.json");
    let second = pkg("acme", "manifests", "main", "second.json");
    mount_manifest_json(
        &server,
        &first,
        json!({
            "schemaVersion": 1,
            "packages": ["acme/tools@main:pkg/a"],
            "packageEnv": { "github-devloop": { "FKST_DEVLOOP_MAX_INFLIGHT": "1" } }
        })
        .to_string(),
    )
    .await;
    mount_manifest_json(
        &server,
        &second,
        json!({
            "schemaVersion": 1,
            "packages": ["acme/tools@main:pkg/b"],
            "packageEnv": { "github-devloop": { "FKST_DEVLOOP_MAX_INFLIGHT": "9" } }
        })
        .to_string(),
    )
    .await;

    let reg = reg("s1", 1, vec![], vec![first, second]);
    let resolved =
        resolve_effective_packages(&reqwest::Client::new(), &server.uri(), &tok(), &[reg], &[])
            .await;

    assert_eq!(
        resolved.package_env_by_session["s1"]["github-devloop"]["FKST_DEVLOOP_MAX_INFLIGHT"], "1",
        "the first manifest to set a key must win"
    );
}

/// Feature off: requiring nothing accepts everything and leaves the set untouched.
#[tokio::test]
async fn an_empty_mandatory_list_requires_nothing() {
    let server = MockServer::start().await;
    let a = pkg("acme", "p", "main", "packages/a");
    let regs = vec![reg("s1", 1, vec![a.clone()], vec![])];
    let got = resolve_with_mandatory(&server, &regs, &[]).await;
    assert!(got.demotions.is_empty());
    assert_eq!(got.by_session.get("s1"), Some(&vec![a]));
}

/// Declaring the baseline is accepted -- and the effective set is NOT reordered or
/// added to, which is the whole point of requiring rather than injecting.
#[tokio::test]
async fn declaring_every_mandatory_ref_is_accepted_unchanged() {
    let server = MockServer::start().await;
    let proxy = pkg(
        "ChronoAIProject",
        "fkst-hosted",
        "packages",
        "packages/github-proxy",
    );
    let own = pkg("acme", "p", "main", "packages/own");
    let regs = vec![reg("s1", 1, vec![own.clone(), proxy.clone()], vec![])];
    let got = resolve_with_mandatory(&server, &regs, std::slice::from_ref(&proxy)).await;
    assert!(got.demotions.is_empty());
    assert_eq!(got.by_session.get("s1"), Some(&vec![own, proxy]));
}

#[tokio::test]
async fn a_missing_mandatory_ref_demotes_and_names_it() {
    let server = MockServer::start().await;
    let proxy = pkg(
        "ChronoAIProject",
        "fkst-hosted",
        "packages",
        "packages/github-proxy",
    );
    let own = pkg("acme", "p", "main", "packages/own");
    let regs = vec![reg("s1", 1, vec![own], vec![])];
    let got = resolve_with_mandatory(&server, &regs, std::slice::from_ref(&proxy)).await;
    assert!(!got.by_session.contains_key("s1"));
    assert_eq!(got.demotions.len(), 1);
    let (issue, reason) = &got.demotions[0];
    assert_eq!(*issue, 1);
    assert!(
        reason.contains("ChronoAIProject/fkst-hosted@packages:packages/github-proxy"),
        "{reason}"
    );
}

/// ALL misses at once: one round trip per missing package would be a poor refusal.
#[tokio::test]
async fn every_missing_mandatory_ref_is_named() {
    let server = MockServer::start().await;
    let proxy = pkg(
        "ChronoAIProject",
        "fkst-hosted",
        "packages",
        "packages/github-proxy",
    );
    let dev = pkg(
        "ChronoAIProject",
        "fkst-hosted",
        "packages",
        "packages/workflow-dev",
    );
    let own = pkg("acme", "p", "main", "packages/own");
    let regs = vec![reg("s1", 1, vec![own], vec![])];
    let got = resolve_with_mandatory(&server, &regs, &[proxy, dev]).await;
    let (_, reason) = &got.demotions[0];
    assert!(reason.contains("packages/github-proxy"), "{reason}");
    assert!(reason.contains("packages/workflow-dev"), "{reason}");
    assert!(reason.contains('2'), "{reason}");
}

/// A manifest carrying the baseline satisfies the requirement -- the check is against
/// the EFFECTIVE set, so an author need not restate what their manifest already brings.
#[tokio::test]
async fn a_manifest_supplying_the_baseline_satisfies_the_requirement() {
    let server = MockServer::start().await;
    let manifest = pkg("acme", "manifests", "main", "m.json");
    let proxy = pkg(
        "ChronoAIProject",
        "fkst-hosted",
        "packages",
        "packages/github-proxy",
    );
    mount_manifest_json(
        &server,
        &manifest,
        manifest_body(
            1,
            &["ChronoAIProject/fkst-hosted@packages:packages/github-proxy"],
        ),
    )
    .await;
    let regs = vec![reg("s1", 1, vec![], vec![manifest])];
    let got = resolve_with_mandatory(&server, &regs, std::slice::from_ref(&proxy)).await;
    assert!(got.demotions.is_empty(), "{:?}", got.demotions);
    assert_eq!(got.by_session.get("s1"), Some(&vec![proxy]));
}

/// The same package at a DIFFERENT ref does not satisfy the requirement: matching on
/// the full identity is what pins the baseline to the intended branch.
#[tokio::test]
async fn the_same_package_at_another_ref_does_not_satisfy_the_requirement() {
    let server = MockServer::start().await;
    let required = pkg(
        "ChronoAIProject",
        "fkst-hosted",
        "packages",
        "packages/github-proxy",
    );
    let other_ref = pkg(
        "ChronoAIProject",
        "fkst-hosted",
        "old-branch",
        "packages/github-proxy",
    );
    let regs = vec![reg("s1", 1, vec![other_ref], vec![])];
    let got = resolve_with_mandatory(&server, &regs, std::slice::from_ref(&required)).await;
    assert_eq!(got.demotions.len(), 1);
    assert!(
        got.demotions[0].1.contains("@packages:"),
        "{}",
        got.demotions[0].1
    );
}
