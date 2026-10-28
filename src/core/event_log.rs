use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::core::events::SessionEvent;
use crate::core::session::Session;

/// Errors from writing or reading the JSONL session event log.
#[derive(Debug, thiserror::Error)]
pub enum EventLogError {
    #[error("event log io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("event log parse error on line {line}: {message}")]
    Parse { line: usize, message: String },
    #[error("event log sequence error: expected seq {expected}, got {got}")]
    Sequence { expected: u64, got: u64 },
}

/// Appends session events to a new JSONL file, flushing after each event.
#[derive(Debug)]
pub struct EventLogWriter {
    writer: BufWriter<File>,
    path: PathBuf,
    written: usize,
}

impl EventLogWriter {
    /// Creates a new event log file, failing if it already exists.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, EventLogError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self {
            writer: BufWriter::new(file),
            path,
            written: 0,
        })
    }

    /// Writes one event as a JSON line and flushes it to disk.
    pub fn append(&mut self, event: &SessionEvent) -> Result<(), EventLogError> {
        let expected = self.written as u64;
        if event.seq != expected {
            return Err(EventLogError::Sequence {
                expected,
                got: event.seq,
            });
        }
        let line = serde_json::to_string(event)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        self.written += 1;
        Ok(())
    }

    /// Writes only the session events not yet written by this writer.
    pub fn write_pending(&mut self, session: &Session) -> Result<usize, EventLogError> {
        let events = session.events();
        if self.written >= events.len() {
            return Ok(0);
        }
        let pending = &events[self.written..];
        let count = pending.len();
        for event in pending {
            self.append(event)?;
        }
        Ok(count)
    }

    /// Returns how many events this writer has written.
    pub fn written_count(&self) -> usize {
        self.written
    }

    /// Returns the path of the event log file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Reads a whole JSONL event log, validating contiguous seq from 0.
pub fn read_events(path: impl AsRef<Path>) -> Result<Vec<SessionEvent>, EventLogError> {
    let content = std::fs::read_to_string(path)?;
    let mut lines: Vec<&str> = content.lines().collect();
    if matches!(lines.last(), Some(line) if line.trim().is_empty()) {
        lines.pop();
    }
    let mut events = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let line_no = index + 1;
        if line.trim().is_empty() {
            return Err(EventLogError::Parse {
                line: line_no,
                message: "blank line".to_string(),
            });
        }
        let event: SessionEvent = serde_json::from_str(line).map_err(|e| EventLogError::Parse {
            line: line_no,
            message: e.to_string(),
        })?;
        let expected = events.len() as u64;
        if event.seq != expected {
            return Err(EventLogError::Sequence {
                expected,
                got: event.seq,
            });
        }
        events.push(event);
    }
    Ok(events)
}
