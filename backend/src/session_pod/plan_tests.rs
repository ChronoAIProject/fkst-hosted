//! Unit tests for the pure `run-substrate` planners ([`super`]). Split into their
//! own file so `plan.rs` stays well under the 500-line module cap.

use super::*;

fn full_env() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("FKST_GITHUB_REPO".to_string(), "acme/site".to_string()),
        (
            "FKST_SESSION_PACKAGE_ROOTS".to_string(),
            "org/pkgs@dev:packages/github-devloop org/pkgs@dev:packages/github-proxy".to_string(),
        ),
        ("FKST_SESSION_WORK_LABEL".to_string(), "audit".to_string()),
        ("FKST_GITHUB_BOT_LOGIN".to_string(), "fkst-bot".to_string()),
        ("FKST_LLM_MODEL".to_string(), "gpt-4.1".to_string()),
        (
            "FKST_LLM_BASE_URL".to_string(),
            "https://proxy/s/llm".to_string(),
        ),
        ("FKST_LLM_WIRE_API".to_string(), "chat".to_string()),
        (
            "FKST_DURABLE_ROOT".to_string(),
            "/var/run/fkst/durable".to_string(),
        ),
        (
            "FKST_RUNTIME_ROOT".to_string(),
            "/var/run/fkst/runtime".to_string(),
        ),
        (
            "FKST_SESSION_CREDS_DIR".to_string(),
            "/var/run/fkst/creds".to_string(),
        ),
        ("CODEX_HOME".to_string(), "/var/run/fkst/codex".to_string()),
    ])
}

fn lookup(map: &BTreeMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
    move |key| map.get(key).cloned()
}

#[test]
fn read_substrate_env_maps_a_full_env() {
    let map = full_env();
    let env = read_substrate_env_from(lookup(&map)).expect("full env parses");
    assert_eq!(env.repo, "acme/site");
    assert_eq!(env.package_refs.len(), 2);
    assert_eq!(env.package_refs[0].path, "packages/github-devloop");
    assert_eq!(env.package_refs[1].path, "packages/github-proxy");
    assert_eq!(env.work_label, "audit");
    assert_eq!(env.bot_login, "fkst-bot");
    assert_eq!(env.llm_model, "gpt-4.1");
    assert_eq!(env.llm_base_url, "https://proxy/s/llm");
    assert_eq!(env.llm_wire_api, "chat");
    assert_eq!(env.durable_root, "/var/run/fkst/durable");
    assert_eq!(env.runtime_root, "/var/run/fkst/runtime");
    assert_eq!(env.creds_dir, "/var/run/fkst/creds");
    assert_eq!(env.codex_home, "/var/run/fkst/codex");
}

#[test]
fn read_substrate_env_defaults_the_llm_trio_when_absent() {
    let mut map = full_env();
    map.remove("FKST_LLM_MODEL");
    map.remove("FKST_LLM_BASE_URL");
    map.remove("FKST_LLM_WIRE_API");
    let env = read_substrate_env_from(lookup(&map)).expect("defaults apply");
    assert_eq!(env.llm_model, DEFAULT_LLM_MODEL);
    assert_eq!(env.llm_base_url, DEFAULT_LLM_BASE_URL);
    assert_eq!(env.llm_wire_api, "chat");
}

#[test]
fn read_substrate_env_errors_on_a_missing_required_var() {
    let mut map = full_env();
    map.remove("FKST_GITHUB_REPO");
    let err = read_substrate_env_from(lookup(&map)).expect_err("missing repo must fail");
    assert!(err.contains("FKST_GITHUB_REPO"), "{err}");
}

#[test]
fn read_substrate_env_errors_on_a_malformed_repo() {
    let mut map = full_env();
    map.insert("FKST_GITHUB_REPO".to_string(), "no-slash".to_string());
    let err = read_substrate_env_from(lookup(&map)).expect_err("bad repo must fail");
    assert!(err.contains("owner/name"), "{err}");
}

#[test]
fn read_substrate_env_errors_on_a_malformed_package_ref() {
    let mut map = full_env();
    map.insert(
        "FKST_SESSION_PACKAGE_ROOTS".to_string(),
        "not-a-valid-ref".to_string(),
    );
    let err = read_substrate_env_from(lookup(&map)).expect_err("bad ref must fail");
    assert!(err.contains("package ref"), "{err}");
}

fn refs(specs: &[&str]) -> Vec<PackageRef> {
    specs
        .iter()
        .map(|s| parse_package_ref(s).expect("valid ref fixture"))
        .collect()
}

#[test]
fn plan_clones_groups_one_workspace_and_derives_names() {
    let refs = refs(&[
        "org/pkgs@dev:packages/github-devloop",
        "org/pkgs@dev:packages/github-proxy",
    ]);
    let plan = plan_clones(&refs).expect("single workspace plans");
    assert_eq!(
        plan.platform_repo,
        WorkspaceRepo {
            owner: "org".to_string(),
            repo: "pkgs".to_string(),
            git_ref: "dev".to_string(),
        }
    );
    assert_eq!(
        plan.platform_packages,
        vec!["github-devloop".to_string(), "github-proxy".to_string()]
    );
}

#[test]
fn plan_clones_rejects_refs_from_two_repos() {
    let refs = refs(&["org/pkgs@dev:packages/a", "org/other@dev:packages/b"]);
    let err = plan_clones(&refs).expect_err("multi-workspace must fail");
    assert!(err.contains("one workspace repo"), "{err}");
}

#[test]
fn plan_clones_rejects_refs_at_two_git_refs() {
    let refs = refs(&["org/pkgs@dev:packages/a", "org/pkgs@main:packages/b"]);
    assert!(plan_clones(&refs).is_err(), "differing git_ref must fail");
}

#[test]
fn build_supervise_args_is_the_exact_vector() {
    let args = build_supervise_args(
        "/rt/project",
        "/rt/platform",
        &["github-devloop".to_string(), "github-proxy".to_string()],
        "/var/run/fkst/durable",
        "/var/run/fkst/runtime",
        "/usr/local/bin/fkst-framework",
    );
    assert_eq!(
        args,
        vec![
            "supervise",
            "--project-root",
            "/rt/project",
            "--platform-root",
            "/rt/platform",
            "--platform-packages",
            "github-devloop github-proxy",
            "--durable-root",
            "/var/run/fkst/durable",
            "--runtime-root",
            "/var/run/fkst/runtime",
            "--framework-bin",
            "/usr/local/bin/fkst-framework",
        ]
    );
}

fn find<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[test]
fn substrate_child_env_wires_platform_vars_and_git_config() {
    let base = vec![("PATH".to_string(), "/usr/bin".to_string())];
    let entries = vec![
        GitConfigEntry {
            key: "credential.https://github.com.helper".to_string(),
            value: "!/rt/gitcred/git-credential-fkst".to_string(),
        },
        GitConfigEntry {
            key: "credential.https://github.com.useHttpPath".to_string(),
            value: "false".to_string(),
        },
    ];
    let env = substrate_child_env(
        base,
        &BTreeMap::new(),
        "sk-secret",
        &entries,
        "/rt/codex",
        "/var/run/fkst/durable",
        "/var/run/fkst/runtime",
    );
    assert_eq!(find(&env, "PATH"), Some("/usr/bin"));
    assert_eq!(find(&env, "LLM_API_KEY"), Some("sk-secret"));
    assert_eq!(find(&env, "CODEX_HOME"), Some("/rt/codex"));
    assert_eq!(
        find(&env, "FKST_DURABLE_ROOT"),
        Some("/var/run/fkst/durable")
    );
    assert_eq!(
        find(&env, "FKST_RUNTIME_ROOT"),
        Some("/var/run/fkst/runtime")
    );
    assert_eq!(find(&env, "GIT_CONFIG_COUNT"), Some("2"));
    assert_eq!(
        find(&env, "GIT_CONFIG_KEY_0"),
        Some("credential.https://github.com.helper")
    );
    assert_eq!(find(&env, "GIT_CONFIG_VALUE_1"), Some("false"));
}

#[test]
fn substrate_child_env_folds_user_env_but_drops_reserved_keys() {
    let user_env = BTreeMap::from([
        // Allowed: an ordinary user key survives.
        ("MY_TOOL_TOKEN".to_string(), "ok".to_string()),
        // Dropped: FKST_-prefixed (reserved by prefix).
        ("FKST_DURABLE_ROOT".to_string(), "/evil".to_string()),
        // Dropped: a git-cred var (reserved by name).
        ("GIT_CONFIG_COUNT".to_string(), "999".to_string()),
        // Dropped: LLM_API_KEY — not in the reserved table, but the platform
        // writes it LAST so a user value can never win.
        ("LLM_API_KEY".to_string(), "sk-attacker".to_string()),
        // Dropped: an allow-listed host var.
        ("PATH".to_string(), "/evil/bin".to_string()),
    ]);
    let base = vec![("PATH".to_string(), "/usr/bin".to_string())];
    let env = substrate_child_env(base, &user_env, "sk-real", &[], "/rt/codex", "/d", "/r");
    assert_eq!(find(&env, "MY_TOOL_TOKEN"), Some("ok"));
    // Platform values win / user values are dropped.
    assert_eq!(find(&env, "FKST_DURABLE_ROOT"), Some("/d"));
    assert_eq!(find(&env, "GIT_CONFIG_COUNT"), Some("0"));
    assert_eq!(find(&env, "LLM_API_KEY"), Some("sk-real"));
    assert_eq!(find(&env, "PATH"), Some("/usr/bin"));
}

#[test]
fn exit_status_to_code_maps_dispositions() {
    assert_eq!(exit_status_to_code(Some(0)), 0);
    assert_eq!(exit_status_to_code(Some(1)), 1);
    assert_eq!(exit_status_to_code(Some(137)), 137);
    // A signal-kill (no code) is a failure.
    assert_eq!(exit_status_to_code(None), 1);
    // A non-zero code that truncates to a zero byte is forced to 1 so a
    // failure never surfaces as success.
    assert_eq!(exit_status_to_code(Some(256)), 1);
}
