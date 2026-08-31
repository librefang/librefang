//! Retry helpers for filesystem calls the OS can interrupt.
//!
//! `EINTR` (`io::ErrorKind::Interrupted`) means a syscall was interrupted by a signal *before it did anything* — the caller is expected to reissue it.
//! Surfacing it is a bug rather than a filesystem fault, and the message the OS supplies ("Interrupted system call") reads like a mount outage, which sends operators looking in the wrong place (#8050).
//!
//! It shows up in practice on slow, cloud-synced mounts (macOS FileProvider, iCloud-backed `~/Documents`): the directory read stays in the kernel long enough that a signal from a busy async runtime is likely to land mid-call.
//! Before this module a single interruption ended the tool call, and the agent saw it 6/6 times on a directory a concurrent `os.listdir()` loop read successfully 160/160 times — Python retries `EINTR` transparently (PEP 475), which is the behaviour reproduced here.

use std::future::Future;
use std::io;
use std::path::Path;

/// How many times an interrupted call is reissued before the error is surfaced.
/// `EINTR` is transient by definition, so a small bound is enough to absorb a burst of signals while still terminating if something is wedged into delivering them continuously.
const MAX_INTERRUPT_RETRIES: usize = 5;

fn is_interrupted(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::Interrupted
}

/// Run `op`, reissuing it while it reports `EINTR`.
///
/// `op` is a closure rather than a future so each attempt starts a fresh call: a future that already returned `Err` cannot be polled again.
async fn retry_on_interrupt<T, F, Fut>(mut op: F) -> io::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = io::Result<T>>,
{
    let mut retries = 0;
    loop {
        match op().await {
            Err(e) if is_interrupted(&e) && retries < MAX_INTERRUPT_RETRIES => {
                retries += 1;
            }
            other => return other,
        }
    }
}

/// `tokio::fs::read_dir` that survives an interrupted syscall.
pub(super) async fn read_dir(path: &Path) -> io::Result<tokio::fs::ReadDir> {
    retry_on_interrupt(|| tokio::fs::read_dir(path)).await
}

/// `ReadDir::next_entry` that survives an interrupted syscall.
///
/// Written as a loop rather than through [`retry_on_interrupt`] because `next_entry` borrows the reader mutably, and a closure returning a future that holds a `&mut` borrow of its own captured state does not type-check.
pub(super) async fn next_entry(
    rd: &mut tokio::fs::ReadDir,
) -> io::Result<Option<tokio::fs::DirEntry>> {
    let mut retries = 0;
    loop {
        match rd.next_entry().await {
            Err(e) if is_interrupted(&e) && retries < MAX_INTERRUPT_RETRIES => {
                retries += 1;
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn eintr() -> io::Error {
        io::Error::from(io::ErrorKind::Interrupted)
    }

    #[tokio::test]
    async fn retries_past_a_burst_of_interruptions_and_returns_the_value() {
        let attempts = Cell::new(0usize);
        let out = retry_on_interrupt(|| {
            let n = attempts.get();
            attempts.set(n + 1);
            async move {
                if n < MAX_INTERRUPT_RETRIES {
                    Err(eintr())
                } else {
                    Ok(7u8)
                }
            }
        })
        .await;
        assert_eq!(out.unwrap(), 7);
        assert_eq!(attempts.get(), MAX_INTERRUPT_RETRIES + 1);
    }

    #[tokio::test]
    async fn gives_up_after_the_bound_instead_of_spinning_forever() {
        let attempts = Cell::new(0usize);
        let out: io::Result<u8> = retry_on_interrupt(|| {
            attempts.set(attempts.get() + 1);
            async { Err(eintr()) }
        })
        .await;
        assert_eq!(out.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(attempts.get(), MAX_INTERRUPT_RETRIES + 1);
    }

    #[tokio::test]
    async fn a_non_interrupt_error_is_surfaced_on_the_first_attempt() {
        let attempts = Cell::new(0usize);
        let out: io::Result<u8> = retry_on_interrupt(|| {
            attempts.set(attempts.get() + 1);
            async { Err(io::Error::from(io::ErrorKind::PermissionDenied)) }
        })
        .await;
        assert_eq!(out.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(attempts.get(), 1, "a hard error must not be retried");
    }

    #[tokio::test]
    async fn read_dir_and_next_entry_still_enumerate_a_real_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"b").unwrap();

        let mut rd = read_dir(dir.path()).await.unwrap();
        let mut names = Vec::new();
        while let Some(entry) = next_entry(&mut rd).await.unwrap() {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
        names.sort();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
    }
}
