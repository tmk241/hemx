use std::collections::VecDeque;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(25);
const DEFAULT_OUTPUT_LIMIT: usize = 64 * 1024;

/// Build and start a child process with an explicit readiness contract.
pub struct TestProcessBuilder {
    command: Command,
    label: String,
    readiness: Option<Readiness>,
    timeout: Duration,
    poll_interval: Duration,
    output_limit: usize,
}

impl fmt::Debug for TestProcessBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TestProcessBuilder")
            .field("command", &self.command)
            .field("label", &self.label)
            .field("readiness", &self.readiness)
            .field("timeout", &self.timeout)
            .field("poll_interval", &self.poll_interval)
            .field("output_limit", &self.output_limit)
            .finish()
    }
}

impl TestProcessBuilder {
    /// Create a builder around a real process command.
    pub fn new(command: Command) -> Self {
        let label = command.get_program().to_string_lossy().into_owned();
        Self {
            command,
            label,
            readiness: None,
            timeout: DEFAULT_TIMEOUT,
            poll_interval: DEFAULT_POLL_INTERVAL,
            output_limit: DEFAULT_OUTPUT_LIMIT,
        }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn arg(mut self, argument: impl AsRef<OsStr>) -> Self {
        self.command.arg(argument);
        self
    }

    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(arguments);
        self
    }

    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.command.env(key, value);
        self
    }

    /// Wait until a TCP connection can be established.
    ///
    /// A listener already occupying the address also satisfies this probe. Use
    /// [`Self::http`] when readiness must identify an application endpoint.
    pub fn tcp(mut self, address: impl Into<String>) -> Self {
        self.readiness = Some(Readiness::Tcp {
            address: address.into(),
        });
        self
    }

    /// Wait until a plain HTTP endpoint returns a status from 200 through 399.
    pub fn http(mut self, address: impl Into<String>, path: impl Into<String>) -> Self {
        self.readiness = Some(Readiness::Http {
            address: address.into(),
            path: path.into(),
        });
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Bound retained output per stream. Readers continue draining after the
    /// limit is reached so a noisy child cannot deadlock on a full pipe.
    pub fn output_limit(mut self, bytes: usize) -> Self {
        self.output_limit = bytes;
        self
    }

    /// Spawn the process and wait for its readiness contract.
    pub fn start(mut self) -> Result<TestProcess, ProcessError> {
        if self.poll_interval.is_zero() {
            return Err(ProcessError::configuration(
                &self.label,
                "poll interval must be greater than zero",
            ));
        }
        let readiness = self.readiness.ok_or_else(|| {
            ProcessError::configuration(&self.label, "choose TCP or HTTP readiness before start")
        })?;
        let readiness = readiness.resolve(&self.label)?;
        let readiness_description = readiness.description();

        self.command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = self.command.spawn().map_err(|source| ProcessError::Spawn {
            label: self.label.clone(),
            source,
        })?;
        let stdout = child
            .stdout
            .take()
            .map(|stream| CapturedOutput::spawn(stream, self.output_limit));
        let stderr = child
            .stderr
            .take()
            .map(|stream| CapturedOutput::spawn(stream, self.output_limit));
        let mut process = TestProcess {
            child: Some(child),
            label: self.label.clone(),
            stdout,
            stderr,
            exit_status: None,
        };
        let started = Instant::now();
        let deadline = started + self.timeout;
        let mut attempts = 0usize;

        loop {
            attempts += 1;
            let remaining = deadline.saturating_duration_since(Instant::now());
            let probe_timeout = remaining.min(self.poll_interval);
            if readiness.probe(probe_timeout) {
                return Ok(process);
            }

            if let Some(status) = process.try_wait().map_err(|source| ProcessError::Monitor {
                label: self.label.clone(),
                source,
            })? {
                process.finish_capture();
                return Err(ProcessError::EarlyExit {
                    status,
                    failure: Box::new(ProcessFailure {
                        label: self.label,
                        readiness: readiness_description,
                        attempts,
                        stdout: process.stdout(),
                        stderr: process.stderr(),
                    }),
                });
            }

            if Instant::now() >= deadline {
                let cleanup_error = process.shutdown().err().map(|error| error.to_string());
                return Err(ProcessError::TimedOut {
                    timeout: self.timeout,
                    failure: Box::new(ProcessFailure {
                        label: self.label,
                        readiness: readiness_description,
                        attempts,
                        stdout: process.stdout(),
                        stderr: process.stderr(),
                    }),
                    cleanup_error,
                });
            }

            thread::sleep(self.poll_interval.min(remaining));
        }
    }
}

/// A real child process owned by an integration test.
///
/// Explicit shutdown and `Drop` are idempotent. Both kill a running child, wait
/// for it, drain captured output, and release the process handle.
pub struct TestProcess {
    child: Option<Child>,
    label: String,
    stdout: Option<CapturedOutput>,
    stderr: Option<CapturedOutput>,
    exit_status: Option<ExitStatus>,
}

impl fmt::Debug for TestProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TestProcess")
            .field("label", &self.label)
            .field("id", &self.id())
            .field("exit_status", &self.exit_status)
            .finish_non_exhaustive()
    }
}

impl TestProcess {
    pub fn builder(command: Command) -> TestProcessBuilder {
        TestProcessBuilder::new(command)
    }

    /// Compatibility entry point for TCP-ready processes.
    pub fn start(
        command: Command,
        label: impl Into<String>,
        address: &str,
        timeout: Duration,
    ) -> io::Result<Self> {
        Self::builder(command)
            .label(label)
            .tcp(address)
            .timeout(timeout)
            .start()
            .map_err(ProcessError::into_io)
    }

    pub fn id(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.exit_status
    }

    /// Return the bounded, currently captured standard output.
    pub fn stdout(&self) -> String {
        self.stdout
            .as_ref()
            .map(CapturedOutput::snapshot)
            .unwrap_or_default()
    }

    /// Return the bounded, currently captured standard error.
    pub fn stderr(&self) -> String {
        self.stderr
            .as_ref()
            .map(CapturedOutput::snapshot)
            .unwrap_or_default()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.exit_status {
            return Ok(Some(status));
        }
        let Some(child) = self.child.as_mut() else {
            return Ok(self.exit_status);
        };
        let status = child.try_wait()?;
        if let Some(status) = status {
            self.exit_status = Some(status);
        }
        Ok(status)
    }

    /// Kill a running child, wait for it, and drain output. Calling this more
    /// than once is harmless.
    pub fn shutdown(&mut self) -> io::Result<()> {
        if let Some(mut child) = self.child.take() {
            let status = match child.try_wait()? {
                Some(status) => status,
                None => {
                    if let Err(error) = child.kill() {
                        if error.kind() != io::ErrorKind::InvalidInput {
                            return Err(error);
                        }
                    }
                    child.wait()?
                }
            };
            self.exit_status = Some(status);
        }
        self.finish_capture();
        Ok(())
    }

    fn finish_capture(&mut self) {
        if let Some(capture) = self.stdout.as_mut() {
            capture.finish();
        }
        if let Some(capture) = self.stderr.as_mut() {
            capture.finish();
        }
    }
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// Captured context for an early exit or readiness timeout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessFailure {
    pub label: String,
    pub readiness: String,
    pub attempts: usize,
    pub stdout: String,
    pub stderr: String,
}

/// Failure to configure, start, observe, or clean up a test process.
#[derive(Debug)]
pub enum ProcessError {
    Configuration {
        label: String,
        message: String,
    },
    Spawn {
        label: String,
        source: io::Error,
    },
    Monitor {
        label: String,
        source: io::Error,
    },
    EarlyExit {
        status: ExitStatus,
        failure: Box<ProcessFailure>,
    },
    TimedOut {
        timeout: Duration,
        failure: Box<ProcessFailure>,
        cleanup_error: Option<String>,
    },
}

impl ProcessError {
    fn configuration(label: &str, message: &str) -> Self {
        Self::Configuration {
            label: label.to_owned(),
            message: message.to_owned(),
        }
    }

    fn into_io(self) -> io::Error {
        let kind = match &self {
            Self::Spawn { source, .. } | Self::Monitor { source, .. } => source.kind(),
            Self::TimedOut { .. } => io::ErrorKind::TimedOut,
            Self::Configuration { .. } => io::ErrorKind::InvalidInput,
            Self::EarlyExit { .. } => io::ErrorKind::Other,
        };
        io::Error::new(kind, self)
    }
}

impl fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration { label, message } => {
                write!(formatter, "invalid readiness for {label}: {message}")
            }
            Self::Spawn { label, source } => write!(formatter, "failed to spawn {label}: {source}"),
            Self::Monitor { label, source } => {
                write!(formatter, "failed to observe {label}: {source}")
            }
            Self::EarlyExit { status, failure } => write!(
                formatter,
                "{} exited with {status} before {} after {} readiness attempts{}{}",
                failure.label,
                failure.readiness,
                failure.attempts,
                output_section("stdout", &failure.stdout),
                output_section("stderr", &failure.stderr)
            ),
            Self::TimedOut {
                timeout,
                failure,
                cleanup_error,
            } => write!(
                formatter,
                "timed out after {timeout:?} waiting for {} to satisfy {} after {} readiness attempts{}{}{}",
                failure.label,
                failure.readiness,
                failure.attempts,
                output_section("stdout", &failure.stdout),
                output_section("stderr", &failure.stderr),
                cleanup_error
                    .as_deref()
                    .map(|error| format!("\ncleanup error:\n{error}"))
                    .unwrap_or_default()
            ),
        }
    }
}

impl Error for ProcessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::Monitor { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
enum Readiness {
    Tcp { address: String },
    Http { address: String, path: String },
}

impl Readiness {
    fn resolve(self, label: &str) -> Result<ResolvedReadiness, ProcessError> {
        match self {
            Self::Tcp { address } => Ok(ResolvedReadiness::Tcp {
                socket: resolve_address(label, &address)?,
                address,
            }),
            Self::Http { address, path } => {
                if !path.starts_with('/') {
                    return Err(ProcessError::configuration(
                        label,
                        "HTTP readiness path must start with '/'",
                    ));
                }
                Ok(ResolvedReadiness::Http {
                    socket: resolve_address(label, &address)?,
                    address,
                    path,
                })
            }
        }
    }
}

#[derive(Clone, Debug)]
enum ResolvedReadiness {
    Tcp {
        address: String,
        socket: SocketAddr,
    },
    Http {
        address: String,
        socket: SocketAddr,
        path: String,
    },
}

impl ResolvedReadiness {
    fn description(&self) -> String {
        match self {
            Self::Tcp { address, .. } => format!("TCP readiness on {address}"),
            Self::Http { address, path, .. } => {
                format!("HTTP readiness at http://{address}{path}")
            }
        }
    }

    fn probe(&self, timeout: Duration) -> bool {
        if timeout.is_zero() {
            return false;
        }
        match self {
            Self::Tcp { socket, .. } => TcpStream::connect_timeout(socket, timeout).is_ok(),
            Self::Http {
                address,
                socket,
                path,
            } => probe_http(*socket, address, path, timeout),
        }
    }
}

fn resolve_address(label: &str, address: &str) -> Result<SocketAddr, ProcessError> {
    address
        .to_socket_addrs()
        .map_err(|error| {
            ProcessError::configuration(label, &format!("could not resolve {address:?}: {error}"))
        })?
        .next()
        .ok_or_else(|| ProcessError::configuration(label, &format!("{address:?} resolved empty")))
}

fn probe_http(socket: SocketAddr, host: &str, path: &str, timeout: Duration) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&socket, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = Vec::with_capacity(128);
    while response.len() < 128 && !response.contains(&b'\n') {
        let mut chunk = [0_u8; 32];
        let Ok(read) = stream.read(&mut chunk) else {
            return false;
        };
        if read == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..read]);
    }
    let Ok(status_line) = std::str::from_utf8(&response) else {
        return false;
    };
    status_line
        .split_whitespace()
        .nth(1)
        .and_then(|status| status.parse::<u16>().ok())
        .is_some_and(|status| (200..400).contains(&status))
}

struct CapturedOutput {
    buffer: Arc<Mutex<CapturedBuffer>>,
    reader: Option<JoinHandle<()>>,
}

impl CapturedOutput {
    fn spawn(mut stream: impl Read + Send + 'static, limit: usize) -> Self {
        let buffer = Arc::new(Mutex::new(CapturedBuffer::new(limit)));
        let reader_buffer = Arc::clone(&buffer);
        let reader = thread::spawn(move || {
            let mut chunk = [0_u8; 4096];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => reader_buffer
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(&chunk[..read]),
                }
            }
        });
        Self {
            buffer,
            reader: Some(reader),
        }
    }

    fn snapshot(&self) -> String {
        self.buffer
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .render()
    }

    fn finish(&mut self) {
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

struct CapturedBuffer {
    bytes: VecDeque<u8>,
    limit: usize,
    omitted: usize,
}

impl CapturedBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::with_capacity(limit.min(4096)),
            limit,
            omitted: 0,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        self.bytes.extend(bytes);
        while self.bytes.len() > self.limit {
            self.bytes.pop_front();
            self.omitted += 1;
        }
    }

    fn render(&self) -> String {
        let bytes = self.bytes.iter().copied().collect::<Vec<_>>();
        let output = String::from_utf8_lossy(&bytes);
        if self.omitted == 0 {
            output.into_owned()
        } else {
            format!("<{} earlier bytes omitted>\n{output}", self.omitted)
        }
    }
}

fn output_section(name: &str, output: &str) -> String {
    if output.is_empty() {
        String::new()
    } else {
        format!("\n{name}:\n{output}")
    }
}
