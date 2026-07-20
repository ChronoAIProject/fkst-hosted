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

use super::{resolve_effective_packages, NO_PACKAGES_DETAIL};
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
        def: SessionDef {
            name: "sess".to_string(),
            packages,
            manifest_refs,
            work_label: Some("wl".to_string()),
            environment: None,
            output_lang: None,
            engine_config: BTreeMap::new(),
        },
        effective_packages: Vec::new(),
        session_id: session_id.to_string(),
        config_hash: "h".to_string(),
        auto_merge: false,
        log_access: vec![],
        collaborators: vec![],
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
    resolve_effective_packages(&reqwest::Client::new(), &server.uri(), &tok(), regs).await
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
