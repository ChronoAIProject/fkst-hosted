use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const EXPECTED_STDERR: &[u8] = b"fkst-local-qa-host: no supported configuration\n";

#[derive(Debug, PartialEq, Eq)]
enum EntryKind {
    Directory,
    File(Vec<u8>),
}

#[derive(Debug, PartialEq, Eq)]
struct EntrySnapshot {
    relative_path: PathBuf,
    kind: EntryKind,
}

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new() -> Self {
        let sequence = TEMP_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "fkst-local-qa-host-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("temporary working directory must be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("temporary working directory must be removed");
    }
}

fn write_configuration_decoys(path: &Path) {
    fs::write(path.join(".env"), b"decoy environment bytes\n").expect(".env decoy must be written");
    fs::write(
        path.join(".fkst-local-qa-host.toml"),
        b"decoy hidden host configuration bytes\n",
    )
    .expect("hidden host configuration decoy must be written");
    fs::write(
        path.join("config.toml"),
        b"decoy generic configuration bytes\n",
    )
    .expect("generic configuration decoy must be written");
    fs::write(
        path.join("fkst-local-qa-host.toml"),
        b"decoy host configuration bytes\n",
    )
    .expect("host configuration decoy must be written");
    fs::create_dir(path.join("state")).expect("state decoy directory must be created");
    fs::write(path.join("state/sentinel"), b"decoy state sentinel bytes\n")
        .expect("state sentinel decoy must be written");
}

fn directory_snapshot(root: &Path) -> Vec<EntrySnapshot> {
    fn visit(root: &Path, directory: &Path, entries: &mut Vec<EntrySnapshot>) {
        for entry in fs::read_dir(directory).expect("snapshot directory must be readable") {
            let entry = entry.expect("snapshot entry must be readable");
            let path = entry.path();
            let relative_path = path
                .strip_prefix(root)
                .expect("snapshot entry must be inside the root")
                .to_path_buf();
            let file_type = entry
                .file_type()
                .expect("snapshot entry type must be readable");

            if file_type.is_dir() {
                entries.push(EntrySnapshot {
                    relative_path,
                    kind: EntryKind::Directory,
                });
                visit(root, &path, entries);
            } else if file_type.is_file() {
                entries.push(EntrySnapshot {
                    relative_path,
                    kind: EntryKind::File(
                        fs::read(&path).expect("snapshot file contents must be readable"),
                    ),
                });
            } else {
                panic!("snapshot contains an unsupported entry type: {relative_path:?}");
            }
        }
    }

    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    entries
}

#[test]
fn unsupported_inputs_fail_closed_without_filesystem_side_effects() {
    let cases: &[(&[&str], bool)] = &[
        (&[], true),
        (&["--help"], false),
        (&["--config", "config.toml"], false),
        (&["project"], false),
        (
            &[
                "local-demo",
                "--listen",
                "0.0.0.0:0",
                "--database",
                "state/non-loopback.sqlite",
            ],
            false,
        ),
    ];

    for (arguments, clear_environment) in cases {
        let working_directory = TempDirectory::new();
        write_configuration_decoys(working_directory.path());
        let before = directory_snapshot(working_directory.path());

        let mut command = Command::new(env!("CARGO_BIN_EXE_fkst-local-qa-host"));
        command
            .current_dir(working_directory.path())
            .args(*arguments);
        if *clear_environment {
            command.env_clear();
        }

        let output = command.output().expect("fkst-local-qa-host must execute");
        let after = directory_snapshot(working_directory.path());

        assert_eq!(output.status.code(), Some(1), "arguments: {arguments:?}");
        assert!(output.stdout.is_empty(), "arguments: {arguments:?}");
        assert_eq!(output.stderr, EXPECTED_STDERR, "arguments: {arguments:?}");
        assert_eq!(after, before, "arguments: {arguments:?}");
    }
}
