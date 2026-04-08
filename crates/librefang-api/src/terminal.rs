//! Terminal PTY abstraction layer.

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use tokio::sync::mpsc;
use tracing::info;

pub struct PtySession {
    _master: Box<dyn portable_pty::MasterPty + Send>,
    _child: Box<dyn portable_pty::Child + Send>,
    pub writer: Box<dyn Write + Send>,
    pub pid: u32,
    pub shell: String,
}

impl PtySession {
    pub fn spawn() -> std::io::Result<(Self, mpsc::Receiver<Vec<u8>>)> {
        let pty_system = native_pty_system();

        let (shell, flag) = shell_for_current_os();
        info!(shell = %shell, flag = %flag, "spawning PTY shell");

        let pair = pty_system
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let mut cmd = CommandBuilder::new(shell.clone());
        cmd.args([flag, "exec 0"]);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let pid = child.process_id().unwrap_or(0);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
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

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        Ok((
            Self {
                _master: pair.master,
                _child: child,
                writer,
                pid,
                shell: shell.clone(),
            },
            rx,
        ))
    }

    pub fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    #[cfg(windows)]
    pub fn resize(&mut self, _cols: u16, _rows: u16) -> std::io::Result<()> {
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn resize(&mut self, cols: u16, rows: u16) -> std::io::Result<()> {
        self._master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}

pub fn shell_for_current_os() -> (String, &'static str) {
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
