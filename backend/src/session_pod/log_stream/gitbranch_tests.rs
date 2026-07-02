//! Tests for the log-branch bootstrap/commit/push orchestration. Split into a
//! sibling file so `gitbranch.rs` stays under the 500-line module cap. A fake
//! [`GitRunner`] records the exact call sequence and programs outcomes, so the git
//! sequence is validated without a real repo (the subprocess wiring is checked
//! on-cluster).

use super::*;

use std::cell::{Cell, RefCell};

/// A fake runner: records every call and returns programmed outcomes keyed off the
/// args (branch existence, a first-push failure to exercise the rebase retry).
struct FakeRunner {
    calls: RefCell<Vec<Vec<String>>>,
    branch_exists: bool,
    push_fails_first: Cell<bool>,
    dirty: bool,
}

impl FakeRunner {
    fn new(branch_exists: bool) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            branch_exists,
            push_fails_first: Cell::new(false),
            dirty: true,
        }
    }

    fn joined(&self) -> Vec<String> {
        self.calls.borrow().iter().map(|c| c.join(" ")).collect()
    }
}

impl GitRunner for FakeRunner {
    fn run(&self, args: &[&str]) -> GitOutcome {
        self.calls
            .borrow_mut()
            .push(args.iter().map(|s| s.to_string()).collect());
        let joined = args.join(" ");
        if joined.contains("ls-remote") {
            let stdout = if self.branch_exists {
                "abc123\trefs/heads/fkst-logs/issue-7\n".to_string()
            } else {
                String::new()
            };
            return GitOutcome {
                ok: true,
                stdout,
                stderr: String::new(),
            };
        }
        if joined.contains("status --porcelain") {
            let stdout = if self.dirty {
                " A instances/i/driver.log\n".to_string()
            } else {
                String::new()
            };
            return GitOutcome {
                ok: true,
                stdout,
                stderr: String::new(),
            };
        }
        if args.contains(&"push") && self.push_fails_first.get() {
            self.push_fails_first.set(false);
            return GitOutcome {
                ok: false,
                stdout: String::new(),
                stderr: "non-fast-forward".to_string(),
            };
        }
        GitOutcome {
            ok: true,
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

fn branch() -> String {
    "fkst-logs/issue-7".to_string()
}

#[test]
fn bootstrap_creates_an_orphan_branch_when_absent() {
    let dir = tempfile::tempdir().expect("dir");
    let mut lb = LogBranch::new(
        FakeRunner::new(false),
        dir.path().join("wt"),
        branch(),
        "inst-1".to_string(),
    );
    lb.bootstrap("https://github.com/acme/site.git", "README", "{}\n")
        .expect("bootstrap");

    assert!(lb.is_bootstrapped());
    let calls = lb.runner.joined();
    // Orphan path: init, remote, ls-remote (empty), checkout --orphan, add + commit README.
    assert!(
        calls.iter().any(|c| c.starts_with("init")),
        "init: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("checkout --orphan fkst-logs/issue-7")),
        "orphan checkout: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("commit -q -m chore(logs): initialize")),
        "README commit: {calls:?}"
    );
    // The README + meta.json landed on disk.
    assert!(dir.path().join("wt/README.md").exists());
    assert!(dir.path().join("wt/instances/inst-1/meta.json").exists());
}

#[test]
fn bootstrap_fetches_the_existing_branch_on_revival() {
    let dir = tempfile::tempdir().expect("dir");
    let mut lb = LogBranch::new(
        FakeRunner::new(true),
        dir.path().join("wt"),
        branch(),
        "inst-2".to_string(),
    );
    lb.bootstrap("https://github.com/acme/site.git", "README", "{}\n")
        .expect("bootstrap");

    let calls = lb.runner.joined();
    // Revival path: fetch + checkout -B, and NO orphan checkout, NO README commit
    // (the existing branch already has them — old instances are preserved).
    assert!(
        calls
            .iter()
            .any(|c| c.contains("fetch --depth=1 origin fkst-logs/issue-7")),
        "fetch: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("checkout -B fkst-logs/issue-7 FETCH_HEAD")),
        "checkout existing: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("--orphan")),
        "must not orphan an existing branch: {calls:?}"
    );
    // The new instance dir + meta still get created (append, never rewrite).
    assert!(dir.path().join("wt/instances/inst-2/meta.json").exists());
}

#[test]
fn flush_adds_commits_and_pushes_the_instance_dir() {
    let dir = tempfile::tempdir().expect("dir");
    let mut lb = LogBranch::new(
        FakeRunner::new(false),
        dir.path().join("wt"),
        branch(),
        "inst-3".to_string(),
    );
    lb.bootstrap("https://github.com/acme/site.git", "README", "{}\n")
        .expect("bootstrap");
    lb.runner.calls.borrow_mut().clear();

    let pushed = lb.flush(1).expect("flush");
    assert!(pushed, "a dirty tree pushes a commit");

    let calls = lb.runner.joined();
    assert_eq!(
        calls[0], "add instances/inst-3",
        "stages only this instance dir"
    );
    assert!(calls.iter().any(|c| c == "status --porcelain"));
    assert!(
        calls
            .iter()
            .any(|c| c == "commit -q -m chore(logs): inst-3 flush 1"),
        "flush commit message carries the instance + seq: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("push origin fkst-logs/issue-7")),
        "pushes the branch: {calls:?}"
    );
}

#[test]
fn flush_retries_after_a_non_fast_forward_push() {
    let dir = tempfile::tempdir().expect("dir");
    let mut lb = LogBranch::new(
        FakeRunner::new(false),
        dir.path().join("wt"),
        branch(),
        "inst-4".to_string(),
    );
    lb.bootstrap("https://github.com/acme/site.git", "README", "{}\n")
        .expect("bootstrap");
    lb.runner.push_fails_first.set(true);
    lb.runner.calls.borrow_mut().clear();

    let pushed = lb.flush(2).expect("flush recovers via rebase");
    assert!(pushed);
    let calls = lb.runner.joined();
    // First push fails → fetch + rebase → second push succeeds.
    let push_count = calls.iter().filter(|c| c.contains("push origin")).count();
    assert_eq!(push_count, 2, "one failed + one retried push: {calls:?}");
    assert!(calls
        .iter()
        .any(|c| c.contains("fetch origin fkst-logs/issue-7")));
    assert!(calls
        .iter()
        .any(|c| c.contains("rebase origin/fkst-logs/issue-7")));
}

#[test]
fn flush_before_bootstrap_is_an_error() {
    let dir = tempfile::tempdir().expect("dir");
    let mut lb = LogBranch::new(
        FakeRunner::new(false),
        dir.path().join("wt"),
        branch(),
        "inst-5".to_string(),
    );
    assert!(lb.flush(1).is_err(), "flush must refuse before bootstrap");
}
