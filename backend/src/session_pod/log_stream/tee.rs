//! The supervise-stream TEE: copy the child's stdout/stderr byte-for-byte to the
//! driver's own inherited stream (so `kubectl logs` + the health scrape are
//! UNCHANGED) while ALSO forwarding each complete line to the collector.
//!
//! Correctness hinges on one invariant: the inherited stream must receive the
//! child's bytes verbatim and in order. So the raw chunk is written to the inherited
//! writer FIRST, unconditionally, and only then is a lossy copy framed into lines for
//! the (best-effort, drop-on-full) collector channel. A slow or full collector can
//! never delay or alter the inherited stream.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::classify::LogClass;
use super::collector::CollectorRecord;
use super::tail::TailTracker;

/// Read `reader` to EOF, writing every chunk verbatim to `inherited` and forwarding
/// each complete line to `sender` tagged with `class`. Returns once the child stream
/// closes. Forwarding is best-effort: a full or disconnected channel drops the line
/// (the inherited stream still carries it), never blocking the child.
pub async fn tee_reader<R, W>(
    mut reader: R,
    mut inherited: W,
    sender: std::sync::mpsc::SyncSender<CollectorRecord>,
    class: LogClass,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = [0u8; 8192];
    let mut framer = TailTracker::new();
    loop {
        let read = reader.read(&mut buf).await?;
        if read == 0 {
            break;
        }
        // Invariant: the inherited stream is byte-faithful. Write the raw bytes
        // BEFORE any framing/forwarding work so kubectl logs is never altered.
        inherited.write_all(&buf[..read]).await?;
        inherited.flush().await?;

        let chunk = String::from_utf8_lossy(&buf[..read]);
        for line in framer.frame(&chunk) {
            forward(&sender, class, line);
        }
    }
    // A final unterminated line still belongs in the stream.
    if let Some(tail) = framer.finish() {
        forward(&sender, class, tail);
    }
    inherited.flush().await?;
    Ok(())
}

/// Try to forward one line to the collector, dropping it on a full/closed channel.
/// The line is NEVER logged (it may hold a not-yet-redacted secret).
fn forward(sender: &std::sync::mpsc::SyncSender<CollectorRecord>, class: LogClass, line: String) {
    // try_send is non-blocking: Full/Disconnected both drop silently — the inherited
    // stream already carried the bytes, so the engine is unaffected.
    let _ = sender.try_send((class, line));
}

#[cfg(test)]
#[path = "tee_tests.rs"]
mod tests;
