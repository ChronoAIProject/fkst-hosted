//! An in-memory `tracing` writer, so a test can search the endpoint's own log
//! output the same way it searches a response body.
//!
//! Logs are the one observability surface a response-body assertion cannot reach:
//! a handler that redacts perfectly on the wire and then writes a collaborator
//! login, a hidden runtime id, or a bearer token into a structured field has
//! leaked it just as effectively. The epic names logs explicitly alongside JSON,
//! audit arguments, and metrics, so they get the same canary treatment.
//!
//! The subscriber is installed with `tracing::subscriber::set_default`, which is
//! THREAD-LOCAL. `#[tokio::test]` runs the current-thread runtime, so every poll
//! of the router future happens on the installing thread and is captured; other
//! tests running in parallel on other threads write nowhere near it.

#![allow(dead_code)]

use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use tracing::level_filters::LevelFilter;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;

/// A shared, in-memory sink for structured log output.
#[derive(Clone, Default)]
pub struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    /// Install a TRACE-level capturing subscriber for the current thread. The
    /// returned guard uninstalls it when dropped.
    ///
    /// TRACE is deliberate: a canary must be absent at the most verbose level
    /// anything is ever run at, not merely at the default one.
    pub fn install() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(LevelFilter::TRACE)
            .with_ansi(false)
            .with_target(true)
            .with_writer(logs.clone())
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        (logs, guard)
    }

    /// Everything written so far.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.lock()).into_owned()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<u8>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
