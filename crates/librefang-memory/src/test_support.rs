//! Test-only helpers for exercising SQLite write-lock contention against a file-backed pool configured like production.

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

/// A file-backed pool with the production connection pragmas (WAL, `busy_timeout=5000`) and migrations applied, so two connections can genuinely contend for the write lock.
/// An in-memory pool pinned to one connection cannot contend at all, which is why the deferred-transaction bug below never showed up in this crate's tests.
pub(crate) fn file_backed_pool(path: &Path) -> Pool<SqliteConnectionManager> {
    let manager = SqliteConnectionManager::file(path)
        .with_init(|c| c.execute_batch(crate::substrate::DEFAULT_CONNECTION_PRAGMAS));
    let pool = Pool::builder().max_size(4).build(manager).unwrap();
    crate::migration::run_migrations(&pool.get().unwrap()).unwrap();
    pool
}

/// Take the database write lock on a separate connection with `BEGIN IMMEDIATE`, hold it for `hold`, then roll back.
/// Returns once the lock is held; join the handle to wait for its release.
///
/// A transaction that begins as a reader and only later writes cannot wait for this lock: SQLite returns `SQLITE_BUSY` at the read-to-write upgrade without consulting the busy handler, because two readers each waiting for the other to upgrade would deadlock.
/// A transaction opened with `BEGIN IMMEDIATE` takes the write lock up front, where the busy handler does apply, so it waits under `busy_timeout` and then proceeds.
pub(crate) fn hold_write_lock(path: &Path, hold: Duration) -> JoinHandle<()> {
    let path = path.to_path_buf();
    let (locked_tx, locked_rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        locked_tx.send(()).unwrap();
        std::thread::sleep(hold);
        conn.execute_batch("ROLLBACK").unwrap();
    });
    locked_rx.recv().unwrap();
    handle
}
