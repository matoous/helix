use chrono::Local;
use log::Level;
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Write},
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub const DEFAULT_LOG_MAX_LINES: usize = 20000;

/// Identifies which subsystem produced a log stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogKind {
    Helix,
    Lsp,
    Dap,
}

impl LogKind {
    pub fn label(&self) -> &'static str {
        match self {
            LogKind::Helix => "helix",
            LogKind::Lsp => "lsp",
            LogKind::Dap => "dap",
        }
    }
}

/// Metadata describing a log buffer that can be opened by the UI.
#[derive(Debug, Clone)]
pub struct LogBufferItem {
    pub kind: LogKind,
    pub name: String,
}

/// A single log line emitted by a named logger.
#[derive(Debug, Clone)]
pub struct LogEvent {
    pub kind: LogKind,
    pub name: String,
    pub line: String,
}

/// Central in-memory log store with line-capped buffers.
#[derive(Clone)]
pub struct LogHub {
    inner: Arc<Mutex<HubInner>>,
}

struct HubInner {
    buffers: HashMap<LogKey, VecDeque<String>>,
    max_lines: usize,
    sender: UnboundedSender<LogEvent>,
}

/// Unique key for a log buffer in the hub.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LogKey {
    kind: LogKind,
    name: String,
}

impl LogHub {
    /// Create a new hub with a maximum number of lines per buffer.
    pub fn new(max_lines: usize) -> (Self, UnboundedReceiver<LogEvent>) {
        let (sender, receiver) = unbounded_channel();
        let inner = HubInner {
            buffers: HashMap::new(),
            max_lines,
            sender,
        };
        (
            Self {
                inner: Arc::new(Mutex::new(inner)),
            },
            receiver,
        )
    }

    /// Create a named logger bound to this hub.
    pub fn logger(&self, kind: LogKind, name: impl Into<String>) -> Logger {
        Logger {
            hub: self.clone(),
            kind,
            name: name.into(),
        }
    }

    /// Create an `io::Write` sink that forwards lines into this hub.
    pub fn writer(&self, kind: LogKind, name: impl Into<String>) -> LogWriter {
        LogWriter {
            hub: self.clone(),
            kind,
            name: name.into(),
            buffer: Vec::new(),
        }
    }

    /// List the currently known log buffers.
    pub fn buffers(&self) -> Vec<LogBufferItem> {
        let inner = self.inner.lock().unwrap();
        inner
            .buffers
            .keys()
            .map(|key| LogBufferItem {
                kind: key.kind,
                name: key.name.clone(),
            })
            .collect()
    }

    /// Ensure a buffer exists for a named logger.
    pub fn ensure_buffer(&self, kind: LogKind, name: &str) {
        let mut inner = self.inner.lock().unwrap();
        let key = LogKey {
            kind,
            name: name.to_string(),
        };
        inner.buffers.entry(key).or_default();
    }

    /// Return the full in-memory content of a log buffer, if any.
    pub fn buffer_content(&self, kind: LogKind, name: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        let key = LogKey {
            kind,
            name: name.to_string(),
        };
        inner
            .buffers
            .get(&key)
            .map(|lines| lines.iter().cloned().collect())
    }

    /// Returns the configured maximum line count per buffer.
    pub fn max_lines(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.max_lines
    }

    fn append_line(&self, kind: LogKind, name: &str, line: String) {
        let mut inner = self.inner.lock().unwrap();
        let key = LogKey {
            kind,
            name: name.to_string(),
        };
        let max_lines = inner.max_lines;
        let buffer = inner.buffers.entry(key.clone()).or_default();
        buffer.push_back(line.clone());
        while buffer.len() > max_lines {
            buffer.pop_front();
        }

        let _ = inner.sender.send(LogEvent {
            kind,
            name: key.name,
            line,
        });
    }
}

#[derive(Clone)]
pub struct Logger {
    hub: LogHub,
    kind: LogKind,
    name: String,
}

impl Logger {
    /// Log an informational message.
    pub fn info(&self, message: &str) {
        self.log(Level::Info, message);
    }

    /// Log a warning message.
    pub fn warn(&self, message: &str) {
        self.log(Level::Warn, message);
    }

    /// Log an error message.
    pub fn error(&self, message: &str) {
        self.log(Level::Error, message);
    }

    /// Log a debug message.
    pub fn debug(&self, message: &str) {
        self.log(Level::Debug, message);
    }

    /// Log a trace message.
    pub fn trace(&self, message: &str) {
        self.log(Level::Trace, message);
    }

    fn log(&self, level: Level, message: &str) {
        let line = format!(
            "{} {} [{:?}] {}\n",
            Local::now().format("%Y-%m-%dT%H:%M:%S%.3f"),
            self.name,
            level,
            message
        );
        self.hub.append_line(self.kind, &self.name, line);
    }
}

pub struct LogWriter {
    hub: LogHub,
    kind: LogKind,
    name: String,
    buffer: Vec<u8>,
}

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        while let Some(pos) = self.buffer.iter().position(|&b| b == b'\n') {
            let line = self.buffer.drain(..=pos).collect::<Vec<u8>>();
            let line = String::from_utf8_lossy(&line).into_owned();
            self.hub.append_line(self.kind, &self.name, line);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            let line = String::from_utf8_lossy(&self.buffer).into_owned();
            self.buffer.clear();
            self.hub.append_line(self.kind, &self.name, line);
        }
        Ok(())
    }
}
