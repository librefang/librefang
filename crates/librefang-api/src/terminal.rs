//! Terminal PTY abstraction layer.

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use tokio::sync::mpsc;
use tracing::info;

pub struct PtySession {
    _master: Box<dyn portable_pty::MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send>,
    pub writer: Box<dyn Write + Send>,
    pub pid: u32,
    pub shell: String,
}

impl PtySession {
    pub fn spawn(
        cols: Option<u16>,
        rows: Option<u16>,
    ) -> std::io::Result<(Self, mpsc::Receiver<Vec<u8>>)> {
        let pty_system = native_pty_system();

        let (shell, _flag) = shell_for_current_os();
        info!(shell = %shell, "spawning PTY shell");

        let pair = pty_system
            .openpty(PtySize {
                rows: rows.unwrap_or(40),
                cols: cols.unwrap_or(120),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(std::io::Error::other)?;

        let mut cmd = CommandBuilder::new(shell.clone());
        // No args — spawn an interactive shell

        // Set CWD to the user's home directory so the shell does not inherit
        // the daemon's working directory, which would expose server internals.
        let home_dir = {
            #[cfg(windows)]
            {
                std::env::var("USERPROFILE")
                    .or_else(|_| std::env::var("HOME"))
                    .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned())
            }
            #[cfg(not(windows))]
            {
                std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string())
            }
        };
        cmd.cwd(home_dir);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(std::io::Error::other)?;
        let pid = child.process_id().unwrap_or(0);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(std::io::Error::other)?;
        let (tx, rx) = mpsc::channel(1024);

        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = buf[..n].to_vec();
                        if tx.blocking_send(data).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "PTY read error");
                        break;
                    }
                }
            }
        });

        let writer = pair.master.take_writer().map_err(std::io::Error::other)?;

        Ok((
            Self {
                _master: pair.master,
                child,
                writer,
                pid,
                shell: shell.clone(),
            },
            rx,
        ))
    }

    /// Spawn a PTY attached to the LibreFang tmux session.
    ///
    /// Unlike `spawn()` which creates an interactive shell directly, this method
    /// attaches to the tmux session via `tmux attach -t <session_name>`. The
    /// `TMUX` environment variable is explicitly cleared to prevent nested sessions.
    ///
    /// The caller must have already called `TmuxController::ensure_session()`.
    pub fn spawn_tmux_attached(
        tmux_path: &str,
        session_name: &str,
        cols: Option<u16>,
        rows: Option<u16>,
    ) -> std::io::Result<(Self, mpsc::Receiver<Vec<u8>>)> {
        let pty_system = native_pty_system();
        info!(tmux = %tmux_path, session = %session_name, "spawning PTY attached to tmux");

        let pair = pty_system
            .openpty(PtySize {
                rows: rows.unwrap_or(40),
                cols: cols.unwrap_or(120),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(std::io::Error::other)?;

        let mut cmd = CommandBuilder::new(tmux_path);
        cmd.arg("-L");
        cmd.arg("librefang");
        cmd.arg("-f");
        cmd.arg("/dev/null");
        cmd.arg("attach");
        cmd.arg("-t");
        cmd.arg(session_name);

        // Clear TMUX env to prevent nesting when daemon inherits user's tmux session.
        cmd.env("TMUX", "");

        // CWD = $HOME, not daemon's working directory.
        let home_dir = {
            #[cfg(windows)]
            {
                std::env::var("USERPROFILE")
                    .or_else(|_| std::env::var("HOME"))
                    .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned())
            }
            #[cfg(not(windows))]
            {
                std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string())
            }
        };
        cmd.cwd(home_dir);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(std::io::Error::other)?;
        let pid = child.process_id().unwrap_or(0);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(std::io::Error::other)?;
        let (tx, rx) = mpsc::channel(1024);

        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = buf[..n].to_vec();
                        if tx.blocking_send(data).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "tmux PTY read error");
                        break;
                    }
                }
            }
        });

        let writer = pair.master.take_writer().map_err(std::io::Error::other)?;

        let shell_name = std::path::Path::new(tmux_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("tmux")
            .to_string();

        Ok((
            Self {
                _master: pair.master,
                child,
                writer,
                pid,
                shell: shell_name,
            },
            rx,
        ))
    }

    pub fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Resize the underlying PTY. portable-pty 0.9 implements
    /// `MasterPty::resize` on every backend it supports, including the
    /// Windows ConPTY path (`ConPtyMasterPty`), so the same call works
    /// cross-platform. Closes #2303.
    pub fn resize(&mut self, cols: u16, rows: u16) -> std::io::Result<()> {
        self._master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(std::io::Error::other)
    }

    /// Kill the child process. Errors are silently ignored (process may already be gone).
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    /// Wait for the child process to exit and return (exit_code, optional_signal).
    pub fn wait_exit(&mut self) -> std::io::Result<(u32, Option<String>)> {
        let status = self.child.wait().map_err(std::io::Error::other)?;
        Ok((status.exit_code(), status.signal().map(|s| s.to_string())))
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

pub fn shell_for_current_os() -> (String, &'static str) {
    // The flag (e.g., "-c" on Unix, "/C" on Windows) is the "execute command" flag,
    // unused here since we spawn an interactive shell without command arguments.
    #[cfg(windows)]
    {
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
        (shell, "/C")
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        (shell, "-c")
    }
}

/// Check if the daemon is running as root (Unix only).
/// Always returns false on Windows.
pub fn is_running_as_root() -> bool {
    #[cfg(unix)]
    {
        rustix::process::geteuid().is_root()
    }
    #[cfg(not(unix))]
    {
        false
    }
}
