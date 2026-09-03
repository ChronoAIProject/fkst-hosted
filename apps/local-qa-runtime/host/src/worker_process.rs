use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use fkst_qa_contracts::{
    encode_local_worker_frame, validate_local_worker_abort, validate_local_worker_cancel_ack,
    validate_local_worker_control_failure, validate_local_worker_frame, ValidatedValue,
};
use nix::errno::Errno;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use serde_json::{json, Value};

use crate::RunError;

const IO_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CLEANUP_GRACE: Duration = Duration::from_millis(500);
const CLEANUP_LIMIT: Duration = Duration::from_secs(3);
const READER_JOIN_LIMIT: Duration = Duration::from_secs(1);
const MAX_FRAME_BYTES: usize = 65_536;
const MAX_STDOUT_BYTES: usize = 1_048_576;
const MAX_STDERR_BYTES: usize = 1_024;

enum ReaderEvent {
    Data(Vec<u8>),
    Eof,
    Failed,
    Overflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerControlResult {
    Accepted,
    TooLate,
    Failed,
}

enum SessionCommand {
    Write {
        bytes: Vec<u8>,
        response: SyncSender<Result<(), SessionError>>,
    },
    Read {
        deadline: Instant,
        response: SyncSender<Result<ValidatedValue, SessionError>>,
    },
    Abort {
        bytes: Vec<u8>,
        control_id: String,
        invocation_id: String,
        deadline: Instant,
        response: SyncSender<Result<WorkerControlResult, SessionError>>,
    },
    CloseStdin {
        response: SyncSender<Result<(), SessionError>>,
    },
    CleanEof {
        deadline: Instant,
        response: SyncSender<Result<(), SessionError>>,
    },
}

#[derive(Clone, Copy)]
struct SessionError(&'static str);

struct PendingAbort {
    control_id: String,
    invocation_id: String,
    deadline: Instant,
    response: SyncSender<Result<WorkerControlResult, SessionError>>,
}

#[derive(Default)]
struct WireDecoder {
    buffer: Vec<u8>,
}

impl WireDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, SessionError> {
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        let mut offset = 0;
        while self.buffer.len().saturating_sub(offset) >= 4 {
            let length = u32::from_be_bytes(
                self.buffer[offset..offset + 4]
                    .try_into()
                    .expect("four-byte prefix"),
            ) as usize;
            if length == 0 || length > MAX_FRAME_BYTES {
                return Err(SessionError("malformed Browser Worker frame"));
            }
            if self.buffer.len() - offset - 4 < length {
                break;
            }
            frames.push(self.buffer[offset + 4..offset + 4 + length].to_vec());
            offset += 4 + length;
        }
        if offset > 0 {
            self.buffer.drain(..offset);
        }
        Ok(frames)
    }

    fn finish(&self) -> Result<(), SessionError> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(SessionError("truncated Browser Worker frame"))
        }
    }
}

pub(crate) struct WorkerProcess {
    child: Child,
    process_group: Pid,
    session: Option<Sender<SessionCommand>>,
    session_thread: Option<JoinHandle<()>>,
    stdout_reader: Option<JoinHandle<()>>,
    stderr: Receiver<Result<Vec<u8>, ()>>,
    stderr_reader: Option<JoinHandle<()>>,
    reaped: bool,
}

#[derive(Clone)]
pub(crate) struct WorkerControlHandle {
    process_group: Pid,
    session: Sender<SessionCommand>,
}

impl WorkerControlHandle {
    pub(crate) fn request_abort(
        &self,
        control_id: &str,
        invocation_id: &str,
        deadline_utc: &str,
        deadline: Instant,
    ) -> Result<WorkerControlResult, RunError> {
        let abort = validate_local_worker_abort(
            &serde_json::to_vec(&json!({
                "protocol": "qa.local-worker-control/v1",
                "kind": "abort",
                "control_id": control_id,
                "invocation_id": invocation_id,
                "deadline_utc": deadline_utc,
            }))
            .map_err(|_| RunError::Contract("Browser Worker abort serialization failed"))?,
        )
        .map_err(|_| RunError::Contract("invalid Browser Worker abort"))?;
        let bytes = encode_local_worker_frame(&abort)
            .map_err(|_| RunError::Contract("Browser Worker abort framing failed"))?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let containment_reserve = remaining.min(CLEANUP_GRACE).min(remaining / 2);
        let acknowledgement_deadline = deadline
            .checked_sub(containment_reserve)
            .unwrap_or_else(Instant::now);
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        if self
            .session
            .send(SessionCommand::Abort {
                bytes,
                control_id: control_id.to_owned(),
                invocation_id: invocation_id.to_owned(),
                deadline: acknowledgement_deadline,
                response: response_sender,
            })
            .is_err()
        {
            contain_process_group_until(self.process_group, deadline)?;
            return Ok(WorkerControlResult::Failed);
        }
        let acknowledgement_wait =
            acknowledgement_deadline.saturating_duration_since(Instant::now());
        let result = response_receiver.recv_timeout(acknowledgement_wait);
        match result {
            Ok(Ok(WorkerControlResult::Accepted)) => Ok(WorkerControlResult::Accepted),
            Ok(Ok(WorkerControlResult::TooLate)) => Ok(WorkerControlResult::TooLate),
            Ok(Ok(WorkerControlResult::Failed)) | Ok(Err(_)) | Err(_) => {
                contain_process_group_until(self.process_group, deadline)?;
                Ok(WorkerControlResult::Failed)
            }
        }
    }

    pub(crate) fn identity(&self) -> String {
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
        let stdin = child.stdin.take().ok_or_else(|| {
            terminate_spawned_child(&mut child, process_group);
            RunError::Contract("Browser Worker stdin unavailable")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            terminate_spawned_child(&mut child, process_group);
            RunError::Contract("Browser Worker stdout unavailable")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            terminate_spawned_child(&mut child, process_group);
            RunError::Contract("Browser Worker stderr unavailable")
        })?;
        let (stdout_sender, stdout_receiver) = mpsc::channel();
        let stdout_reader = thread::Builder::new()
            .name("local-qa-worker-stdout".to_owned())
            .spawn(move || read_stdout(stdout, stdout_sender))
            .map_err(|error| {
                terminate_spawned_child(&mut child, process_group);
                RunError::Io(error)
            })?;
        let (session_sender, session_receiver) = mpsc::channel();
        let session_thread = match thread::Builder::new()
            .name("local-qa-worker-session".to_owned())
            .spawn(move || run_session(stdin, stdout_receiver, session_receiver))
        {
            Ok(thread) => thread,
            Err(error) => {
                terminate_spawned_child(&mut child, process_group);
                let _ = join_bounded(stdout_reader, Instant::now() + READER_JOIN_LIMIT);
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
                drop(session_sender);
                let deadline = Instant::now() + READER_JOIN_LIMIT;
                let _ = join_bounded(session_thread, deadline);
                let _ = join_bounded(stdout_reader, deadline);
                return Err(RunError::Io(error));
            }
        };
        Ok(Self {
            child,
            process_group,
            session: Some(session_sender),
            session_thread: Some(session_thread),
            stdout_reader: Some(stdout_reader),
            stderr: stderr_receiver,
            stderr_reader: Some(stderr_reader),
            reaped: false,
        })
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), RunError> {
        self.session_request(|response| SessionCommand::Write {
            bytes: bytes.to_vec(),
            response,
        })
    }

    pub(crate) fn control_handle(&self) -> Result<WorkerControlHandle, RunError> {
        Ok(WorkerControlHandle {
            process_group: self.process_group,
            session: self
                .session
                .as_ref()
                .ok_or(RunError::Contract("Browser Worker session is closed"))?
                .clone(),
        })
    }

    pub(crate) fn close_stdin(&self) -> Result<(), RunError> {
        self.session_request(|response| SessionCommand::CloseStdin { response })
    }

    pub(crate) fn read_frame(&self, deadline: Instant) -> Result<ValidatedValue, RunError> {
        self.session_request(|response| SessionCommand::Read { deadline, response })
    }

    pub(crate) fn require_clean_eof(&self, deadline: Instant) -> Result<(), RunError> {
        self.session_request(|response| SessionCommand::CleanEof { deadline, response })
    }

    fn session_request<T>(
        &self,
        command: impl FnOnce(SyncSender<Result<T, SessionError>>) -> SessionCommand,
    ) -> Result<T, RunError> {
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.session
            .as_ref()
            .ok_or(RunError::Contract("Browser Worker session is closed"))?
            .send(command(response_sender))
            .map_err(|_| RunError::Contract("Browser Worker session stopped"))?;
        response_receiver
            .recv()
            .map_err(|_| RunError::Contract("Browser Worker session stopped"))?
            .map_err(|error| RunError::Contract(error.0))
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
        self.session.take();
        let stderr = self
            .stderr
            .recv_timeout(READER_JOIN_LIMIT)
            .map_err(|_| RunError::Contract("Browser Worker stderr reader timed out"))?
            .map_err(|_| RunError::Contract("Browser Worker stderr failed"))?;
        let join_deadline = Instant::now() + READER_JOIN_LIMIT;
        join_bounded(
            self.session_thread
                .take()
                .ok_or(RunError::Contract("Browser Worker session thread missing"))?,
            join_deadline,
        )
        .map_err(|_| RunError::Contract("Browser Worker session thread did not join"))?;
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
        self.session.take();
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
        if let Some(session) = self.session_thread.take() {
            let _ = join_bounded(session, deadline);
        }
        if let Some(reader) = self.stdout_reader.take() {
            let _ = join_bounded(reader, deadline);
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = join_bounded(reader, deadline);
        }
    }
}

fn run_session(
    stdin: ChildStdin,
    stdout: Receiver<ReaderEvent>,
    commands: Receiver<SessionCommand>,
) {
    let mut decoder = WireDecoder::default();
    let mut execution_frames = VecDeque::new();
    let mut pending_read = None;
    let mut pending_abort: Option<PendingAbort> = None;
    let mut pending_eof = None;
    let mut stdin = Some(stdin);
    let mut eof = false;

    loop {
        if let Some((deadline, response)) = pending_read.as_ref() {
            if let Some(frame) = execution_frames.pop_front() {
                let response = response.clone();
                pending_read = None;
                let _ = response.send(Ok(frame));
            } else if eof {
                let response = response.clone();
                pending_read = None;
                let _ = response.send(Err(SessionError("unexpected Browser Worker EOF")));
            } else if Instant::now() >= *deadline {
                let response = response.clone();
                pending_read = None;
                let _ = response.send(Err(SessionError("Browser Worker timed out")));
            }
        }
        if let Some(pending) = pending_abort.as_ref() {
            if Instant::now() >= pending.deadline {
                let response = pending.response.clone();
                pending_abort = None;
                let _ = response.send(Err(SessionError(
                    "Browser Worker control acknowledgement timed out",
                )));
            }
        }
        if let Some((deadline, response)) = pending_eof.as_ref() {
            if eof {
                let result = decoder.finish().and_then(|()| {
                    if execution_frames.is_empty() {
                        Ok(())
                    } else {
                        Err(SessionError("trailing Browser Worker frame"))
                    }
                });
                let response = response.clone();
                let _ = response.send(result);
                return;
            } else if Instant::now() >= *deadline {
                let response = response.clone();
                pending_eof = None;
                let _ = response.send(Err(SessionError("Browser Worker EOF timed out")));
            }
        }

        if eof
            && stdin.is_some()
            && execution_frames.is_empty()
            && pending_read.is_none()
            && pending_abort.is_none()
            && pending_eof.is_none()
        {
            return;
        }

        match commands.recv_timeout(IO_POLL_INTERVAL) {
            Ok(SessionCommand::Write { bytes, response }) => {
                let result = if let Some(stdin) = stdin.as_mut() {
                    stdin
                        .write_all(&bytes)
                        .and_then(|()| stdin.flush())
                        .map_err(|_| SessionError("Browser Worker stdin write failed"))
                } else {
                    Err(SessionError("Browser Worker stdin is closed"))
                };
                let _ = response.send(result);
            }
            Ok(SessionCommand::Read { deadline, response }) => {
                if pending_read.is_some() {
                    let _ = response.send(Err(SessionError("Browser Worker read already pending")));
                } else {
                    pending_read = Some((deadline, response));
                }
            }
            Ok(SessionCommand::Abort {
                bytes,
                control_id,
                invocation_id,
                deadline,
                response,
            }) => {
                if pending_abort.is_some() {
                    let _ = response.send(Err(SessionError(
                        "Browser Worker abort already pending",
                    )));
                } else if stdin.is_none() || Instant::now() >= deadline {
                    let _ = response.send(Err(SessionError(
                        "Browser Worker control channel closed",
                    )));
                } else {
                    let result = stdin
                        .as_mut()
                        .expect("open stdin checked above")
                        .write_all(&bytes)
                        .and_then(|()| stdin.flush())
                        .map_err(|_| SessionError("Browser Worker abort write failed"));
                    match result {
                        Ok(()) => {
                            pending_abort = Some(PendingAbort {
                                control_id,
                                invocation_id,
                                deadline,
                                response,
                            });
                        }
                        Err(error) => {
                            let _ = response.send(Err(error));
                        }
                    }
                }
            }
            Ok(SessionCommand::CloseStdin { response }) => {
                if stdin.take().is_some() {
                    let _ = response.send(Ok(()));
                } else {
                    let _ = response.send(Err(SessionError("Browser Worker stdin already closed")));
                }
            }
            Ok(SessionCommand::CleanEof { deadline, response }) => {
                if pending_eof.is_some() {
                    let _ = response.send(Err(SessionError(
                        "Browser Worker EOF wait already pending",
                    )));
                } else {
                    pending_eof = Some((deadline, response));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        loop {
            match stdout.try_recv() {
                Ok(ReaderEvent::Data(bytes)) => {
                    let frames = match decoder.push(&bytes) {
                        Ok(frames) => frames,
                        Err(error) => {
                            fail_session(
                                error,
                                &mut pending_read,
                                &mut pending_abort,
                                &mut pending_eof,
                            );
                            return;
                        }
                    };
                    let execution_count = frames
                        .iter()
                        .filter(|raw| {
                            frame_protocol(raw).as_deref()
                                == Some("qa.local-worker-protocol/v1")
                        })
                        .count();
                    if execution_count > 1 {
                        fail_session(
                            SessionError("unexpected Browser Worker frame sequence"),
                            &mut pending_read,
                            &mut pending_abort,
                            &mut pending_eof,
                        );
                        return;
                    }
                    for raw in frames {
                        if let Err(error) = route_output_frame(
                            &raw,
                            &mut execution_frames,
                            &mut pending_abort,
                        ) {
                            fail_session(
                                error,
                                &mut pending_read,
                                &mut pending_abort,
                                &mut pending_eof,
                            );
                            return;
                        }
                    }
                }
                Ok(ReaderEvent::Eof) => {
                    eof = true;
                    break;
                }
                Ok(ReaderEvent::Failed) => {
                    fail_session(
                        SessionError("Browser Worker stdout failed"),
                        &mut pending_read,
                        &mut pending_abort,
                        &mut pending_eof,
                    );
                    return;
                }
                Ok(ReaderEvent::Overflow) => {
                    fail_session(
                        SessionError("Browser Worker stdout exceeded limit"),
                        &mut pending_read,
                        &mut pending_abort,
                        &mut pending_eof,
                    );
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    eof = true;
                    break;
                }
            }
        }
    }
}

fn frame_protocol(raw: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(raw).ok()?;
    value.get("protocol")?.as_str().map(str::to_owned)
}

fn route_output_frame(
    raw: &[u8],
    execution_frames: &mut VecDeque<ValidatedValue>,
    pending_abort: &mut Option<PendingAbort>,
) -> Result<(), SessionError> {
    match frame_protocol(raw).as_deref() {
        Some("qa.local-worker-protocol/v1") => {
            execution_frames.push_back(
                validate_local_worker_frame(raw)
                    .map_err(|_| SessionError("malformed Browser Worker frame"))?,
            );
            Ok(())
        }
        Some("qa.local-worker-control/v1") => {
            let value: Value = serde_json::from_slice(raw)
                .map_err(|_| SessionError("malformed Browser Worker control frame"))?;
            let kind = value.get("kind").and_then(Value::as_str);
            let pending = pending_abort
                .take()
                .ok_or(SessionError("unexpected Browser Worker control frame"))?;
            if Instant::now() > pending.deadline {
                let _ = pending.response.send(Err(SessionError(
                    "Browser Worker control acknowledgement arrived late",
                )));
                return Ok(());
            }
            let control_id = value.get("control_id").and_then(Value::as_str);
            if control_id != Some(pending.control_id.as_str()) {
                let _ = pending
                    .response
                    .send(Err(SessionError("Browser Worker control identity failed")));
                return Ok(());
            }
            match kind {
                Some("cancel_ack") => {
                    let ack = validate_local_worker_cancel_ack(raw).map_err(|_| {
                        SessionError("invalid Browser Worker cancel acknowledgement")
                    })?;
                    let object = ack
                        .value()
                        .as_object()
                        .ok_or(SessionError("invalid Browser Worker cancel acknowledgement"))?;
                    if object.get("invocation_id").and_then(Value::as_str)
                        != Some(pending.invocation_id.as_str())
                    {
                        let _ = pending.response.send(Err(SessionError(
                            "Browser Worker control invocation relation failed",
                        )));
                        return Ok(());
                    }
                    let result = match object.get("status").and_then(Value::as_str) {
                        Some("accepted") => WorkerControlResult::Accepted,
                        Some("too_late") => WorkerControlResult::TooLate,
                        _ => {
                            let _ = pending.response.send(Err(SessionError(
                                "invalid Browser Worker cancel acknowledgement",
                            )));
                            return Ok(());
                        }
                    };
                    let _ = pending.response.send(Ok(result));
                    Ok(())
                }
                Some("control_failure") => {
                    validate_local_worker_control_failure(raw)
                        .map_err(|_| SessionError("invalid Browser Worker control failure"))?;
                    let _ = pending.response.send(Ok(WorkerControlResult::Failed));
                    Ok(())
                }
                _ => {
                    let _ = pending.response.send(Err(SessionError(
                        "unexpected Browser Worker control frame",
                    )));
                    Ok(())
                }
            }
        }
        _ => Err(SessionError("malformed Browser Worker frame")),
    }
}

fn fail_session(
    error: SessionError,
    pending_read: &mut Option<(Instant, SyncSender<Result<ValidatedValue, SessionError>>)>,
    pending_abort: &mut Option<PendingAbort>,
    pending_eof: &mut Option<(Instant, SyncSender<Result<(), SessionError>>)>,
) {
    if let Some((_, response)) = pending_read.take() {
        let _ = response.send(Err(error));
    }
    if let Some(pending) = pending_abort.take() {
        let _ = pending.response.send(Err(error));
    }
    if let Some((_, response)) = pending_eof.take() {
        let _ = response.send(Err(error));
    }
}

fn contain_process_group_until(process_group: Pid, deadline: Instant) -> Result<(), RunError> {
    signal_group(process_group, Signal::SIGTERM)
        .map_err(|_| RunError::Contract("Browser Worker group termination failed"))?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    let grace_deadline = deadline.min(Instant::now() + remaining / 2);
    while Instant::now() < grace_deadline && process_group_is_alive(process_group)? {
        thread::sleep(IO_POLL_INTERVAL);
    }
    if process_group_is_alive(process_group)? && Instant::now() < deadline {
        signal_group(process_group, Signal::SIGKILL)
            .map_err(|_| RunError::Contract("Browser Worker group kill failed"))?;
        while Instant::now() < deadline && process_group_is_alive(process_group)? {
            thread::sleep(IO_POLL_INTERVAL);
        }
    }
    Ok(())
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
        let error = process
            .read_frame(Instant::now() + Duration::from_secs(2))
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
        let error = process
            .read_frame(Instant::now() + Duration::from_secs(2))
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
        let first = process.read_frame(Instant::now() + Duration::from_secs(2));
        if first.is_ok() {
            let error = process
                .require_clean_eof(Instant::now() + Duration::from_secs(2))
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
    fn control_handle_writes_abort_and_observes_matching_acknowledgement() {
        let directory = temporary_directory("worker-control-ack");
        let script = directory.join("worker.py");
        fs::write(
            &script,
            r#"#!/usr/bin/env python3
import json, struct, sys
length = struct.unpack('>I', sys.stdin.buffer.read(4))[0]
value = json.loads(sys.stdin.buffer.read(length))
ack = json.dumps({
  'protocol': 'qa.local-worker-control/v1',
  'kind': 'cancel_ack',
  'control_id': value['control_id'],
  'invocation_id': value['invocation_id'],
  'status': 'accepted'
}, separators=(',', ':')).encode()
sys.stdout.buffer.write(struct.pack('>I', len(ack)) + ack)
sys.stdout.buffer.flush()
"#,
        )
        .expect("control Worker script writes");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("control Worker script is executable");

        let mut process = WorkerProcess::spawn(&script, &script).expect("Worker process starts");
        let handle = process.control_handle().expect("control handle attaches");
        let deadline = Instant::now() + Duration::from_secs(2);
        assert_eq!(
            handle
                .request_abort(
                    "00000000-0000-0000-0000-000000000001",
                    "invocation/0",
                    "2099-09-02T12:00:00Z",
                    deadline,
                )
                .expect("abort acknowledgement arrives"),
            WorkerControlResult::Accepted
        );
        process.terminate();
        fs::remove_dir_all(directory).expect("temporary Worker directory removes");
    }

    #[test]
    fn missing_acknowledgement_uses_only_the_remaining_deadline_for_containment() {
        let directory = temporary_directory("worker-control-no-ack");
        let script = directory.join("worker.sh");
        fs::write(&script, b"#!/bin/sh\ntrap '' TERM\ncat >/dev/null\n")
            .expect("no-ack Worker script writes");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("no-ack Worker script is executable");

        let mut process = WorkerProcess::spawn(&script, &script).expect("Worker process starts");
        let process_group = process.process_group;
        let handle = process.control_handle().expect("control handle attaches");
        let started = Instant::now();
        let deadline = started + Duration::from_millis(300);
        assert_eq!(
            handle
                .request_abort(
                    "00000000-0000-0000-0000-000000000001",
                    "invocation/0",
                    "2099-09-02T12:00:00Z",
                    deadline,
                )
                .expect("missing acknowledgement is contained"),
            WorkerControlResult::Failed
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!process_group_is_alive(process_group).expect("group inspection succeeds"));
        process.terminate();
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
