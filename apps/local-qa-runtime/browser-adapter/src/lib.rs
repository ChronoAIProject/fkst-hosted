use thiserror::Error;

#[derive(Debug, PartialEq, Eq)]
pub struct FixedBrowserSmokeResult {
    pub final_url: String,
    pub selector: String,
    pub expected_text: String,
    pub observed_text: String,
    pub screenshot: FixedPngScreenshot,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FixedPngScreenshot {
    pub bytes: Vec<u8>,
    pub media_type: String,
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FixedBrowserObservation {
    pub final_url: String,
    pub observed_text: String,
    pub screenshot: FixedPngScreenshot,
}

#[derive(Debug, Error)]
pub enum BrowserAdapterError {
    #[error("unsupported platform: the fixed browser smoke supports Linux and macOS arm64 only")]
    UnsupportedPlatform,
    #[error("no allowlisted system Chrome executable found")]
    ChromeNotFound,
    #[error("fixed browser smoke setup failed: {0}")]
    Setup(String),
    #[error("fixed browser smoke operation failed: {0}")]
    Operation(String),
    #[error("fixed browser smoke cleanup failed: {0}")]
    Cleanup(String),
    #[error("fixed browser smoke operation failed: {operation}; cleanup also failed: {cleanup}")]
    OperationAndCleanup { operation: String, cleanup: String },
}

pub async fn observe_fixed_browser_fixture(
) -> Result<FixedBrowserObservation, BrowserAdapterError> {
    #[cfg(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64")))]
    {
        let (sender, receiver) = futures_channel::oneshot::channel();
        std::thread::Builder::new()
            .name("local-qa-fixed-browser-observation".to_string())
            .spawn(move || {
                let _ = sender.send(unix::observe_fixed_browser_fixture());
            })
            .map_err(|error| {
                BrowserAdapterError::Setup(format!(
                    "start fixed browser observation worker: {error}"
                ))
            })?;
        receiver.await.map_err(|_| {
            BrowserAdapterError::Operation(
                "fixed browser observation worker terminated without a result".to_string(),
            )
        })?
    }

    #[cfg(not(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64"))))]
    {
        Err(BrowserAdapterError::UnsupportedPlatform)
    }
}

pub async fn run_fixed_browser_smoke() -> Result<FixedBrowserSmokeResult, BrowserAdapterError> {
    let observation = observe_fixed_browser_fixture().await?;
    if observation.observed_text != fixed_expected_text() {
        return Err(BrowserAdapterError::Operation(format!(
            "fixed status rendered text was {:?}, expected {:?}",
            observation.observed_text,
            fixed_expected_text()
        )));
    }

    Ok(FixedBrowserSmokeResult {
        final_url: observation.final_url,
        selector: fixed_selector().to_string(),
        expected_text: fixed_expected_text().to_string(),
        observed_text: observation.observed_text,
        screenshot: observation.screenshot,
    })
}

fn fixed_selector() -> &'static str {
    r#"[data-local-qa="status"]"#
}

fn fixed_expected_text() -> &'static str {
    "READY"
}

#[cfg(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64")))]
mod unix {
    use super::{
        fixed_selector, BrowserAdapterError, FixedBrowserObservation, FixedPngScreenshot,
    };
    use headless_chrome::{protocol::cdp::Page, Browser, Tab};
    use nix::{
        errno::Errno,
        sys::signal::{killpg, Signal},
        unistd::Pid,
    };
    use png::Decoder;
    use serde_json::json;
    use std::{
        fs,
        io::{Cursor, Read, Write},
        net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
        os::unix::{fs::PermissionsExt, process::CommandExt},
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };
    use tempfile::TempDir;

    #[cfg(target_os = "linux")]
    const CHROME_CANDIDATES: [&str; 4] = [
        "/usr/bin/google-chrome-stable",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ];
    #[cfg(target_os = "macos")]
    const CHROME_CANDIDATES: [&str; 1] =
        ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"];
    const FIXTURE_PATH: &str = "/fixed-page.html";
    const FIXTURE_HTML: &[u8] = br#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Local QA Fixed Page</title></head><body><main><span data-local-qa="status">READY</span></main></body></html>"#;
    #[cfg(test)]
    const NOT_READY_FIXTURE_HTML: &[u8] = br#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Local QA Fixed Page</title></head><body><main><span data-local-qa="status">NOT READY</span></main></body></html>"#;
    #[cfg(test)]
    const MISSING_SELECTOR_FIXTURE_HTML: &[u8] = br#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Local QA Fixed Page</title></head><body><main>READY</main></body></html>"#;
    #[cfg(test)]
    const REDIRECT_PATH: &str = "/redirected-page.html";
    const SCREENSHOT_MEDIA_TYPE: &str = "image/png";
    const VIEWPORT_WIDTH: u32 = 1280;
    const VIEWPORT_HEIGHT: u32 = 720;
    const MAX_SCREENSHOT_BYTES: usize = 2_097_152;
    const OPERATION_TIMEOUT: Duration = Duration::from_secs(15);
    const IO_POLL_INTERVAL: Duration = Duration::from_millis(10);
    const CLEANUP_GRACE: Duration = Duration::from_millis(500);
    const CLEANUP_LIMIT: Duration = Duration::from_secs(3);

    pub(super) fn observe_fixed_browser_fixture(
    ) -> Result<FixedBrowserObservation, BrowserAdapterError> {
        run_with_options(RunOptions::production())
    }

    #[derive(Clone, Copy)]
    enum FixtureContent {
        Ready,
        #[cfg(test)]
        NotReady,
        #[cfg(test)]
        Redirect,
        #[cfg(test)]
        MissingSelector,
    }

    impl FixtureContent {
        fn html(self) -> &'static [u8] {
            match self {
                Self::Ready => FIXTURE_HTML,
                #[cfg(test)]
                Self::NotReady => NOT_READY_FIXTURE_HTML,
                #[cfg(test)]
                Self::Redirect | Self::MissingSelector => MISSING_SELECTOR_FIXTURE_HTML,
            }
        }
    }

    struct RunOptions {
        timeout: Duration,
        fixture_content: FixtureContent,
        #[cfg(test)]
        executable: Option<PathBuf>,
        #[cfg(test)]
        screenshot_override: Option<Vec<u8>>,
        #[cfg(test)]
        cleanup_failure: Option<&'static str>,
    }

    impl RunOptions {
        fn production() -> Self {
            Self {
                timeout: OPERATION_TIMEOUT,
                fixture_content: FixtureContent::Ready,
                #[cfg(test)]
                executable: None,
                #[cfg(test)]
                screenshot_override: None,
                #[cfg(test)]
                cleanup_failure: None,
            }
        }

        fn chrome_path(&self) -> Result<PathBuf, BrowserAdapterError> {
            #[cfg(test)]
            if let Some(executable) = &self.executable {
                return Ok(executable.clone());
            }
            discover_chrome()
        }
    }

    #[derive(Default)]
    struct OwnedRun {
        fixture: Option<FixtureServer>,
        profile: Option<TempDir>,
        downloads: Option<TempDir>,
        chrome: Option<OwnedChrome>,
    }

    fn run_with_options(
        options: RunOptions,
    ) -> Result<FixedBrowserObservation, BrowserAdapterError> {
        let mut owned = OwnedRun::default();
        let operation = (|| {
            let chrome_path = options.chrome_path()?;
            owned.fixture = Some(FixtureServer::start(options.fixture_content)?);
            record_owned_resources(&owned);

            owned.profile =
                Some(TempDir::new().map_err(setup_error("create temporary Chrome profile"))?);
            record_owned_resources(&owned);
            owned.downloads = Some(
                TempDir::new()
                    .map_err(setup_error("create temporary Chrome downloads directory"))?,
            );
            record_owned_resources(&owned);

            write_download_preferences(
                owned.profile.as_ref().expect("owned profile exists").path(),
                owned
                    .downloads
                    .as_ref()
                    .expect("owned downloads directory exists")
                    .path(),
            )?;

            let deadline = Instant::now() + options.timeout;
            owned.chrome = Some(OwnedChrome::launch(
                &chrome_path,
                owned.profile.as_ref().expect("owned profile exists").path(),
                deadline,
            )?);
            record_owned_resources(&owned);

            let navigation_url = owned
                .fixture
                .as_ref()
                .expect("owned fixture exists")
                .navigation_url();
            perform_smoke(
                owned.chrome.as_mut().expect("owned Chrome exists"),
                navigation_url,
                deadline,
                &options,
            )
        })();

        owned.finish(operation, &options)
    }

    fn perform_smoke(
        chrome: &mut OwnedChrome,
        navigation_url: String,
        deadline: Instant,
        options: &RunOptions,
    ) -> Result<FixedBrowserObservation, BrowserAdapterError> {
        #[cfg(not(test))]
        let _ = options;
        let debug_ws_url = chrome.wait_for_debug_ws_url(deadline)?;
        let browser = Browser::connect_with_timeout(debug_ws_url, remaining(deadline)?).map_err(
            operation_error_before_deadline("connect to owned Chrome", deadline),
        )?;
        browser.set_default_timeout(remaining(deadline)?);

        let tab = wait_for_initial_tab(&browser, deadline)?;
        tab.set_default_timeout(remaining(deadline)?);
        tab.navigate_to(&navigation_url)
            .and_then(|tab| tab.wait_until_navigated())
            .map_err(operation_error_before_deadline(
                "navigate to fixed fixture",
                deadline,
            ))?;
        let final_url = tab.get_url();
        validate_final_url(&final_url, &navigation_url)?;
        tab.set_default_timeout(remaining(deadline)?);

        let element =
            tab.wait_for_element(fixed_selector())
                .map_err(operation_error_before_deadline(
                    "locate fixed status element",
                    deadline,
                ))?;
        let observed_value = element
            .call_js_fn("function() { return this.innerText; }", Vec::new(), false)
            .map_err(operation_error_before_deadline(
                "read fixed status rendered text",
                deadline,
            ))?
            .value;
        let observed_text = decode_rendered_text(observed_value)?;

        ensure_before_deadline(deadline)?;
        #[cfg(test)]
        let screenshot = match &options.screenshot_override {
            Some(screenshot) => screenshot.clone(),
            None => tab
                .capture_screenshot(
                    Page::CaptureScreenshotFormatOption::Png,
                    None,
                    Some(fixed_screenshot_viewport()),
                    true,
                )
                .map_err(operation_error_before_deadline(
                    "capture fixed PNG screenshot",
                    deadline,
                ))?,
        };
        #[cfg(not(test))]
        let screenshot = tab
            .capture_screenshot(
                Page::CaptureScreenshotFormatOption::Png,
                None,
                Some(fixed_screenshot_viewport()),
                true,
            )
            .map_err(operation_error_before_deadline(
                "capture fixed PNG screenshot",
                deadline,
            ))?;
        let (width_px, height_px) = validate_png(&screenshot)?;
        ensure_before_deadline(deadline)?;

        drop(element);
        tab.close(false).map_err(operation_error_before_deadline(
            "close fixed Chrome tab",
            deadline,
        ))?;
        drop(tab);
        drop(browser);

        Ok(FixedBrowserObservation {
            final_url,
            observed_text,
            screenshot: FixedPngScreenshot {
                bytes: screenshot,
                media_type: SCREENSHOT_MEDIA_TYPE.to_string(),
                width_px,
                height_px,
            },
        })
    }

    fn validate_final_url(
        final_url: &str,
        navigation_url: &str,
    ) -> Result<(), BrowserAdapterError> {
        if final_url == navigation_url {
            Ok(())
        } else {
            Err(BrowserAdapterError::Operation(format!(
                "final URL was {final_url:?}, expected {navigation_url:?}"
            )))
        }
    }

    fn decode_rendered_text(
        value: Option<serde_json::Value>,
    ) -> Result<String, BrowserAdapterError> {
        match value {
            Some(serde_json::Value::String(text)) => Ok(text),
            Some(value) => Err(BrowserAdapterError::Operation(format!(
                "fixed status rendered text was not a string: {value}"
            ))),
            None => Err(BrowserAdapterError::Operation(
                "fixed status rendered text did not produce a value".to_string(),
            )),
        }
    }

    fn wait_for_initial_tab(
        browser: &Browser,
        deadline: Instant,
    ) -> Result<Arc<Tab>, BrowserAdapterError> {
        loop {
            ensure_before_deadline(deadline)?;
            if let Some(tab) = browser
                .get_tabs()
                .lock()
                .map_err(operation_error("inspect owned Chrome tabs"))?
                .iter()
                .find(|tab| tab.get_url() == "about:blank")
                .cloned()
            {
                ensure_before_deadline(deadline)?;
                return Ok(tab);
            }
            thread::sleep(IO_POLL_INTERVAL.min(remaining(deadline)?));
        }
    }

    fn fixed_screenshot_viewport() -> Page::Viewport {
        Page::Viewport {
            x: 0.0,
            y: 0.0,
            width: f64::from(VIEWPORT_WIDTH),
            height: f64::from(VIEWPORT_HEIGHT),
            scale: 1.0,
        }
    }

    impl OwnedRun {
        fn finish(
            mut self,
            operation: Result<FixedBrowserObservation, BrowserAdapterError>,
            options: &RunOptions,
        ) -> Result<FixedBrowserObservation, BrowserAdapterError> {
            #[cfg(not(test))]
            let _ = options;
            record_owned_resources(&self);
            let mut cleanup_failures = Vec::new();

            if let Some(mut chrome) = self.chrome.take() {
                if let Err(error) = chrome.cleanup() {
                    cleanup_failures.push(error);
                }
            }
            if let Some(fixture) = self.fixture.take() {
                if let Err(error) = fixture.stop() {
                    cleanup_failures.push(error);
                }
            }
            if let Some(downloads) = self.downloads.take() {
                if let Err(error) = downloads.close() {
                    cleanup_failures.push(format!("remove owned downloads directory: {error}"));
                }
            }
            if let Some(profile) = self.profile.take() {
                if let Err(error) = profile.close() {
                    cleanup_failures.push(format!("remove owned profile directory: {error}"));
                }
            }
            #[cfg(test)]
            if let Some(error) = options.cleanup_failure {
                cleanup_failures.push(error.to_string());
            }

            combine_outcome(operation, cleanup_failures)
        }
    }

    fn combine_outcome(
        operation: Result<FixedBrowserObservation, BrowserAdapterError>,
        cleanup_failures: Vec<String>,
    ) -> Result<FixedBrowserObservation, BrowserAdapterError> {
        let cleanup = cleanup_failures.join("; ");
        match (operation, cleanup.is_empty()) {
            (Ok(result), true) => Ok(result),
            (Ok(_), false) => Err(BrowserAdapterError::Cleanup(cleanup)),
            (Err(error), true) => Err(error),
            (Err(error), false) => Err(BrowserAdapterError::OperationAndCleanup {
                operation: error.to_string(),
                cleanup,
            }),
        }
    }

    fn discover_chrome() -> Result<PathBuf, BrowserAdapterError> {
        discover_chrome_from(CHROME_CANDIDATES.iter().map(Path::new))
    }

    fn discover_chrome_from<'a>(
        candidates: impl IntoIterator<Item = &'a Path>,
    ) -> Result<PathBuf, BrowserAdapterError> {
        for candidate in candidates {
            let Ok(metadata) = fs::metadata(candidate) else {
                continue;
            };
            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                return Ok(candidate.to_path_buf());
            }
        }
        Err(BrowserAdapterError::ChromeNotFound)
    }

    fn write_download_preferences(
        profile: &Path,
        downloads: &Path,
    ) -> Result<(), BrowserAdapterError> {
        let default_profile = profile.join("Default");
        fs::create_dir(&default_profile)
            .map_err(setup_error("create owned Chrome Default profile directory"))?;
        let preferences = json!({
            "download": {
                "default_directory": downloads,
                "prompt_for_download": false
            }
        });
        fs::write(
            default_profile.join("Preferences"),
            serde_json::to_vec(&preferences)
                .map_err(setup_error("encode owned Chrome download preferences"))?,
        )
        .map_err(setup_error("write owned Chrome download preferences"))
    }

    struct FixtureServer {
        address: SocketAddrV4,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<Result<(), String>>>,
    }

    impl FixtureServer {
        fn start(content: FixtureContent) -> Result<Self, BrowserAdapterError> {
            let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
                .map_err(setup_error("bind fixed loopback fixture"))?;
            listener
                .set_nonblocking(true)
                .map_err(setup_error("configure fixed loopback fixture"))?;
            let address = match listener.local_addr() {
                Ok(std::net::SocketAddr::V4(address)) => address,
                Ok(_) => {
                    return Err(BrowserAdapterError::Setup(
                        "fixed fixture did not bind an IPv4 address".to_string(),
                    ));
                }
                Err(error) => {
                    return Err(BrowserAdapterError::Setup(format!(
                        "read fixed fixture address: {error}"
                    )));
                }
            };
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let thread = thread::Builder::new()
                .name("local-qa-fixed-fixture".to_string())
                .spawn(move || serve_fixture(listener, &thread_stop, content))
                .map_err(setup_error("start fixed fixture thread"))?;
            Ok(Self {
                address,
                stop,
                thread: Some(thread),
            })
        }

        fn navigation_url(&self) -> String {
            format!("http://127.0.0.1:{}{FIXTURE_PATH}", self.address.port())
        }

        fn stop(mut self) -> Result<(), String> {
            self.stop.store(true, Ordering::Release);
            let Some(thread) = self.thread.take() else {
                return Ok(());
            };
            match thread.join() {
                Ok(result) => result,
                Err(_) => Err("fixed fixture thread panicked".to_string()),
            }
        }
    }

    impl Drop for FixtureServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn serve_fixture(
        listener: TcpListener,
        stop: &AtomicBool,
        content: FixtureContent,
    ) -> Result<(), String> {
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => handle_fixture_connection(stream, content)?,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(IO_POLL_INTERVAL);
                }
                Err(error) => return Err(format!("accept fixed fixture connection: {error}")),
            }
        }
        Ok(())
    }

    fn handle_fixture_connection(
        mut stream: TcpStream,
        content: FixtureContent,
    ) -> Result<(), String> {
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .map_err(|error| format!("configure fixed fixture connection: {error}"))?;
        let mut request = Vec::with_capacity(1024);
        let mut chunk = [0_u8; 1024];
        while request.len() <= 8192 && !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => request.extend_from_slice(&chunk[..read]),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => return Err(format!("read fixed fixture request: {error}")),
            }
        }

        let header_end = request.windows(4).position(|bytes| bytes == b"\r\n\r\n");
        let request_line = request
            .split(|byte| *byte == b'\n')
            .next()
            .map(|line| line.strip_suffix(b"\r").unwrap_or(line));
        let request_path = request_line.and_then(|line| {
            line.strip_prefix(b"GET ")
                .and_then(|line| line.strip_suffix(b" HTTP/1.1"))
        });
        let valid_request = header_end.is_some_and(|header_end| {
            request.len() <= 8192
                && request_path.is_some()
                && request[header_end + 4..].is_empty()
                && !headers_declare_body(&request[..header_end])
        });
        if valid_request && request_path == Some(FIXTURE_PATH.as_bytes()) {
            #[cfg(test)]
            if matches!(content, FixtureContent::Redirect) {
                return write_response(
                    &mut stream,
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: {REDIRECT_PATH}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                    &[],
                );
            }
            let fixture_html = content.html();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                fixture_html.len()
            );
            write_response(&mut stream, head.as_bytes(), fixture_html)
        } else if valid_request && is_redirect_destination(content, request_path) {
            let fixture_html = content.html();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                fixture_html.len()
            );
            write_response(&mut stream, head.as_bytes(), fixture_html)
        } else {
            write_response(
                &mut stream,
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                &[],
            )
        }
    }

    fn is_redirect_destination(content: FixtureContent, request_path: Option<&[u8]>) -> bool {
        #[cfg(test)]
        {
            matches!(content, FixtureContent::Redirect)
                && request_path == Some(REDIRECT_PATH.as_bytes())
        }
        #[cfg(not(test))]
        {
            let _ = (content, request_path);
            false
        }
    }

    fn write_response(stream: &mut TcpStream, head: &[u8], body: &[u8]) -> Result<(), String> {
        stream
            .write_all(head)
            .and_then(|()| stream.write_all(body))
            .and_then(|()| stream.flush())
            .map_err(|error| format!("write fixed fixture response: {error}"))
    }

    fn headers_declare_body(headers: &[u8]) -> bool {
        headers.split(|byte| *byte == b'\n').skip(1).any(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let Some(separator) = line.iter().position(|byte| *byte == b':') else {
                return true;
            };
            let (name, value) = line.split_at(separator);
            let value = &value[1..];
            if name.eq_ignore_ascii_case(b"transfer-encoding") {
                return true;
            }
            name.eq_ignore_ascii_case(b"content-length")
                && value
                    .iter()
                    .copied()
                    .filter(|byte| !byte.is_ascii_whitespace())
                    .any(|byte| byte != b'0')
        })
    }

    struct OwnedChrome {
        child: Option<Child>,
        process_group: Pid,
        profile_path: PathBuf,
        watchdog_done: Option<mpsc::Sender<()>>,
        watchdog: Option<JoinHandle<()>>,
        cleaned: bool,
    }

    impl OwnedChrome {
        fn launch(
            executable: &Path,
            profile_path: &Path,
            deadline: Instant,
        ) -> Result<Self, BrowserAdapterError> {
            let user_data_argument = format!("--user-data-dir={}", profile_path.display());
            let mut command = Command::new(executable);
            command
                .args([
                    "--headless=new",
                    "--window-size=1280,720",
                    "--force-device-scale-factor=1",
                    "--remote-debugging-address=127.0.0.1",
                    "--remote-debugging-port=0",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--disable-background-networking",
                    "--disable-component-update",
                    "--disable-default-apps",
                    "--disable-extensions",
                    "--disable-sync",
                    "--metrics-recording-only",
                    user_data_argument.as_str(),
                    "about:blank",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0);
            let child = command
                .spawn()
                .map_err(operation_error("launch allowlisted system Chrome"))?;
            let process_group = Pid::from_raw(child.id() as i32);
            let (watchdog_done, watchdog_rx) = mpsc::channel();
            let watchdog = match thread::Builder::new()
                .name("local-qa-chrome-deadline".to_string())
                .spawn(move || {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    if watchdog_rx.recv_timeout(wait).is_err() {
                        let _ = killpg(process_group, Signal::SIGKILL);
                    }
                }) {
                Ok(watchdog) => watchdog,
                Err(error) => {
                    let operation = BrowserAdapterError::Operation(format!(
                        "start Chrome deadline watchdog: {error}"
                    ));
                    let mut chrome = Self {
                        child: Some(child),
                        process_group,
                        profile_path: profile_path.to_path_buf(),
                        watchdog_done: None,
                        watchdog: None,
                        cleaned: false,
                    };
                    return match chrome.cleanup() {
                        Ok(()) => Err(operation),
                        Err(cleanup) => Err(BrowserAdapterError::OperationAndCleanup {
                            operation: operation.to_string(),
                            cleanup,
                        }),
                    };
                }
            };
            Ok(Self {
                child: Some(child),
                process_group,
                profile_path: profile_path.to_path_buf(),
                watchdog_done: Some(watchdog_done),
                watchdog: Some(watchdog),
                cleaned: false,
            })
        }

        fn wait_for_debug_ws_url(
            &mut self,
            deadline: Instant,
        ) -> Result<String, BrowserAdapterError> {
            let active_port = self.profile_path.join("DevToolsActivePort");
            loop {
                ensure_before_deadline(deadline)?;
                if let Some(status) = self
                    .child
                    .as_mut()
                    .expect("owned Chrome child exists before cleanup")
                    .try_wait()
                    .map_err(operation_error_before_deadline(
                        "observe owned Chrome startup",
                        deadline,
                    ))?
                {
                    return Err(BrowserAdapterError::Operation(format!(
                        "allowlisted system Chrome exited during launch with {status}"
                    )));
                }
                match fs::read_to_string(&active_port) {
                    Ok(contents) => return parse_debug_ws_url(&contents),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        thread::sleep(IO_POLL_INTERVAL);
                    }
                    Err(error) => {
                        return Err(BrowserAdapterError::Operation(format!(
                            "read owned Chrome debugging endpoint: {error}"
                        )));
                    }
                }
            }
        }

        fn cleanup(&mut self) -> Result<(), String> {
            self.stop_watchdog();
            let mut failures = Vec::new();
            if let Err(error) = signal_process_group(self.process_group, Signal::SIGTERM) {
                failures.push(format!("terminate owned Chrome process group: {error}"));
            }
            match wait_for_process_group_exit(self.process_group, CLEANUP_GRACE) {
                Ok(true) => {}
                Ok(false) => {
                    if let Err(error) = signal_process_group(self.process_group, Signal::SIGKILL) {
                        failures.push(format!("kill owned Chrome process group: {error}"));
                    }
                }
                Err(error) => {
                    failures.push(error);
                    if let Err(error) = signal_process_group(self.process_group, Signal::SIGKILL) {
                        failures.push(format!("kill owned Chrome process group: {error}"));
                    }
                }
            }
            if let Some(mut child) = self.child.take() {
                match child.try_wait() {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        if let Err(error) = child.kill() {
                            failures.push(format!("kill owned Chrome root process: {error}"));
                        }
                    }
                    Err(error) => {
                        failures.push(format!("observe owned Chrome root process: {error}"));
                    }
                }
                if let Err(error) = child.wait() {
                    failures.push(format!("reap owned Chrome root process: {error}"));
                }
            }
            let group_exited = match wait_for_process_group_exit(self.process_group, CLEANUP_LIMIT)
            {
                Ok(true) => true,
                Ok(false) => {
                    failures.push(format!(
                        "owned Chrome process group {} still has live members",
                        self.process_group
                    ));
                    false
                }
                Err(error) => {
                    failures.push(error);
                    false
                }
            };
            self.cleaned = group_exited;
            if failures.is_empty() {
                Ok(())
            } else {
                Err(failures.join("; "))
            }
        }

        fn stop_watchdog(&mut self) {
            if let Some(sender) = self.watchdog_done.take() {
                let _ = sender.send(());
            }
            if let Some(watchdog) = self.watchdog.take() {
                let _ = watchdog.join();
            }
        }
    }

    impl Drop for OwnedChrome {
        fn drop(&mut self) {
            if self.cleaned {
                return;
            }
            self.stop_watchdog();
            let _ = killpg(self.process_group, Signal::SIGKILL);
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = wait_for_process_group_exit(self.process_group, CLEANUP_LIMIT);
            self.cleaned = true;
        }
    }

    fn parse_debug_ws_url(contents: &str) -> Result<String, BrowserAdapterError> {
        let mut lines = contents.lines();
        let port: u16 = lines
            .next()
            .ok_or_else(|| {
                BrowserAdapterError::Operation(
                    "owned Chrome debugging endpoint omitted its port".to_string(),
                )
            })?
            .parse()
            .map_err(operation_error("parse owned Chrome debugging port"))?;
        let browser_path = lines.next().ok_or_else(|| {
            BrowserAdapterError::Operation(
                "owned Chrome debugging endpoint omitted its browser path".to_string(),
            )
        })?;
        if !browser_path.starts_with("/devtools/browser/") {
            return Err(BrowserAdapterError::Operation(
                "owned Chrome debugging endpoint returned an unexpected browser path".to_string(),
            ));
        }
        Ok(format!("ws://127.0.0.1:{port}{browser_path}"))
    }

    fn validate_png(bytes: &[u8]) -> Result<(u32, u32), BrowserAdapterError> {
        if bytes.is_empty() {
            return Err(BrowserAdapterError::Operation(
                "fixed PNG screenshot was empty".to_string(),
            ));
        }
        if bytes.len() > MAX_SCREENSHOT_BYTES {
            return Err(BrowserAdapterError::Operation(format!(
                "fixed PNG screenshot was {} bytes, exceeding {MAX_SCREENSHOT_BYTES}",
                bytes.len()
            )));
        }
        let decoder = Decoder::new(Cursor::new(bytes));
        let mut reader = decoder
            .read_info()
            .map_err(operation_error("decode fixed PNG screenshot metadata"))?;
        let mut decoded = vec![
            0;
            reader.output_buffer_size().ok_or_else(|| {
                BrowserAdapterError::Operation(
                    "fixed PNG screenshot decoded size is unknown".to_string(),
                )
            })?
        ];
        let output = reader
            .next_frame(&mut decoded)
            .map_err(operation_error("decode fixed PNG screenshot pixels"))?;
        if output.width != VIEWPORT_WIDTH || output.height != VIEWPORT_HEIGHT {
            return Err(BrowserAdapterError::Operation(format!(
                "fixed PNG screenshot dimensions were {}x{}, expected {}x{}",
                output.width, output.height, VIEWPORT_WIDTH, VIEWPORT_HEIGHT
            )));
        }
        Ok((output.width, output.height))
    }

    fn remaining(deadline: Instant) -> Result<Duration, BrowserAdapterError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(deadline_error())
        } else {
            Ok(remaining)
        }
    }

    fn ensure_before_deadline(deadline: Instant) -> Result<(), BrowserAdapterError> {
        if Instant::now() >= deadline {
            Err(deadline_error())
        } else {
            Ok(())
        }
    }

    fn deadline_error() -> BrowserAdapterError {
        BrowserAdapterError::Operation(
            "fixed browser smoke exceeded its 15-second deadline".to_string(),
        )
    }

    fn signal_process_group(process_group: Pid, signal: Signal) -> Result<(), Errno> {
        match killpg(process_group, signal) {
            Ok(()) | Err(Errno::ESRCH) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn wait_for_process_group_exit(process_group: Pid, limit: Duration) -> Result<bool, String> {
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
    fn process_group_is_alive(process_group: Pid) -> Result<bool, String> {
        Ok(!process_group_members(process_group)?.is_empty())
    }

    #[cfg(target_os = "macos")]
    fn process_group_is_alive(process_group: Pid) -> Result<bool, String> {
        let group_target = Pid::from_raw(-process_group.as_raw());
        match nix::sys::signal::kill(group_target, None) {
            Ok(()) | Err(Errno::EPERM) => Ok(true),
            Err(Errno::ESRCH) => Ok(false),
            Err(error) => Err(format!("inspect owned Chrome process group: {error}")),
        }
    }

    #[cfg(target_os = "linux")]
    fn process_group_members(process_group: Pid) -> Result<Vec<u32>, String> {
        let entries = fs::read_dir("/proc")
            .map_err(|error| format!("inspect owned Chrome process group: {error}"))?;
        let mut members = Vec::new();
        for entry in entries.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
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
                members.push(pid);
            }
        }
        members.sort_unstable();
        Ok(members)
    }

    fn setup_error<E: std::fmt::Display>(
        context: &'static str,
    ) -> impl FnOnce(E) -> BrowserAdapterError {
        move |error| BrowserAdapterError::Setup(format!("{context}: {error}"))
    }

    fn operation_error<E: std::fmt::Display>(
        context: &'static str,
    ) -> impl FnOnce(E) -> BrowserAdapterError {
        move |error| BrowserAdapterError::Operation(format!("{context}: {error}"))
    }

    fn operation_error_before_deadline<E: std::fmt::Display>(
        context: &'static str,
        deadline: Instant,
    ) -> impl FnOnce(E) -> BrowserAdapterError {
        move |error| {
            if Instant::now() >= deadline {
                deadline_error()
            } else {
                BrowserAdapterError::Operation(format!("{context}: {error}"))
            }
        }
    }

    #[cfg(test)]
    #[derive(Clone, Debug, Default)]
    struct TestObservation {
        root_pid: Option<u32>,
        process_group: Option<Pid>,
        fixture_address: Option<SocketAddrV4>,
        profile_path: PathBuf,
        downloads_path: PathBuf,
    }

    #[cfg(test)]
    static TEST_OBSERVATION: std::sync::OnceLock<
        std::sync::Mutex<Option<Arc<std::sync::Mutex<TestObservation>>>>,
    > = std::sync::OnceLock::new();

    #[cfg(test)]
    fn record_owned_resources(owned: &OwnedRun) {
        let slot = TEST_OBSERVATION.get_or_init(|| std::sync::Mutex::new(None));
        let Ok(slot_guard) = slot.lock() else {
            return;
        };
        let Some(observation) = slot_guard.as_ref() else {
            return;
        };
        let observation_lock = observation.lock();
        if let Ok(mut observation_guard) = observation_lock {
            if let Some(fixture) = &owned.fixture {
                observation_guard.fixture_address = Some(fixture.address);
            }
            if let Some(profile) = &owned.profile {
                observation_guard.profile_path = profile.path().to_path_buf();
            }
            if let Some(downloads) = &owned.downloads {
                observation_guard.downloads_path = downloads.path().to_path_buf();
            }
            if let Some(chrome) = &owned.chrome {
                observation_guard.root_pid = chrome.child.as_ref().map(Child::id);
                observation_guard.process_group = Some(chrome.process_group);
            }
        }
    }

    #[cfg(not(test))]
    fn record_owned_resources(_owned: &OwnedRun) {}

    #[cfg(test)]
    mod tests {
        use super::*;
        use png::{BitDepth, ColorType, Encoder};
        use std::{net::Shutdown, sync::MutexGuard};

        static BROWSER_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
            std::sync::OnceLock::new();

        #[test]
        fn fixture_serves_only_the_fixed_contract() {
            let _browser_guard = browser_test_guard();
            let fixture = FixtureServer::start(FixtureContent::Ready).expect("fixture starts");
            let address = fixture.address;
            let ok = send_request(
                fixture.address,
                b"GET /fixed-page.html HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            );
            let (head, body) = split_response(&ok);
            assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
            assert!(head.contains("Content-Type: text/html; charset=utf-8\r\n"));
            assert!(head.contains("Content-Length: 174\r\n"));
            assert_eq!(body, FIXTURE_HTML);

            let mut oversized = b"GET /fixed-page.html HTTP/1.1\r\nX-Fill: ".to_vec();
            oversized.extend_from_slice(&vec![b'x'; 8_192]);
            oversized.extend_from_slice(b"\r\n\r\n");
            let requests = [
                b"GET /other HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec(),
                b"GET /fixed-page.html?query=1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec(),
                b"POST /fixed-page.html HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec(),
                b"GET /fixed-page.html HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 4\r\n\r\n".to_vec(),
                b"GET /fixed-page.html HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(),
                b"GET /fixed-page.html HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\nDATA".to_vec(),
                b"not an HTTP request\r\n\r\n".to_vec(),
                b"GET /fixed-page.html HTTP/1.1\r\nHost: 127.0.0.1".to_vec(),
                oversized,
            ];
            for request in requests {
                let response = send_request(fixture.address, &request);
                let (head, body) = split_response(&response);
                assert!(head.starts_with("HTTP/1.1 404 Not Found\r\n"));
                assert!(body.is_empty());
            }
            fixture.stop().expect("fixture stops");
            assert!(TcpStream::connect(address).is_err());
        }

        #[test]
        fn discovery_uses_only_executable_regular_files_in_candidate_order() {
            let directory = TempDir::new().expect("temporary candidates");
            let missing = directory.path().join("missing");
            let candidate_directory = directory.path().join("directory");
            let non_executable = directory.path().join("non-executable");
            let first = directory.path().join("first");
            let second = directory.path().join("second");
            fs::create_dir(&candidate_directory).expect("candidate directory");
            fs::set_permissions(&candidate_directory, fs::Permissions::from_mode(0o755))
                .expect("executable directory permissions");
            fs::write(&non_executable, b"not executable").expect("non-executable candidate");
            fs::set_permissions(&non_executable, fs::Permissions::from_mode(0o644))
                .expect("non-executable permissions");
            fs::write(&first, b"first").expect("first candidate");
            fs::write(&second, b"second").expect("second candidate");
            fs::set_permissions(&first, fs::Permissions::from_mode(0o755))
                .expect("first executable");
            fs::set_permissions(&second, fs::Permissions::from_mode(0o755))
                .expect("second executable");

            let selected = discover_chrome_from([
                missing.as_path(),
                candidate_directory.as_path(),
                non_executable.as_path(),
                first.as_path(),
                second.as_path(),
            ])
            .expect("candidate selected");
            assert_eq!(selected, first);
            #[cfg(target_os = "linux")]
            assert_eq!(
                CHROME_CANDIDATES,
                [
                    "/usr/bin/google-chrome-stable",
                    "/usr/bin/google-chrome",
                    "/usr/bin/chromium",
                    "/usr/bin/chromium-browser",
                ]
            );
            #[cfg(target_os = "macos")]
            assert_eq!(
                CHROME_CANDIDATES,
                ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"]
            );
        }

        #[test]
        fn discovery_reports_the_fixed_missing_chrome_error() {
            let directory = TempDir::new().expect("temporary candidates");
            let missing = directory.path().join("missing");
            let non_executable = directory.path().join("non-executable");
            fs::write(&non_executable, b"not executable").expect("non-executable candidate");
            fs::set_permissions(&non_executable, fs::Permissions::from_mode(0o644))
                .expect("non-executable permissions");

            let error = discover_chrome_from([missing.as_path(), non_executable.as_path()])
                .expect_err("no candidate qualifies");
            assert!(matches!(error, BrowserAdapterError::ChromeNotFound));
            assert_eq!(
                error.to_string(),
                "no allowlisted system Chrome executable found"
            );
        }

        #[test]
        fn png_validation_rejects_invalid_data_and_accepts_fixed_dimensions() {
            assert!(validate_png(&[]).is_err());
            assert!(validate_png(&vec![0; MAX_SCREENSHOT_BYTES + 1]).is_err());
            assert!(validate_png(b"not a PNG").is_err());
            assert!(validate_png(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]).is_err());
            assert!(validate_png(&png_bytes(VIEWPORT_WIDTH - 1, VIEWPORT_HEIGHT)).is_err());
            assert!(validate_png(&png_bytes(VIEWPORT_WIDTH, VIEWPORT_HEIGHT - 1)).is_err());
            assert_eq!(
                validate_png(&png_bytes(VIEWPORT_WIDTH, VIEWPORT_HEIGHT))
                    .expect("fixed PNG validates"),
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
            );
        }

        #[test]
        fn rendered_text_decoding_accepts_only_json_strings_verbatim() {
            assert_eq!(
                decode_rendered_text(Some(json!("NOT READY")))
                    .expect("JSON string is accepted"),
                "NOT READY"
            );

            let invalid_values = [
                None,
                Some(serde_json::Value::Null),
                Some(json!(true)),
                Some(json!(42)),
                Some(json!(["READY"])),
                Some(json!({"status": "READY"})),
            ];
            for value in invalid_values {
                assert!(matches!(
                    decode_rendered_text(value),
                    Err(BrowserAdapterError::Operation(_))
                ));
            }
        }

        #[test]
        fn launch_failure_explicitly_cleans_every_acquired_resource() {
            let _browser_guard = browser_test_guard();
            let observation = Arc::new(std::sync::Mutex::new(TestObservation::default()));
            let _observation_guard = install_observer(Arc::clone(&observation));
            let directory = TempDir::new().expect("temporary missing executable parent");
            let mut options = RunOptions::production();
            options.executable = Some(directory.path().join("missing-chrome"));

            let error = run_with_options(options).expect_err("Chrome launch fails");
            assert!(matches!(error, BrowserAdapterError::Operation(_)));
            assert!(error
                .to_string()
                .contains("launch allowlisted system Chrome"));
            assert_observed_resources_cleaned(&observation, false);
        }

        #[test]
        fn real_browser_observes_non_matching_fixture_and_cleans_owned_resources() {
            let _browser_guard = browser_test_guard();
            let resources = Arc::new(std::sync::Mutex::new(TestObservation::default()));
            let _observation_guard = install_observer(Arc::clone(&resources));
            let mut options = RunOptions::production();
            options.fixture_content = FixtureContent::NotReady;

            let observation = run_with_options(options).expect("non-matching observation succeeds");
            let resources = resources.lock().expect("observation lock").clone();
            assert_eq!(
                observation.final_url,
                format!(
                    "http://127.0.0.1:{}{FIXTURE_PATH}",
                    resources
                        .fixture_address
                        .expect("fixture address recorded")
                        .port()
                )
            );
            assert_eq!(observation.observed_text, "NOT READY");
            assert_eq!(observation.screenshot.media_type, SCREENSHOT_MEDIA_TYPE);
            assert_eq!(observation.screenshot.width_px, VIEWPORT_WIDTH);
            assert_eq!(observation.screenshot.height_px, VIEWPORT_HEIGHT);
            assert!(!observation.screenshot.bytes.is_empty());
            assert!(observation.screenshot.bytes.len() <= MAX_SCREENSHOT_BYTES);
            assert_eq!(
                validate_png(&observation.screenshot.bytes).expect("observation PNG decodes"),
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
            );
            assert_observation_cleaned(&resources, true);
        }

        #[test]
        fn real_browser_rejects_redirect_before_observation_and_cleans_owned_resources() {
            let _browser_guard = browser_test_guard();
            let resources = Arc::new(std::sync::Mutex::new(TestObservation::default()));
            let _observation_guard = install_observer(Arc::clone(&resources));
            let mut options = RunOptions::production();
            options.fixture_content = FixtureContent::Redirect;
            options.screenshot_override = Some(b"not a PNG".to_vec());

            let error = run_with_options(options).expect_err("redirected final URL fails");
            match error {
                BrowserAdapterError::Operation(message) => {
                    assert!(message.contains("final URL was"));
                    assert!(message.contains(REDIRECT_PATH));
                    assert!(!message.contains("locate fixed status element"));
                    assert!(!message.contains("fixed PNG screenshot"));
                }
                other => panic!("unexpected redirect outcome: {other}"),
            }
            assert_observed_resources_cleaned(&resources, true);
        }

        #[test]
        fn real_browser_rejects_missing_selector_and_cleans_owned_resources() {
            let _browser_guard = browser_test_guard();
            let resources = Arc::new(std::sync::Mutex::new(TestObservation::default()));
            let _observation_guard = install_observer(Arc::clone(&resources));
            let mut options = RunOptions::production();
            options.fixture_content = FixtureContent::MissingSelector;

            let error = run_with_options(options).expect_err("missing selector fails");
            assert!(matches!(error, BrowserAdapterError::Operation(_)));
            assert!(error
                .to_string()
                .contains("locate fixed status element"));
            assert_observed_resources_cleaned(&resources, true);
        }

        #[test]
        fn screenshot_validation_failure_cleans_all_owned_resources() {
            let _browser_guard = browser_test_guard();
            let observation = Arc::new(std::sync::Mutex::new(TestObservation::default()));
            let _observation_guard = install_observer(Arc::clone(&observation));
            let mut options = RunOptions::production();
            options.screenshot_override = Some(b"not a PNG".to_vec());

            let error = run_with_options(options).expect_err("invalid screenshot fails");
            assert!(matches!(error, BrowserAdapterError::Operation(_)));
            assert!(error.to_string().contains("fixed PNG screenshot"));
            assert_observed_resources_cleaned(&observation, true);
        }

        #[test]
        fn deadline_failure_is_deterministic_and_cleans_all_owned_resources() {
            let _browser_guard = browser_test_guard();
            let observation = Arc::new(std::sync::Mutex::new(TestObservation::default()));
            let _observation_guard = install_observer(Arc::clone(&observation));
            let mut options = RunOptions::production();
            options.timeout = Duration::from_millis(1);

            let error = run_with_options(options).expect_err("short deadline expires");
            assert!(error
                .to_string()
                .contains("fixed browser smoke exceeded its 15-second deadline"));
            assert_observed_resources_cleaned(&observation, true);
        }

        #[test]
        fn injected_cleanup_failure_preserves_the_launch_failure() {
            let _browser_guard = browser_test_guard();
            let observation = Arc::new(std::sync::Mutex::new(TestObservation::default()));
            let _observation_guard = install_observer(Arc::clone(&observation));
            let directory = TempDir::new().expect("temporary missing executable parent");
            let mut options = RunOptions::production();
            options.executable = Some(directory.path().join("missing-chrome"));
            options.cleanup_failure = Some("injected cleanup failure");

            let error = run_with_options(options).expect_err("both failures are retained");
            match error {
                BrowserAdapterError::OperationAndCleanup { operation, cleanup } => {
                    assert!(operation.contains("launch allowlisted system Chrome"));
                    assert_eq!(cleanup, "injected cleanup failure");
                }
                other => panic!("unexpected combined outcome: {other}"),
            }
            assert_observed_resources_cleaned(&observation, false);
        }

        #[test]
        fn outcome_composition_covers_all_operation_and_cleanup_combinations() {
            assert!(combine_outcome(Ok(dummy_result()), Vec::new()).is_ok());

            let cleanup = combine_outcome(
                Ok(dummy_result()),
                vec!["cleanup failed independently".to_string()],
            )
            .expect_err("cleanup failure replaces success");
            assert!(matches!(cleanup, BrowserAdapterError::Cleanup(_)));

            let operation = combine_outcome(
                Err(BrowserAdapterError::Operation(
                    "operation failed independently".to_string(),
                )),
                Vec::new(),
            )
            .expect_err("operation failure is preserved");
            assert!(matches!(operation, BrowserAdapterError::Operation(_)));

            let combined = combine_outcome(
                Err(BrowserAdapterError::Operation(
                    "operation failed independently".to_string(),
                )),
                vec!["cleanup failed independently".to_string()],
            )
            .expect_err("both failures are preserved");
            match combined {
                BrowserAdapterError::OperationAndCleanup { operation, cleanup } => {
                    assert!(operation.contains("operation failed independently"));
                    assert_eq!(cleanup, "cleanup failed independently");
                }
                other => panic!("unexpected combined outcome: {other}"),
            }
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn owned_chrome_drop_kills_the_live_group_without_touching_unrelated_processes() {
            let _browser_guard = browser_test_guard();
            let unrelated = KillOnDrop::spawn("sleep", &["30"]);
            let unrelated_pid = unrelated.id();

            let mut command = Command::new("/bin/sh");
            command
                .args(["-c", "sleep 30 & wait"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0);
            let child = command.spawn().expect("owned process group starts");
            let process_group = Pid::from_raw(child.id() as i32);
            wait_for_group_size(process_group, 2);

            drop(OwnedChrome {
                child: Some(child),
                process_group,
                profile_path: PathBuf::new(),
                watchdog_done: None,
                watchdog: None,
                cleaned: false,
            });

            assert!(
                wait_for_process_group_exit(process_group, CLEANUP_LIMIT)
                    .expect("inspect dropped process group"),
                "owned process group still has live non-zombie members"
            );
            assert!(
                process_is_alive(unrelated_pid),
                "unrelated process was killed"
            );
        }

        #[test]
        fn real_browser_walks_fixed_fixture_and_cleans_owned_resources() {
            let _browser_guard = browser_test_guard();
            let observation = Arc::new(std::sync::Mutex::new(TestObservation::default()));
            let _observation_guard = install_observer(Arc::clone(&observation));

            let result = futures_executor::block_on(super::super::run_fixed_browser_smoke())
                .expect("real browser smoke succeeds");
            let observation = observation.lock().expect("observation lock").clone();
            assert_eq!(
                result.final_url,
                format!(
                    "http://127.0.0.1:{}{FIXTURE_PATH}",
                    observation
                        .fixture_address
                        .expect("fixture address recorded")
                        .port()
                )
            );
            assert_eq!(result.selector, fixed_selector());
            assert_eq!(result.expected_text, fixed_expected_text());
            assert_eq!(result.observed_text, fixed_expected_text());
            assert_eq!(result.screenshot.media_type, SCREENSHOT_MEDIA_TYPE);
            assert_eq!(result.screenshot.width_px, VIEWPORT_WIDTH);
            assert_eq!(result.screenshot.height_px, VIEWPORT_HEIGHT);
            assert!(!result.screenshot.bytes.is_empty());
            assert!(result.screenshot.bytes.len() <= MAX_SCREENSHOT_BYTES);
            assert_eq!(
                result.screenshot.bytes.get(..8),
                Some(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a][..])
            );
            assert_eq!(
                validate_png(&result.screenshot.bytes).expect("result PNG decodes"),
                (VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
            );
            assert_observation_cleaned(&observation, true);
        }

        fn png_bytes(width: u32, height: u32) -> Vec<u8> {
            let mut bytes = Vec::new();
            {
                let mut encoder = Encoder::new(&mut bytes, width, height);
                encoder.set_color(ColorType::Rgb);
                encoder.set_depth(BitDepth::Eight);
                let mut writer = encoder.write_header().expect("PNG header");
                writer
                    .write_image_data(&vec![0; width as usize * height as usize * 3])
                    .expect("PNG pixels");
            }
            bytes
        }

        fn dummy_result() -> FixedBrowserObservation {
            FixedBrowserObservation {
                final_url: "http://127.0.0.1:1/fixed-page.html".to_string(),
                observed_text: fixed_expected_text().to_string(),
                screenshot: FixedPngScreenshot {
                    bytes: Vec::new(),
                    media_type: SCREENSHOT_MEDIA_TYPE.to_string(),
                    width_px: VIEWPORT_WIDTH,
                    height_px: VIEWPORT_HEIGHT,
                },
            }
        }

        fn send_request(address: SocketAddrV4, request: &[u8]) -> Vec<u8> {
            let mut stream = TcpStream::connect(address).expect("connect to fixture");
            stream.write_all(request).expect("write fixture request");
            stream.shutdown(Shutdown::Write).expect("finish request");
            let mut response = Vec::new();
            stream
                .read_to_end(&mut response)
                .expect("read fixture response");
            response
        }

        fn split_response(response: &[u8]) -> (String, &[u8]) {
            let boundary = response
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .expect("response header boundary");
            (
                String::from_utf8(response[..boundary + 4].to_vec())
                    .expect("ASCII response headers"),
                &response[boundary + 4..],
            )
        }

        fn assert_observed_resources_cleaned(
            observation: &Arc<std::sync::Mutex<TestObservation>>,
            expected_process_group: bool,
        ) {
            let observation = observation.lock().expect("observation lock").clone();
            assert_observation_cleaned(&observation, expected_process_group);
        }

        fn assert_observation_cleaned(observation: &TestObservation, expected_process_group: bool) {
            let fixture_address = observation
                .fixture_address
                .expect("fixture address was recorded");
            assert!(TcpStream::connect(fixture_address).is_err());
            assert!(!observation.profile_path.exists());
            assert!(!observation.downloads_path.exists());
            if expected_process_group {
                assert!(observation.process_group.is_some());
            } else {
                assert!(observation.process_group.is_none());
            }
            if let Some(process_group) = observation.process_group {
                assert!(
                    !process_group_is_alive(process_group)
                        .expect("inspect owned process group after return"),
                    "owned process group still has live non-zombie members"
                );
            }
            if let Some(root_pid) = observation.root_pid {
                assert!(
                    !process_is_alive(root_pid),
                    "owned Chrome root is still alive"
                );
            }
        }

        #[cfg(target_os = "linux")]
        fn process_is_alive(pid: u32) -> bool {
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
                return false;
            };
            stat.rsplit_once(')')
                .and_then(|(_, fields)| fields.split_whitespace().next())
                != Some("Z")
        }

        #[cfg(target_os = "macos")]
        fn process_is_alive(pid: u32) -> bool {
            match nix::sys::signal::kill(Pid::from_raw(pid as i32), None) {
                Ok(()) | Err(Errno::EPERM) => true,
                Err(Errno::ESRCH) => false,
                Err(_) => true,
            }
        }

        #[cfg(target_os = "linux")]
        fn wait_for_group_size(process_group: Pid, minimum: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let members =
                    process_group_members(process_group).expect("inspect test-owned process group");
                if members.len() >= minimum {
                    return;
                }
                assert!(Instant::now() < deadline, "owned descendant did not start");
                thread::sleep(IO_POLL_INTERVAL);
            }
        }

        fn browser_test_guard() -> MutexGuard<'static, ()> {
            BROWSER_TEST_LOCK
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .expect("browser test lock")
        }

        fn install_observer(observer: Arc<std::sync::Mutex<TestObservation>>) -> ObserverGuard {
            let slot = TEST_OBSERVATION.get_or_init(|| std::sync::Mutex::new(None));
            let mut slot = slot.lock().expect("test observation slot");
            assert!(slot.is_none(), "only one browser observation may be active");
            *slot = Some(observer);
            drop(slot);
            ObserverGuard
        }

        struct ObserverGuard;

        impl Drop for ObserverGuard {
            fn drop(&mut self) {
                let slot = TEST_OBSERVATION.get_or_init(|| std::sync::Mutex::new(None));
                if let Ok(mut slot) = slot.lock() {
                    *slot = None;
                }
            }
        }

        #[cfg(target_os = "linux")]
        struct KillOnDrop(Child);

        #[cfg(target_os = "linux")]
        impl KillOnDrop {
            fn spawn(executable: &str, arguments: &[&str]) -> Self {
                Self(
                    Command::new(executable)
                        .args(arguments)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .expect("unrelated process starts"),
                )
            }

            fn id(&self) -> u32 {
                self.0.id()
            }
        }

        #[cfg(target_os = "linux")]
        impl Drop for KillOnDrop {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
}
