use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

fn directory_snapshot(path: &Path) -> Vec<OsString> {
    let mut entries = fs::read_dir(path)
        .expect("temporary working directory must be readable")
        .map(|entry| {
            entry
                .expect("temporary working directory entry must be readable")
                .file_name()
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

#[test]
fn zero_argument_startup_fails_closed_without_side_effects() {
    let working_directory = TempDirectory::new();
    let before = directory_snapshot(working_directory.path());

    let output = Command::new(env!("CARGO_BIN_EXE_fkst-local-qa-host"))
        .current_dir(working_directory.path())
        .output()
        .expect("fkst-local-qa-host must execute");

    let after = directory_snapshot(working_directory.path());

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"fkst-local-qa-host: no supported configuration\n"
    );
    assert!(before.is_empty());
    assert_eq!(after, before);
}
