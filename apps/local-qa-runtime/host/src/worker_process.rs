use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use fkst_qa_contracts::{LocalWorkerFrameDecoder, ValidatedValue};
use nix::errno::Errno;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;

use crate::RunError;

const IO_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CLEANUP_GRACE: Duration = Duration::from_millis(500);
const CLEANUP_LIMIT: Duration = Duration::from_secs(3);
const READER_JOIN_LIMIT: Duration = Duration::from_secs(1);
const MAX_STDOUT_BYTES: usize = 1_048_576;
const MAX_STDERR_BYTES: usize = 1_024;

enum ReaderEvent {
    Data(Vec<u8>),
    Eof,
    Failed,
    Overflow,
}

pub(crate) struct WorkerProcess {
    child: Child,
    process_group: Pid,
    stdin: Option<ChildStdin>,
    stdout: Receiver<ReaderEvent>,
    stdout_reader: Option<JoinHandle<()>>,
    stderr: Receiver<Result<Vec<u8>, ()>>,
    stderr_reader: Option<JoinHandle<()>>,
    reaped: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct WorkerControlHandle {
    process_group: Pid,
}

impl WorkerControlHandle {
    pub(crate) fn request_stop(self) -> Result<(), RunError> {
        signal_group(self.process_group, Signal::SIGTERM)
            .map_err(|_| RunError::Contract("Browser Worker control signal failed"))
    }

    pub(crate) fn identity(self) -> String {
        format!("worker-pgid:{}", self.process_group.as_raw())
    }
}

impl WorkerProcess {
    pub(crate) fn spawn(node: &Path, worker: &Path) -> Result<Self, RunError> {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(node);
        command
            .arg(worker)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = command
            .spawn()
            .map_err(|_| RunError::Contract("Browser Worker spawn failed"))?;
        let process_group = Pid::from_raw(child.id() as i32);
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                terminate_spawned_child(&mut child, process_group);
                return Err(RunError::Contract("Browser Worker stdin unavailable"));
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_spawned_child(&mut child, process_group);
                return Err(RunError::Contract("Browser Worker stdout unavailable"));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_spawned_child(&mut child, process_group);
                return Err(RunError::Contract("Browser Worker stderr unavailable"));
            }
        };
        let (stdout_sender, stdout_receiver) = mpsc::channel();
        let stdout_reader = match thread::Builder::new()
            .name("local-qa-worker-stdout".to_owned())
            .spawn(move || read_stdout(stdout, stdout_sender))
        {
            Ok(reader) => reader,
            Err(error) => {
                terminate_spawned_child(&mut child, process_group);
                return Err(RunError::Io(error));
            }
        };
        let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
        let stderr_reader = match thread::Builder::new()
            .name("local-qa-worker-stderr".to_owned())
            .spawn(move || {
                let _ = stderr_sender.send(read_stderr(stderr));
            }) {
            Ok(reader) => reader,
            Err(error) => {
                terminate_spawned_child(&mut child, process_group);
                let _ = join_bounded(stdout_reader, Instant::now() + READER_JOIN_LIMIT);
                return Err(RunError::Io(error));
            }
        };
        Ok(Self {
            child,
            process_group,
            stdin: Some(stdin),
            stdout: stdout_receiver,
            stdout_reader: Some(stdout_reader),
            stderr: stderr_receiver,
            stderr_reader: Some(stderr_reader),
            reaped: false,
        })
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<(), RunError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or(RunError::Contract("Browser Worker stdin is closed"))?;
        stdin
            .write_all(bytes)
            .and_then(|()| stdin.flush())
            .map_err(|_| RunError::Contract("Browser Worker stdin write failed"))
    }

    pub(crate) fn control_handle(&self) -> WorkerControlHandle {
        WorkerControlHandle {
            process_group: self.process_group,
        }
    }

    pub(crate) fn close_stdin(&mut self) -> Result<(), RunError> {
        self.stdin
            .take()
            .ok_or(RunError::Contract("Browser Worker stdin already closed"))?;
        Ok(())
    }

    pub(crate) fn read_frame(
        &mut self,
        decoder: &mut LocalWorkerFrameDecoder,
        deadline: Instant,
    ) -> Result<ValidatedValue, RunError> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RunError::Contract("Browser Worker timed out"));
            }
            match self.stdout.recv_timeout(remaining) {
                Ok(ReaderEvent::Data(bytes)) => {
                    let frames = decoder
                        .push(&bytes)
                        .map_err(|_| RunError::Contract("malformed Browser Worker frame"))?;
                    match frames.len() {
                        0 => {}
                        1 => return Ok(frames.into_iter().next().expect("one decoded frame")),
                        _ => {
                            return Err(RunError::Contract(
                                "unexpected Browser Worker frame sequence",
                            ))
                        }
                    }
                }
                Ok(ReaderEvent::Eof) => {
                    decoder
                        .finish()
                        .map_err(|_| RunError::Contract("truncated Browser Worker frame"))?;
                    return Err(RunError::Contract("unexpected Browser Worker EOF"));
                }
                Ok(ReaderEvent::Failed) | Err(_) => {
                    return Err(RunError::Contract("Browser Worker stdout failed"))
                }
                Ok(ReaderEvent::Overflow) => {
                    return Err(RunError::Contract("Browser Worker stdout exceeded limit"))
                }
            }
        }
    }

    pub(crate) fn require_clean_eof(
        &mut self,
        decoder: &mut LocalWorkerFrameDecoder,
        deadline: Instant,
    ) -> Result<(), RunError> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RunError::Contract("Browser Worker EOF timed out"));
            }
            match self.stdout.recv_timeout(remaining) {
                Ok(ReaderEvent::Data(bytes)) => {
                    if !decoder
                        .push(&bytes)
                        .map_err(|_| RunError::Contract("malformed trailing Worker frame"))?
                        .is_empty()
                    {
                        return Err(RunError::Contract("trailing Browser Worker frame"));
                    }
                }
                Ok(ReaderEvent::Eof) => {
                    return decoder
                        .finish()
                        .map_err(|_| RunError::Contract("truncated trailing Worker frame"));
                }
                Ok(ReaderEvent::Failed) | Err(_) => {
                    return Err(RunError::Contract("Browser Worker stdout failed"))
                }
                Ok(ReaderEvent::Overflow) => {
                    return Err(RunError::Contract("Browser Worker stdout exceeded limit"))
                }
            }
        }
    }

    pub(crate) fn wait_success(&mut self, deadline: Instant) -> Result<(), RunError> {
        let status = self.wait_for_root(deadline)?;
        let group_cleanup = self.cleanup_process_group();
        let readers = self.join_readers(status);
        group_cleanup?;
        readers
    }

    fn wait_for_root(&mut self, deadline: Instant) -> Result<ExitStatus, RunError> {
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|_| RunError::Contract("Browser Worker status failed"))?
            {
                self.reaped = true;
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(RunError::Contract("Browser Worker exit timed out"));
            }
            thread::sleep(IO_POLL_INTERVAL);
        }
    }

    fn cleanup_process_group(&mut self) -> Result<(), RunError> {
        signal_group(self.process_group, Signal::SIGTERM)
            .map_err(|_| RunError::Contract("Browser Worker group termination failed"))?;
        if !wait_for_process_group_exit(self.process_group, CLEANUP_GRACE)? {
            signal_group(self.process_group, Signal::SIGKILL)
                .map_err(|_| RunError::Contract("Browser Worker group kill failed"))?;
        }
        if wait_for_process_group_exit(self.process_group, CLEANUP_LIMIT)? {
            Ok(())
        } else {
            Err(RunError::Contract(
                "Browser Worker process group still has live members",
            ))
        }
    }

    fn join_readers(&mut self, status: ExitStatus) -> Result<(), RunError> {
        let stderr = self
            .stderr
            .recv_timeout(READER_JOIN_LIMIT)
            .map_err(|_| RunError::Contract("Browser Worker stderr reader timed out"))?
            .map_err(|_| RunError::Contract("Browser Worker stderr failed"))?;
        let join_deadline = Instant::now() + READER_JOIN_LIMIT;
        join_bounded(
            self.stdout_reader
                .take()
                .ok_or(RunError::Contract("Browser Worker stdout reader missing"))?,
            join_deadline,
        )
        .map_err(|_| RunError::Contract("Browser Worker stdout reader did not join"))?;
        join_bounded(
            self.stderr_reader
                .take()
                .ok_or(RunError::Contract("Browser Worker stderr reader missing"))?,
            join_deadline,
        )
        .map_err(|_| RunError::Contract("Browser Worker stderr reader did not join"))?;
        if !status.success() || !stderr.is_empty() {
            return Err(RunError::Contract("Browser Worker did not exit cleanly"));
        }
        Ok(())
    }

    pub(crate) fn terminate(&mut self) {
        self.stdin.take();
        let _ = signal_group(self.process_group, Signal::SIGTERM);
        let grace_deadline = Instant::now() + CLEANUP_GRACE;
        while Instant::now() < grace_deadline {
            self.try_reap_root();
            if !process_group_is_alive(self.process_group).unwrap_or(true) {
                break;
            }
            thread::sleep(IO_POLL_INTERVAL);
        }
        if process_group_is_alive(self.process_group).unwrap_or(true) {
            let _ = signal_group(self.process_group, Signal::SIGKILL);
        }
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.reaped = true;
        }
        let _ = wait_for_process_group_exit(self.process_group, CLEANUP_LIMIT);
        self.join_reader_fallbacks();
    }

    fn try_reap_root(&mut self) {
        if !self.reaped && self.child.try_wait().ok().flatten().is_some() {
            self.reaped = true;
        }
    }

    fn join_reader_fallbacks(&mut self) {
        let deadline = Instant::now() + READER_JOIN_LIMIT;
        if let Some(reader) = self.stdout_reader.take() {
            let _ = join_bounded(reader, deadline);
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = join_bounded(reader, deadline);
        }
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        if !self.reaped || process_group_is_alive(self.process_group).unwrap_or(true) {
            self.terminate();
        } else {
            self.join_reader_fallbacks();
        }
    }
}

fn read_stdout(mut stdout: impl Read, sender: Sender<ReaderEvent>) {
    let mut total = 0_usize;
    let mut chunk = [0_u8; 4096];
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) => {
                let _ = sender.send(ReaderEvent::Eof);
                return;
            }
            Ok(read) => {
                total = total.saturating_add(read);
                if total > MAX_STDOUT_BYTES {
                    let _ = sender.send(ReaderEvent::Overflow);
                    return;
                }
                if sender
                    .send(ReaderEvent::Data(chunk[..read].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            Err(_) => {
                let _ = sender.send(ReaderEvent::Failed);
                return;
            }
        }
    }
}

fn read_stderr(stderr: impl Read) -> Result<Vec<u8>, ()> {
    let mut bytes = Vec::new();
    stderr
        .take((MAX_STDERR_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() > MAX_STDERR_BYTES {
        Err(())
    } else {
        Ok(bytes)
    }
}

fn join_bounded<T>(handle: JoinHandle<T>, deadline: Instant) -> Result<T, ()> {
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return Err(());
        }
        thread::sleep(IO_POLL_INTERVAL);
    }
    handle.join().map_err(|_| ())
}

fn wait_for_process_group_exit(process_group: Pid, limit: Duration) -> Result<bool, RunError> {
    let deadline = Instant::now() + limit;
    loop {
        if !process_group_is_alive(process_group)? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(IO_POLL_INTERVAL);
    }
}

#[cfg(target_os = "linux")]
fn process_group_is_alive(process_group: Pid) -> Result<bool, RunError> {
    let entries = std::fs::read_dir("/proc")
        .map_err(|_| RunError::Contract("Browser Worker process group inspection failed"))?;
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some(after_name) = stat.rsplit_once(')').map(|(_, rest)| rest.trim()) else {
            continue;
        };
        let mut fields = after_name.split_whitespace();
        let state = fields.next();
        let _parent_pid = fields.next();
        let group = fields.next().and_then(|value| value.parse::<i32>().ok());
        if state != Some("Z") && group == Some(process_group.as_raw()) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
fn process_group_is_alive(process_group: Pid) -> Result<bool, RunError> {
    let target = Pid::from_raw(-process_group.as_raw());
    match nix::sys::signal::kill(target, None) {
        Ok(()) | Err(Errno::EPERM) => Ok(true),
        Err(Errno::ESRCH) => Ok(false),
        Err(_) => Err(RunError::Contract(
            "Browser Worker process group inspection failed",
        )),
    }
}

fn terminate_spawned_child(child: &mut Child, process_group: Pid) {
    let _ = signal_group(process_group, Signal::SIGKILL);
    let _ = child.kill();
    let _ = child.wait();
    let _ = wait_for_process_group_exit(process_group, CLEANUP_LIMIT);
}

fn signal_group(process_group: Pid, signal: Signal) -> Result<(), Errno> {
    match killpg(process_group, signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    use fkst_qa_contracts::{encode_local_worker_frame, validate_local_worker_frame};
    use serde_json::json;

    use super::*;

    #[test]
    fn clean_worker_exit_without_a_frame_is_rejected_and_reaped() {
        let directory = temporary_directory("worker-no-frame");
        let script = directory.join("worker.sh");
        write_output_script(&script, &[]);

        let mut process = WorkerProcess::spawn(&script, &script).expect("Worker process starts");
        let process_group = process.process_group;
        let mut decoder = LocalWorkerFrameDecoder::default();
        let error = process
            .read_frame(&mut decoder, Instant::now() + Duration::from_secs(2))
            .expect_err("a Worker without a frame is rejected");
        assert!(
            error.to_string().contains("unexpected Browser Worker EOF"),
            "unexpected no-frame error: {error}"
        );
        process.terminate();
        assert!(!process_group_is_alive(process_group).expect("group inspection succeeds"));
        fs::remove_dir_all(directory).expect("temporary Worker directory removes");
    }

    #[test]
    fn malformed_worker_frame_is_rejected_and_reaped() {
        let directory = temporary_directory("worker-malformed-frame");
        let script = directory.join("worker.sh");
        write_output_script(&script, &[0, 0, 0, 1, b'X']);

        let mut process = WorkerProcess::spawn(&script, &script).expect("Worker process starts");
        let process_group = process.process_group;
        let mut decoder = LocalWorkerFrameDecoder::default();
        let error = process
            .read_frame(&mut decoder, Instant::now() + Duration::from_secs(2))
            .expect_err("malformed Worker frame is rejected");
        assert!(error.to_string().contains("malformed Browser Worker frame"));
        process.terminate();
        assert!(!process_group_is_alive(process_group).expect("group inspection succeeds"));
        fs::remove_dir_all(directory).expect("temporary Worker directory removes");
    }

    #[test]
    fn trailing_worker_frame_is_rejected_after_the_first_frame() {
        let directory = temporary_directory("worker-trailing-frame");
        let script = directory.join("worker.sh");
        let value = validate_local_worker_frame(
            &serde_json::to_vec(&json!({
                "protocol": "qa.local-worker-protocol/v1",
                "kind": "invocation",
                "invocation_id": "invocation/0",
                "operation": "browser-smoke",
                "input": {
                    "version": "local-qa-browser-smoke/request-v1",
                    "fixtureUrl": "http://127.0.0.1:43123/fixed-page.html",
                    "selector": r#"[data-local-qa="status"]"#,
                    "expectedText": "READY",
                    "timeoutMs": 5000
                }
            }))
            .expect("valid invocation serializes"),
        )
        .expect("valid invocation validates");
        let mut output = encode_local_worker_frame(&value).expect("invocation encodes");
        output.extend_from_slice(&encode_local_worker_frame(&value).expect("second frame encodes"));
        write_output_script(&script, &output);

        let mut process = WorkerProcess::spawn(&script, &script).expect("Worker process starts");
        let process_group = process.process_group;
        let mut decoder = LocalWorkerFrameDecoder::default();
        let first = process.read_frame(&mut decoder, Instant::now() + Duration::from_secs(2));
        if first.is_ok() {
            let error = process
                .require_clean_eof(&mut decoder, Instant::now() + Duration::from_secs(2))
                .expect_err("trailing Worker frame is rejected");
            assert!(error.to_string().contains("trailing Browser Worker frame"));
        } else {
            assert!(first
                .expect_err("first result is either valid or a coalesced sequence")
                .to_string()
                .contains("unexpected Browser Worker frame sequence"));
        }
        process.terminate();
        assert!(!process_group_is_alive(process_group).expect("group inspection succeeds"));
        fs::remove_dir_all(directory).expect("temporary Worker directory removes");
    }

    #[test]
    fn successful_root_exit_still_escalates_and_reaps_the_owned_group() {
        let directory = temporary_directory("worker-descendant");
        let script = directory.join("worker.sh");
        fs::write(
            &script,
            b"#!/bin/sh\n( trap '' TERM; while :; do sleep 1; done ) &\nexit 0\n",
        )
        .expect("hostile Worker script writes");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("hostile Worker script is executable");

        let mut process = WorkerProcess::spawn(&script, &script).expect("Worker process starts");
        let process_group = process.process_group;
        let started = Instant::now();
        process
            .wait_success(Instant::now() + Duration::from_secs(5))
            .expect("root success is retained after descendant cleanup");

        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!process_group_is_alive(process_group).expect("owned group inspection succeeds"));
        fs::remove_dir_all(directory).expect("temporary Worker directory removes");
    }

    fn write_output_script(path: &std::path::Path, output: &[u8]) {
        let escaped = output
            .iter()
            .map(|byte| format!("\\{byte:03o}"))
            .collect::<String>();
        fs::write(path, format!("#!/bin/sh\nprintf '{escaped}'\n"))
            .expect("Worker output script writes");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("Worker output script is executable");
    }

    fn temporary_directory(label: &str) -> std::path::PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock follows Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "fkst-local-qa-host-{label}-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("temporary Worker directory creates");
        directory
    }
}
