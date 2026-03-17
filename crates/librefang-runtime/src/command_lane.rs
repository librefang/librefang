//! Command lane system — lane-based command queue with concurrency control.
//!
//! Routes different types of work through separate lanes with independent
//! concurrency limits to prevent starvation:
//! - Main: user messages (3 concurrent by default)
//! - Cron: scheduled jobs (2 concurrent)
//! - Subagent: spawned child agents (3 concurrent)

use std::sync::Arc;
use tokio::sync::Semaphore;

/// Command lane type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// User-facing message processing (3 concurrent by default).
    Main,
    /// Cron/scheduled job execution (2 concurrent).
    Cron,
    /// Subagent spawn/call execution (3 concurrent).
    Subagent,
}

impl std::fmt::Display for Lane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Lane::Main => write!(f, "main"),
            Lane::Cron => write!(f, "cron"),
            Lane::Subagent => write!(f, "subagent"),
        }
    }
}

/// Lane occupancy snapshot.
#[derive(Debug, Clone)]
pub struct LaneOccupancy {
    /// Lane type.
    pub lane: Lane,
    /// Current number of active tasks.
    pub active: u32,
    /// Maximum concurrent tasks.
    pub capacity: u32,
}

/// Command queue with lane-based concurrency control.
#[derive(Debug, Clone)]
pub struct CommandQueue {
    main_sem: Arc<Semaphore>,
    cron_sem: Arc<Semaphore>,
    subagent_sem: Arc<Semaphore>,
    main_capacity: u32,
    cron_capacity: u32,
    subagent_capacity: u32,
}

impl CommandQueue {
    /// Create a new command queue with default capacities.
    pub fn new() -> Self {
        Self {
            main_sem: Arc::new(Semaphore::new(3)),
            cron_sem: Arc::new(Semaphore::new(2)),
            subagent_sem: Arc::new(Semaphore::new(3)),
            main_capacity: 3,
            cron_capacity: 2,
            subagent_capacity: 3,
        }
    }

    /// Create with custom capacities.
    pub fn with_capacities(main: u32, cron: u32, subagent: u32) -> Self {
        Self {
            main_sem: Arc::new(Semaphore::new(main as usize)),
            cron_sem: Arc::new(Semaphore::new(cron as usize)),
            subagent_sem: Arc::new(Semaphore::new(subagent as usize)),
            main_capacity: main,
            cron_capacity: cron,
            subagent_capacity: subagent,
        }
    }

    /// Submit work to a lane. Acquires a permit, executes the future, releases.
    ///
    /// Returns `Err` if the semaphore is closed (shutdown).
    pub async fn submit<F, T>(&self, lane: Lane, work: F) -> Result<T, String>
    where
        F: std::future::Future<Output = T>,
    {
        let sem = self.semaphore_for(lane);
        let _permit = sem
            .acquire()
            .await
            .map_err(|_| format!("Lane {} is closed", lane))?;

        Ok(work.await)
    }

    /// Try to submit work without waiting (non-blocking).
    ///
    /// Returns `None` if the lane is at capacity.
    pub async fn try_submit<F, T>(&self, lane: Lane, work: F) -> Option<T>
    where
        F: std::future::Future<Output = T>,
    {
        let sem = self.semaphore_for(lane);
        let _permit = sem.try_acquire().ok()?;
        Some(work.await)
    }

    /// Get current occupancy for all lanes.
    pub fn occupancy(&self) -> Vec<LaneOccupancy> {
        vec![
            LaneOccupancy {
                lane: Lane::Main,
                active: self.main_capacity - self.main_sem.available_permits() as u32,
                capacity: self.main_capacity,
            },
            LaneOccupancy {
                lane: Lane::Cron,
                active: self.cron_capacity - self.cron_sem.available_permits() as u32,
                capacity: self.cron_capacity,
            },
            LaneOccupancy {
                lane: Lane::Subagent,
                active: self.subagent_capacity - self.subagent_sem.available_permits() as u32,
                capacity: self.subagent_capacity,
            },
        ]
    }

    fn semaphore_for(&self, lane: Lane) -> &Arc<Semaphore> {
        match lane {
            Lane::Main => &self.main_sem,
            Lane::Cron => &self.cron_sem,
            Lane::Subagent => &self.subagent_sem,
        }
    }
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_main_lane_submit() {
        let queue = CommandQueue::new();
        let counter = Arc::new(AtomicU32::new(0));

        // Main lane accepts and executes tasks
        let c1 = counter.clone();
        let result = queue
            .submit(Lane::Main, async move {
                c1.fetch_add(1, Ordering::SeqCst);
                42
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_cron_lane_parallel() {
        let queue = Arc::new(CommandQueue::new());
        let counter = Arc::new(AtomicU32::new(0));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let q = queue.clone();
            let c = counter.clone();
            handles.push(tokio::spawn(async move {
                q.submit(Lane::Cron, async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                })
                .await
            }));
        }

        for h in handles {
            h.await.unwrap().unwrap();
        }
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_occupancy() {
        let queue = CommandQueue::new();
        let occ = queue.occupancy();
        assert_eq!(occ.len(), 3);
        assert_eq!(occ[0].active, 0);
        assert_eq!(occ[0].capacity, 3);
        assert_eq!(occ[1].capacity, 2);
        assert_eq!(occ[2].capacity, 3);
    }

    #[tokio::test]
    async fn test_try_submit_when_full() {
        let queue = CommandQueue::with_capacities(1, 1, 1);

        // Acquire the main permit
        let sem = queue.main_sem.clone();
        let _permit = sem.acquire().await.unwrap();

        // try_submit should return None since lane is full
        let result = queue.try_submit(Lane::Main, async { 42 }).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_custom_capacities() {
        let queue = CommandQueue::with_capacities(2, 4, 6);
        let occ = queue.occupancy();
        assert_eq!(occ[0].capacity, 2);
        assert_eq!(occ[1].capacity, 4);
        assert_eq!(occ[2].capacity, 6);
    }
}
