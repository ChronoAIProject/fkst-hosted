use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use fkst_qa_contracts::{LocalWorkerFrameDecoder, ValidatedValue};
use nix::errno::Errno;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;

use crate::RunError;

const IO_POLL_INTERVAL: Duration = Duration::from_millis(10);
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
    stderr_reader: Option<JoinHandle<Result<Vec<u8>, ()>>>,
    reaped: bool,
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
        let stderr_reader = match thread::Builder::new()
            .name("local-qa-worker-stderr".to_owned())
            .spawn(move || read_stderr(stderr))
        {
            Ok(reader) => reader,
            Err(error) => {
                terminate_spawned_child(&mut child, process_group);
                let _ = stdout_reader.join();
                return Err(RunError::Io(error));
            }
        };
        Ok(Self {
            child,
            process_group,
            stdin: Some(stdin),
            stdout: stdout_receiver,
            stdout_reader: Some(stdout_reader),
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

    pub(crate) fn close_stdin(&mut self) -> Result<(), RunError> {
        self.stdin
            .take()
            .ok_or(RunError::Contract("Browser Worker stdin already closed"))?;
        Ok(())
    }

    pub(crate) fn read_frame(
        &mut self,
        decoder: &mut LocalWorkerFrameDecoder,
        pending: &mut VecDeque<ValidatedValue>,
        deadline: Instant,
    ) -> Result<ValidatedValue, RunError> {
        if let Some(frame) = pending.pop_front() {
            return Ok(frame);
        }
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
                    if frames.len() > 1 || (!frames.is_empty() && !pending.is_empty()) {
                        return Err(RunError::Contract(
                            "unexpected Browser Worker frame sequence",
                        ));
                    }
                    pending.extend(frames);
                    if let Some(frame) = pending.pop_front() {
                        return Ok(frame);
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
        pending: &mut VecDeque<ValidatedValue>,
        deadline: Instant,
    ) -> Result<(), RunError> {
        if !pending.is_empty() {
            return Err(RunError::Contract("trailing Browser Worker frame"));
        }
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
        let status = loop {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|_| RunError::Contract("Browser Worker status failed"))?
            {
                break status;
            }
            if Instant::now() >= deadline {
                return Err(RunError::Contract("Browser Worker exit timed out"));
            }
            thread::sleep(IO_POLL_INTERVAL);
        };
        self.reaped = true;
        self.join_readers(status)
    }

    fn join_readers(&mut self, status: ExitStatus) -> Result<(), RunError> {
        if self
            .stdout_reader
            .take()
            .ok_or(RunError::Contract("Browser Worker stdout reader missing"))?
            .join()
            .is_err()
        {
            return Err(RunError::Contract("Browser Worker stdout reader panicked"));
        }
        let stderr = self
            .stderr_reader
            .take()
            .ok_or(RunError::Contract("Browser Worker stderr reader missing"))?
            .join()
            .map_err(|_| RunError::Contract("Browser Worker stderr reader panicked"))?
            .map_err(|_| RunError::Contract("Browser Worker stderr failed"))?;
        if !status.success() || !stderr.is_empty() {
            return Err(RunError::Contract("Browser Worker did not exit cleanly"));
        }
        Ok(())
    }

    pub(crate) fn terminate(&mut self) {
        if self.reaped {
            self.join_reader_fallbacks();
            return;
        }
        self.stdin.take();
        let _ = signal_group(self.process_group, Signal::SIGTERM);
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                self.reaped = true;
                self.join_reader_fallbacks();
                return;
            }
            thread::sleep(IO_POLL_INTERVAL);
        }
        let _ = signal_group(self.process_group, Signal::SIGKILL);
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.reaped = true;
        self.join_reader_fallbacks();
    }

    fn join_reader_fallbacks(&mut self) {
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        if !self.reaped {
            self.terminate();
        }
    }
}

fn read_stdout(mut stdout: impl Read, sender: mpsc::Sender<ReaderEvent>) {
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

fn terminate_spawned_child(child: &mut Child, process_group: Pid) {
    let _ = signal_group(process_group, Signal::SIGKILL);
    let _ = child.kill();
    let _ = child.wait();
}

fn signal_group(process_group: Pid, signal: Signal) -> Result<(), Errno> {
    match killpg(process_group, signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(error),
    }
}
