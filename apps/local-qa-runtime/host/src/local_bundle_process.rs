//! Fixed Git commands and bounded pipe polling for the trusted local adapter.
use std::ffi::OsString;
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;

use super::{check_time, LocalBundleLimits};
use crate::RunError;

const POLL: Duration = Duration::from_millis(5);
const PROCESS_TABLE_BYTES: usize = 1024 * 1024;

pub(super) enum GitStep<'a> {
    Init,
    Verify,
    Import,
    Fsck,
    ObjectInventory,
    CommitType(&'a str),
    Tree(&'a str),
    Blob(&'a str),
    Index(&'a str),
    Detach(&'a str),
}

pub(super) struct GitRunner {
    executable: PathBuf,
    root: PathBuf,
    private: PathBuf,
    deadline: Instant,
    work_deadline: Instant,
    output_limit: usize,
    remaining_output: usize,
}

impl GitRunner {
    pub(super) fn new(
        executable: &Path,
        root: &Path,
        private: &Path,
        deadline: Instant,
        limits: &LocalBundleLimits,
    ) -> Self {
        let remaining = deadline.saturating_duration_since(Instant::now());
        Self {
            executable: executable.into(),
            root: root.into(),
            private: private.into(),
            deadline,
            work_deadline: deadline - (remaining / 3).min(Duration::from_secs(2)),
            output_limit: limits.max_output_bytes,
            remaining_output: limits.max_total_output_bytes,
        }
    }

    pub(super) fn run(&mut self, step: GitStep<'_>) -> Result<Vec<u8>, RunError> {
        check_time(self.work_deadline)?;
        let mut command = Command::new(&self.executable);
        let option = |prefix: &str, path: &Path| {
            let mut value = OsString::from(prefix);
            value.push(path.as_os_str());
            value
        };
        command
            .env_clear()
            .current_dir(&self.private)
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.private)
            .env("XDG_CONFIG_HOME", &self.private)
            .env("TMPDIR", &self.private)
            .env("LC_ALL", "C")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_ALLOW_PROTOCOL", "")
            .arg("--no-pager")
            .arg("--no-replace-objects")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.attributesFile=/dev/null",
                "-c",
                "protocol.allow=never",
                "-c",
                "credential.helper=",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "gc.auto=0",
                "-c",
                "maintenance.auto=false",
                "-c",
                "core.autocrlf=false",
            ])
            .arg(option("--git-dir=", &self.root.join(".git")))
            .arg(option("--work-tree=", &self.root));
        match step {
            GitStep::Init => {
                command
                    .arg("init")
                    .arg("--quiet")
                    .arg("--object-format=sha1")
                    .arg(option("--template=", &self.private.join("template")))
                    .arg(&self.root);
            }
            GitStep::Verify => {
                command
                    .args(["bundle", "verify"])
                    .arg(self.private.join("input.bundle"));
            }
            GitStep::Import => {
                command
                    .args(["bundle", "unbundle"])
                    .arg(self.private.join("input.bundle"));
            }
            GitStep::Fsck => {
                command.args([
                    "fsck",
                    "--full",
                    "--strict",
                    "--no-reflogs",
                    "--no-progress",
                ]);
            }
            GitStep::ObjectInventory => {
                command.args([
                    "cat-file",
                    "--batch-all-objects",
                    "--batch-check=%(objectname) %(objecttype) %(objectsize)",
                ]);
            }
            GitStep::CommitType(id) => {
                command.args(["cat-file", "-t", id]);
            }
            GitStep::Tree(id) => {
                command.args(["ls-tree", "-r", "-z", "--full-tree", id]);
            }
            GitStep::Blob(id) => {
                command.args(["cat-file", "blob", id]);
            }
            GitStep::Index(id) => {
                command.args(["read-tree", id]);
            }
            GitStep::Detach(id) => {
                command.args(["update-ref", "--no-deref", "HEAD", id]);
            }
        }
        let (status, output) = run_owned(
            command,
            self.work_deadline,
            self.deadline,
            self.output_limit,
            &mut self.remaining_output,
        )?;
        if !status.success() {
            return Err(RunError::Lifecycle(
                "local bundle Git verification command failed",
            ));
        }
        Ok(output)
    }
}

fn nonblocking(pipe: &impl AsFd) -> io::Result<()> {
    let bits = fcntl(pipe, FcntlArg::F_GETFL).map_err(io::Error::from)?;
    fcntl(
        pipe,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(bits) | OFlag::O_NONBLOCK),
    )
    .map_err(io::Error::from)?;
    Ok(())
}

fn drain(
    pipe: &mut impl Read,
    eof: &mut bool,
    bytes: &mut Vec<u8>,
    limit: usize,
    remaining: &mut usize,
) -> io::Result<()> {
    if *eof {
        return Ok(());
    }
    let mut buffer = [0; 8192];
    // A flood cannot starve the other pipe or the deadline check.
    for _ in 0..8 {
        match pipe.read(&mut buffer) {
            Ok(0) => {
                *eof = true;
                break;
            }
            Ok(n) => {
                if n > limit.saturating_sub(bytes.len()) || n > *remaining {
                    return Err(io::Error::other(
                        "local bundle subprocess output limit exceeded",
                    ));
                }
                *remaining -= n;
                bytes.extend_from_slice(&buffer[..n]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

struct OwnedChild {
    child: Child,
    group: Pid,
    reaped: bool,
}
impl OwnedChild {
    fn spawn(mut command: Command) -> io::Result<Self> {
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        let group = Pid::from_raw(child.id() as i32);
        Ok(Self {
            child,
            group,
            reaped: false,
        })
    }
    fn signal(&self) -> io::Result<()> {
        match killpg(self.group, Signal::SIGKILL) {
            Ok(()) | Err(Errno::ESRCH) => Ok(()),
            Err(error) => Err(io::Error::from(error)),
        }
    }
    fn reap(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }
    fn contain(&mut self, deadline: Instant) -> io::Result<()> {
        // Keep the leader unreaped while inspecting/signalling: its PID reserves
        // this PGID, even when descendants outlive it. Never signal after reap.
        let initial = group_live(self.group, deadline);
        let signal_error = match &initial {
            Ok(false) => None,
            _ => self.signal().err(),
        };
        let mut observation = initial;
        while !matches!(observation, Ok(false)) && Instant::now() < deadline {
            thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
            observation = group_live(self.group, deadline);
        }
        let reap = self.reap();
        if let Some(error) = signal_error {
            return Err(error);
        }
        reap?;
        match observation {
            Ok(false) => Ok(()),
            Ok(true) => Err(io::Error::other(
                "local bundle group containment was not established before deadline",
            )),
            Err(error) => Err(error),
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            // Synchronous fallback only; kernel wait/I/O cannot promise wall time.
            let _ = self.signal();
            let _ = self.reap();
        }
    }
}

fn run_owned(
    command: Command,
    work_deadline: Instant,
    deadline: Instant,
    output_limit: usize,
    remaining: &mut usize,
) -> Result<(ExitStatus, Vec<u8>), RunError> {
    let mut child = OwnedChild::spawn(command)?;
    let result = (|| -> Result<Vec<u8>, RunError> {
        let mut stdout = child
            .child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing stdout"))?;
        let mut stderr = child
            .child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("missing stderr"))?;
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (mut out_eof, mut err_eof) = (false, false);
        loop {
            check_time(work_deadline)?;
            drain(&mut stdout, &mut out_eof, &mut out, output_limit, remaining)?;
            drain(&mut stderr, &mut err_eof, &mut err, output_limit, remaining)?;
            if out_eof && err_eof && !group_live(child.group, work_deadline)? {
                return Ok(out);
            }
            thread::sleep(POLL.min(work_deadline.saturating_duration_since(Instant::now())));
        }
    })();
    match result {
        Ok(output) => Ok((child.reap()?, output)),
        Err(error) => {
            let cleanup = child.contain(deadline);
            match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => {
                    Err(io::Error::other(format!("{error}; owned group cleanup: {cleanup}")).into())
                }
            }
        }
    }
}

// ps is a fixed system builtin, with no helper execution. Its own output and
// lifetime are bounded separately; it is never used to select a signal target.
// Looking only at killpg(0) misclassifies macOS zombie-only groups as live.
fn group_live(group: Pid, deadline: Instant) -> io::Result<bool> {
    if Instant::now() >= deadline {
        return Err(io::Error::other("process inspection deadline expired"));
    }
    #[cfg(target_os = "macos")]
    let executable = "/bin/ps";
    #[cfg(not(target_os = "macos"))]
    let executable = "/usr/bin/ps";
    let mut command = Command::new(executable);
    command
        .env_clear()
        .env("LC_ALL", "C")
        .args(["-A", "-o", "pid=,pgid=,stat="]);
    let mut child = OwnedChild::spawn(command)?;
    let mut stdout = child
        .child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing ps stdout"))?;
    let mut stderr = child
        .child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing ps stderr"))?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let (mut out_eof, mut err_eof) = (false, false);
    let mut remaining = PROCESS_TABLE_BYTES;
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::other("process inspection deadline expired"));
        }
        drain(
            &mut stdout,
            &mut out_eof,
            &mut out,
            PROCESS_TABLE_BYTES,
            &mut remaining,
        )?;
        drain(&mut stderr, &mut err_eof, &mut err, 4096, &mut remaining)?;
        // This fixed ps implementation does not fork. No inherited descendant
        // writers exist; try_wait is used only for this exact inspection child.
        if out_eof && err_eof {
            if let Some(status) = child.child.try_wait()? {
                child.reaped = true;
                if !status.success() {
                    return Err(io::Error::other("process inspection failed"));
                }
                break;
            }
        }
        thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
    let text = std::str::from_utf8(&out)
        .map_err(|_| io::Error::other("invalid process inspection output"))?;
    let mut saw_leader = false;
    let mut live = false;
    for line in text.lines() {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() != 3 {
            return Err(io::Error::other("incomplete process inspection output"));
        }
        let pid = columns[0]
            .parse::<i32>()
            .map_err(|_| io::Error::other("invalid process identity"))?;
        let pgid = columns[1]
            .parse::<i32>()
            .map_err(|_| io::Error::other("invalid process group"))?;
        if pid == group.as_raw() && pgid == group.as_raw() {
            saw_leader = true;
        }
        if pgid == group.as_raw() && !columns[2].starts_with('Z') {
            live = true;
        }
    }
    if !saw_leader {
        return Err(io::Error::other(
            "unreaped process group leader absent from inspection",
        ));
    }
    Ok(live)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .args(["-c", script]);
        command
    }
    #[test]
    fn bounded_runner_handles_binary_empty_and_stderr_flood() {
        for script in ["printf '\\000binary'", "exit 0"] {
            let now = Instant::now();
            let mut total = 1000;
            assert!(run_owned(
                shell(script),
                now + Duration::from_secs(2),
                now + Duration::from_secs(3),
                1000,
                &mut total
            )
            .unwrap()
            .0
            .success());
        }
        let now = Instant::now();
        let mut total = 4096;
        let error = run_owned(
            shell("while :; do printf 'stderr flooding' >&2; done"),
            now + Duration::from_secs(2),
            now + Duration::from_secs(3),
            1024,
            &mut total,
        )
        .unwrap_err();
        assert!(error.to_string().contains("output limit"), "{error}");
    }
    #[test]
    fn bounded_runner_contains_early_exit_and_term_resistant_descendants() {
        for script in [
            "trap '' TERM; while :; do :; done",
            "(trap '' TERM; sleep 30) & exit 0",
            "(trap '' TERM; exec 1>&- 2>&-; sleep 30) & exit 0",
            "exec 1>&- 2>&-; sleep 30",
        ] {
            let now = Instant::now();
            let mut total = 4096;
            let error = run_owned(
                shell(script),
                now + Duration::from_millis(100),
                now + Duration::from_secs(3),
                1024,
                &mut total,
            )
            .unwrap_err();
            assert!(!error.to_string().contains("cleanup:"), "{script}: {error}");
            assert!(now.elapsed() < Duration::from_secs(3));
        }
    }
    #[test]
    fn total_output_and_absolute_deadline_are_shared() {
        let now = Instant::now();
        let mut total = 5;
        run_owned(
            shell("printf 1234"),
            now + Duration::from_secs(1),
            now + Duration::from_secs(2),
            100,
            &mut total,
        )
        .unwrap();
        let error = run_owned(
            shell("printf 12"),
            now + Duration::from_secs(1),
            now + Duration::from_secs(2),
            100,
            &mut total,
        )
        .unwrap_err();
        assert!(error.to_string().contains("output limit"));
        let mut total = 100;
        let error = run_owned(
            shell("sleep 1"),
            now,
            now + Duration::from_secs(2),
            100,
            &mut total,
        )
        .unwrap_err();
        assert!(error.to_string().contains("deadline"));
    }
}
