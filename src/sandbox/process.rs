use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use super::error::SandboxError;

/// Default cap for each captured output stream.
pub const DEFAULT_MAX_OUTPUT: usize = 64 * 1024;

/// Cap for each captured stream of an in-container execution.
pub const EXEC_MAX_OUTPUT: usize = 256 * 1024;

/// An argv-only command. It must only ever hold validated non-secret values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    /// Builds a spec from a program and string-convertible arguments.
    pub fn new<I, S>(program: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    /// Returns a space-joined description for error context.
    pub fn describe(&self) -> String {
        let mut out = self.program.clone();
        for arg in &self.args {
            out.push(' ');
            out.push_str(arg);
        }
        out
    }
}

/// Bounded result of a finished command. A non-zero exit is not an error here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

impl CommandOutcome {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Reads a stream keeping at most `max` bytes while draining the rest.
async fn read_capped<R>(mut reader: R, max: usize) -> (Vec<u8>, bool)
where
    R: AsyncRead + Unpin,
{
    let mut kept: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let chunk = &buf[..n];
                let room = max.saturating_sub(kept.len());
                if chunk.len() > room {
                    truncated = true;
                }
                kept.extend_from_slice(&chunk[..room.min(chunk.len())]);
            }
        }
    }
    (kept, truncated)
}

/// Runs `spec` without a shell, bounding time and each captured stream.
pub async fn run_bounded(
    spec: &CommandSpec,
    stdin: Option<&[u8]>,
    timeout: Duration,
    max_output: usize,
) -> Result<CommandOutcome, SandboxError> {
    let program = spec.program.clone();
    let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    let deadline = Instant::now() + timeout;
    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .kill_on_drop(true)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SandboxError::ExecutableNotFound {
                    program: program.clone(),
                }
            } else {
                SandboxError::Io(e.to_string())
            }
        })?;
    let stdin_data = stdin.map(<[u8]>::to_vec);
    let mut stdin_pipe = child.stdin.take();
    let stdin_task = tokio::spawn(async move {
        if let (Some(mut pipe), Some(data)) = (stdin_pipe.take(), stdin_data) {
            let _ = pipe.write_all(&data).await;
            let _ = pipe.shutdown().await;
        }
    });
    let stdout_pipe = child.stdout.take().expect("stdout piped");
    let stderr_pipe = child.stderr.take().expect("stderr piped");
    let stdout_task = tokio::spawn(read_capped(stdout_pipe, max_output));
    let stderr_task = tokio::spawn(read_capped(stderr_pipe, max_output));
    let timed_out = || SandboxError::Timeout {
        program: program.clone(),
        timeout_ms,
    };
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            stdin_task.abort();
            stdout_task.abort();
            stderr_task.abort();
            return Err(SandboxError::Io(e.to_string()));
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            stdin_task.abort();
            stdout_task.abort();
            stderr_task.abort();
            return Err(timed_out());
        }
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let joined = tokio::time::timeout(remaining, async {
        let _ = stdin_task.await;
        let out = stdout_task.await;
        let err = stderr_task.await;
        (out, err)
    })
    .await;
    let (out, err) = match joined {
        Ok(pair) => pair,
        Err(_) => return Err(timed_out()),
    };
    let (stdout, stdout_truncated) = out.unwrap_or_default();
    let (stderr, stderr_truncated) = err.unwrap_or_default();
    Ok(CommandOutcome {
        exit_code: status.code(),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(program: &str, args: Vec<String>) -> CommandSpec {
        CommandSpec::new(program, args)
    }

    #[tokio::test]
    async fn run_bounded_captures_echo_stdout() {
        let out = run_bounded(
            &spec("echo", vec!["hello".to_string()]),
            None,
            Duration::from_secs(5),
            DEFAULT_MAX_OUTPUT,
        )
        .await
        .expect("echo runs");
        assert!(out.success());
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout_text(), "hello\n");
        assert!(!out.stdout_truncated);
    }

    #[tokio::test]
    async fn run_bounded_reports_nonzero_exit() {
        let out = run_bounded(
            &spec("false", vec![]),
            None,
            Duration::from_secs(5),
            DEFAULT_MAX_OUTPUT,
        )
        .await
        .expect("false runs");
        assert!(!out.success());
        assert_eq!(out.exit_code, Some(1));
    }

    #[tokio::test]
    async fn run_bounded_missing_executable_typed() {
        let err = run_bounded(
            &spec("pairproof-no-such-program-xyz", vec![]),
            None,
            Duration::from_secs(5),
            DEFAULT_MAX_OUTPUT,
        )
        .await
        .expect_err("missing program must error");
        assert_eq!(
            err,
            SandboxError::ExecutableNotFound {
                program: "pairproof-no-such-program-xyz".to_string()
            }
        );
    }

    #[tokio::test]
    async fn run_bounded_times_out_and_kills() {
        let started = Instant::now();
        let err = run_bounded(
            &spec("sleep", vec!["5".to_string()]),
            None,
            Duration::from_millis(100),
            DEFAULT_MAX_OUTPUT,
        )
        .await
        .expect_err("sleep must time out");
        assert_eq!(
            err,
            SandboxError::Timeout {
                program: "sleep".to_string(),
                timeout_ms: 100
            }
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn run_bounded_truncates_output() {
        let format = "a".repeat(5000);
        let out = run_bounded(
            &spec("/usr/bin/printf", vec![format]),
            None,
            Duration::from_secs(5),
            100,
        )
        .await
        .expect("printf runs");
        assert!(out.success());
        assert!(out.stdout.len() <= 100);
        assert!(out.stdout_truncated);
    }

    #[tokio::test]
    async fn run_bounded_feeds_stdin() {
        let out = run_bounded(
            &spec("cat", vec![]),
            Some(b"piped input"),
            Duration::from_secs(5),
            DEFAULT_MAX_OUTPUT,
        )
        .await
        .expect("cat runs");
        assert_eq!(out.stdout_text(), "piped input");
        assert_eq!(out.stderr_text(), "");
    }

    #[test]
    fn command_spec_describe_joins_argv() {
        let spec = CommandSpec::new("docker", ["--version"]);
        assert_eq!(spec.describe(), "docker --version");
        assert_eq!(spec.program, "docker");
        assert_eq!(spec.args, vec!["--version".to_string()]);
    }
}
