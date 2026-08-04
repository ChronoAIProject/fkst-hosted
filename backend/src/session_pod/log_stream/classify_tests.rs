//! Tests for the pure log-tree classification + source discovery. Split into a
//! sibling file so `classify.rs` stays well under the 500-line module cap.

use super::*;

fn anchors() -> TreeAnchors {
    TreeAnchors::new(
        Path::new("/var/run/fkst/runtime"),
        Path::new("/var/run/fkst/codex"),
    )
}

#[test]
fn every_class_has_a_distinct_stable_relative_path() {
    let paths: Vec<&str> = LogClass::ALL.iter().map(|c| c.relative_path()).collect();
    assert_eq!(
        paths,
        vec![
            "fkst-hosted/driver.log",
            "fkst-substrate/framework/supervise.log",
            "fkst-substrate/codex/codex.log",
            "fkst-substrate/etc/misc.log",
        ]
    );
    // No two classes may collide onto one file.
    let mut sorted = paths.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), LogClass::ALL.len());
}

#[test]
fn anchors_resolve_the_framework_child_and_codex_dirs() {
    let a = anchors();
    assert_eq!(a.logs_dir, Path::new("/var/run/fkst/runtime/logs"));
    assert_eq!(
        a.framework_child_dir,
        Path::new("/var/run/fkst/runtime/logs/framework-child")
    );
    assert_eq!(a.codex_log_dir, Path::new("/var/run/fkst/codex/log"));
}

#[test]
fn framework_child_files_route_to_supervise() {
    let a = anchors();
    let path = a.framework_child_dir.join("codex-abc.log");
    assert_eq!(classify_file(&path, &a), LogClass::Supervise);
}

#[test]
fn codex_log_files_route_to_codex() {
    let a = anchors();
    let path = a.codex_log_dir.join("codex-tui.log");
    assert_eq!(classify_file(&path, &a), LogClass::Codex);
}

#[test]
fn unknown_files_route_to_misc() {
    let a = anchors();
    let path = a.logs_dir.join("stray.log");
    assert_eq!(classify_file(&path, &a), LogClass::Misc);
}

#[test]
fn discover_sources_classifies_each_dir_and_ignores_missing() {
    let runtime = tempfile::tempdir().expect("runtime");
    let codex = tempfile::tempdir().expect("codex");
    let a = TreeAnchors::new(runtime.path(), codex.path());

    std::fs::create_dir_all(&a.framework_child_dir).expect("fc dir");
    std::fs::create_dir_all(&a.codex_log_dir).expect("codex dir");
    std::fs::write(a.framework_child_dir.join("child.log"), b"x").expect("child");
    std::fs::write(a.codex_log_dir.join("codex.log"), b"y").expect("codex");
    std::fs::write(a.logs_dir.join("misc.log"), b"z").expect("misc");
    // A non-.log file must be ignored entirely.
    std::fs::write(a.logs_dir.join("supervise.sock"), b"").expect("sock");

    let mut found = discover_sources(&a);
    found.sort_by(|l, r| l.0.cmp(&r.0));

    let classes: Vec<LogClass> = found.iter().map(|(_, c)| *c).collect();
    assert!(classes.contains(&LogClass::Supervise));
    assert!(classes.contains(&LogClass::Codex));
    assert!(classes.contains(&LogClass::Misc));
    // The socket file is not a *.log, so exactly three sources are discovered.
    assert_eq!(found.len(), 3, "only the three .log files: {found:?}");
}

#[test]
fn discover_sources_is_empty_when_no_dirs_exist() {
    let runtime = tempfile::tempdir().expect("runtime");
    let codex = tempfile::tempdir().expect("codex");
    let a = TreeAnchors::new(runtime.path(), codex.path());
    assert!(discover_sources(&a).is_empty());
}

#[test]
fn the_health_anchor_hangs_off_the_runtime_root() {
    let anchors = anchors();
    assert_eq!(
        anchors.health_dir,
        Path::new("/var/run/fkst/runtime/health"),
        "resolved from FKST_RUNTIME_ROOT, so it is backend-neutral"
    );
}

/// The health dir is a WHOLE-FILE source, so it must stay invisible to the tailing
/// path — otherwise reports would be appended into a class file and their per-report
/// boundaries destroyed.
#[test]
fn the_health_dir_is_never_tailed_as_a_log_source() {
    let dir = tempfile::tempdir().expect("dir");
    let runtime_root = dir.path().join("runtime");
    let anchors = TreeAnchors::new(&runtime_root, &dir.path().join("codex"));
    std::fs::create_dir_all(&anchors.health_dir).expect("health dir");
    std::fs::create_dir_all(&anchors.logs_dir).expect("logs dir");
    std::fs::write(anchors.health_dir.join("report.md"), "r").expect("report");
    // Even a .log inside the health dir must not be tailed: the tail passes only
    // walk the logs and codex anchors.
    std::fs::write(anchors.health_dir.join("stray.log"), "l").expect("stray");
    std::fs::write(anchors.logs_dir.join("real.log"), "l").expect("real log");

    let sources = discover_sources(&anchors);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].0, anchors.logs_dir.join("real.log"));
}

#[test]
fn copied_file_discovery_takes_only_markdown_directly_under_the_health_dir() {
    let dir = tempfile::tempdir().expect("dir");
    let anchors = TreeAnchors::new(&dir.path().join("runtime"), &dir.path().join("codex"));
    std::fs::create_dir_all(anchors.health_dir.join("nested")).expect("dirs");
    std::fs::write(anchors.health_dir.join("b.md"), "b").expect("b");
    std::fs::write(anchors.health_dir.join("a.md"), "a").expect("a");
    std::fs::write(anchors.health_dir.join("notes.txt"), "t").expect("txt");
    std::fs::write(anchors.health_dir.join("nested/deep.md"), "d").expect("deep");

    let files = discover_copied_files(&anchors);
    assert_eq!(
        files,
        vec![
            anchors.health_dir.join("a.md"),
            anchors.health_dir.join("b.md")
        ],
        "markdown only, non-recursive, sorted (= chronological, by the stamp)"
    );
}

#[test]
fn copied_file_discovery_on_an_absent_dir_is_empty_not_an_error() {
    let anchors = TreeAnchors::new(
        Path::new("/nonexistent/runtime"),
        Path::new("/nonexistent/codex"),
    );
    assert!(discover_copied_files(&anchors).is_empty());
}
