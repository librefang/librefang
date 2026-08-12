//! Bounded subprocess helpers for CLI-backed LLM drivers.

use std::process::Output;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time::{timeout_at, Instant};

pub(crate) const DEFAULT_MESSAGE_TIMEOUT_SECS: u64 = 300;

pub(crate) fn timeout_error(timeout_secs: u64, driver: &str) -> crate::llm_driver::LlmError {
    crate::llm_driver::LlmError::TimedOut {
        inactivity_secs: timeout_secs,
        partial_text: None,
        partial_text_len: 0,
        last_activity: format!("waiting for {driver} subprocess"),
    }
}

pub(crate) enum OutputError {
    Io(std::io::Error),
    TimedOut,
}

pub(crate) struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> AbortOnDrop<T> {
    pub(crate) fn new(task: JoinHandle<T>) -> Self {
        Self(task)
    }

    pub(crate) fn abort(&self) {
        self.0.abort();
    }

    pub(crate) async fn join(&mut self) -> Result<T, tokio::task::JoinError> {
        (&mut self.0).await
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn read_pipe<R>(pipe: Option<R>) -> std::io::Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut bytes).await?;
    }
    Ok(bytes)
}

fn join_error(error: tokio::task::JoinError) -> std::io::Error {
    std::io::Error::other(format!("subprocess output task failed: {error}"))
}

async fn collect_pipe(
    task: &mut AbortOnDrop<std::io::Result<Vec<u8>>>,
) -> std::io::Result<Vec<u8>> {
    (&mut task.0).await.map_err(join_error)?
}

/// Run a command while draining both output pipes and enforcing a hard deadline.
///
/// On timeout the child is killed and reaped before this function returns. Pipe
/// readers are aborted as well, so descendants that inherited stdout/stderr
/// cannot keep the request alive after the direct child has exited.
pub(crate) async fn output_with_timeout(
    command: &mut Command,
    duration: Duration,
) -> Result<Output, OutputError> {
    command.kill_on_drop(true);
    let mut child = command.spawn().map_err(OutputError::Io)?;
    let mut stdout_task = AbortOnDrop::new(tokio::spawn(read_pipe(child.stdout.take())));
    let mut stderr_task = AbortOnDrop::new(tokio::spawn(read_pipe(child.stderr.take())));
    let deadline = Instant::now() + duration;

    let status = match timeout_at(deadline, child.wait()).await {
        Ok(result) => result.map_err(OutputError::Io)?,
        Err(_) => {
            let _ = child.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(OutputError::TimedOut);
        }
    };

    let pipes = timeout_at(deadline, async {
        tokio::try_join!(
            collect_pipe(&mut stdout_task),
            collect_pipe(&mut stderr_task)
        )
    })
    .await;

    match pipes {
        Ok(Ok((stdout, stderr))) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        Ok(Err(error)) => Err(OutputError::Io(error)),
        Err(_) => {
            stdout_task.abort();
            stderr_task.abort();
            Err(OutputError::TimedOut)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timeout_bounds_child_and_inherited_output_pipes() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30 & wait"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let started = Instant::now();
        let result = output_with_timeout(&mut command, Duration::from_millis(50)).await;

        assert!(matches!(result, Err(OutputError::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
