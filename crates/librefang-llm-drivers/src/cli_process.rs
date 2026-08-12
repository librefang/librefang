//! Bounded subprocess helpers for CLI-backed LLM drivers.

use std::process::Output;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time::{timeout_at, Instant};

pub(crate) const DEFAULT_MESSAGE_TIMEOUT_SECS: u64 = 300;

pub(crate) fn timeout_error(timeout_secs: u64, driver: &str) -> crate::llm_driver::LlmError {
    timeout_error_with_partial(timeout_secs, driver, String::new())
}

/// Same as [`timeout_error`], but for streaming callers that have already
/// accumulated assistant text before the deadline hit (see #3552) — passing
/// it through means the timeout error carries the same partial-output detail
/// the non-CLI streaming drivers already surface, instead of silently
/// downgrading to an empty `partial_text` just because this call happened to
/// go through the CLI-subprocess path.
pub(crate) fn timeout_error_with_partial(
    timeout_secs: u64,
    driver: &str,
    partial_text: String,
) -> crate::llm_driver::LlmError {
    let partial_text_len = partial_text.len();
    crate::llm_driver::LlmError::TimedOut {
        inactivity_secs: timeout_secs,
        partial_text: if partial_text.is_empty() {
            None
        } else {
            Some(std::sync::Arc::from(partial_text))
        },
        partial_text_len,
        last_activity: format!("waiting for {driver} subprocess"),
    }
}

pub(crate) enum OutputError {
    /// The subprocess itself never started (binary missing, exec permission
    /// denied, …). Distinct from `Io` below so callers can keep pointing
    /// operators at "install the CLI" guidance only for this case, not for
    /// a failure that happened after the process was already running.
    Spawn(std::io::Error),
    /// The subprocess started successfully but a later step — reaping its
    /// exit status or draining its output pipes — failed.
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

/// Make the spawned child the leader of its own process group (Unix only).
///
/// CLI drivers here (codex/gemini/qwen) shell out to Node-based wrapper
/// binaries that can themselves fork helper/background processes. Without
/// this, `kill_on_timeout` below can only reach the direct child's pid —
/// any grandchild it backgrounded inherits our pgid, survives the kill, and
/// leaks as an orphan for the rest of its natural runtime (observed with a
/// plain `sleep 30 &` grandchild surviving a 100ms-timeout kill of the
/// direct child). `process_group(0)` makes the child pid its own pgid, so
/// `kill(-pid, ...)` in `kill_on_timeout` reaches the whole subtree.
#[cfg(unix)]
pub(crate) fn set_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
pub(crate) fn set_process_group(_command: &mut Command) {}

/// Kill the child and, on Unix, its entire process group so that any
/// grandchildren it forked (see `set_process_group`) are also reaped rather
/// than left running as orphans past the deadline.
#[cfg(unix)]
pub(crate) async fn kill_on_timeout(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        // SAFETY: `kill` only delivers a signal; `set_process_group` made
        // this child its own group leader at spawn time, so `-pid` is
        // guaranteed to target only this child's own subtree, never an
        // unrelated process group.
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
    let _ = child.kill().await;
}

#[cfg(windows)]
pub(crate) async fn kill_on_timeout(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        // `taskkill /T` walks Windows' own parent-pid tree, so no
        // equivalent to `process_group(0)` is needed at spawn time.
        let _ = tokio::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .output()
            .await;
    }
    let _ = child.kill().await;
}

#[cfg(not(any(unix, windows)))]
pub(crate) async fn kill_on_timeout(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
}

/// Run a command while draining both output pipes and enforcing a hard deadline.
///
/// On timeout the child (and, where the platform allows, its whole process tree — see `kill_on_timeout`) is killed and reaped before this function returns.
/// Pipe readers are aborted as well, so descendants that inherited stdout/stderr cannot keep the request alive after the direct child has exited.
pub(crate) async fn output_with_timeout(
    command: &mut Command,
    duration: Duration,
) -> Result<Output, OutputError> {
    output_with_optional_input_timeout(command, None, duration).await
}

/// Run a command with private stdin input while draining output and enforcing
/// the same hard deadline as [`output_with_timeout`].
///
/// The input is written after spawning and stdin is then closed so one-shot
/// CLI programs see EOF. Keeping input out of argv prevents prompts from being
/// exposed through process listings and avoids platform argument-size limits.
pub(crate) async fn output_with_input_timeout(
    command: &mut Command,
    input: &[u8],
    duration: Duration,
) -> Result<Output, OutputError> {
    command.stdin(std::process::Stdio::piped());
    output_with_optional_input_timeout(command, Some(input), duration).await
}

async fn output_with_optional_input_timeout(
    command: &mut Command,
    input: Option<&[u8]>,
    duration: Duration,
) -> Result<Output, OutputError> {
    command.kill_on_drop(true);
    set_process_group(command);
    let mut child = command.spawn().map_err(OutputError::Spawn)?;
    let mut stdout_task = AbortOnDrop::new(tokio::spawn(read_pipe(child.stdout.take())));
    let mut stderr_task = AbortOnDrop::new(tokio::spawn(read_pipe(child.stderr.take())));
    let deadline = Instant::now() + duration;

    if let Some(input) = input {
        let write_result = timeout_at(deadline, async {
            let mut stdin = child.stdin.take().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "subprocess stdin was not available",
                )
            })?;
            stdin.write_all(input).await?;
            stdin.shutdown().await
        })
        .await;

        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                kill_on_timeout(&mut child).await;
                stdout_task.abort();
                stderr_task.abort();
                return Err(OutputError::Io(error));
            }
            Err(_) => {
                kill_on_timeout(&mut child).await;
                stdout_task.abort();
                stderr_task.abort();
                return Err(OutputError::TimedOut);
            }
        }
    }

    let status = match timeout_at(deadline, child.wait()).await {
        Ok(result) => result.map_err(OutputError::Io)?,
        Err(_) => {
            kill_on_timeout(&mut child).await;
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
            kill_on_timeout(&mut child).await;
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

    #[tokio::test]
    async fn input_helper_writes_stdin_and_closes_it() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "input=$(cat); printf '%s' \"$input\""])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let result =
            output_with_input_timeout(&mut command, b"private prompt", Duration::from_secs(2))
                .await;
        let output = match result {
            Ok(output) => output,
            Err(_) => panic!("stdin subprocess helper failed"),
        };

        assert!(output.status.success());
        assert_eq!(output.stdout, b"private prompt");
    }

    /// A process that received SIGKILL but has not yet been reaped by its
    /// (possibly reparented) parent shows up as a `Z` (zombie) entry in
    /// `/proc/<pid>/stat` — dead, holding only a slot in the process table,
    /// not the resource-consuming leak `timeout_kills_the_whole_process_group_not_just_the_direct_child`
    /// below guards against. Treat only a non-zombie, still-`kill(pid, 0)`-able
    /// pid as genuinely running.
    fn is_still_running(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 only probes liveness; no signal is delivered.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        // Format: "<pid> (<comm>) <state> ...". `comm` can itself contain
        // spaces or parens, so split on the *last* ')' rather than the
        // first, then take the token right after it.
        stat.rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            != Some("Z")
    }

    /// A CLI-backed driver's subprocess can itself background a helper
    /// process (Node child processes, MCP helper servers, …). Killing only
    /// the direct child pid leaves that grandchild running as an orphan for
    /// the rest of its natural lifetime, because it never belonged to the
    /// direct child in the process sense — it only inherited the same pgid.
    /// `set_process_group` + `kill_on_timeout` must reach it too.
    #[tokio::test]
    async fn timeout_kills_the_whole_process_group_not_just_the_direct_child() {
        let pid_file = tempfile::NamedTempFile::new().unwrap();
        let pid_path = pid_file.path().to_str().unwrap().to_string();

        let mut command = Command::new("sh");
        command
            .args(["-c", &format!("sleep 30 & echo $! > {pid_path}; wait")])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let result = output_with_timeout(&mut command, Duration::from_millis(100)).await;
        assert!(matches!(result, Err(OutputError::TimedOut)));

        // Give the signal a moment to land, then confirm the backgrounded
        // `sleep 30` — never a direct child of `output_with_timeout`, only a
        // process-group sibling — died with the rest of the group.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let grandchild_pid: libc::pid_t = std::fs::read_to_string(&pid_path)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            !is_still_running(grandchild_pid),
            "grandchild process leaked past the timeout"
        );
    }
}
