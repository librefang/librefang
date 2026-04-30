//! Command lane system — lane-based command queue with concurrency control.
//!
//! Routes different types of work through separate lanes with independent
//! concurrency limits to prevent starvation:
//! - Main: user messages (3 concurrent by default)
//! - Cron: scheduled jobs (2 concurrent)
//! - Subagent: spawned child agents (3 concurrent)
//! - Trigger: event-trigger dispatches (8 concurrent)

use std::sync::{Arc, RwLock};
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
    /// Event-trigger dispatch — `TaskPosted`, `MessageReceived`, etc.
    /// fired against the kernel by `task_post`/event-bus callers.
    /// Bounded globally so a runaway producer can't spawn unbounded
    /// tokio tasks racing for the per-agent semaphore.
    Trigger,
}

impl std::fmt::Display for Lane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Lane::Main => write!(f, "main"),
            Lane::Cron => write!(f, "cron"),
            Lane::Subagent => write!(f, "subagent"),
            Lane::Trigger => write!(f, "trigger"),
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

/// One lane's semaphore + the capacity that produced it.
///
/// Wrapped in an `Arc<RwLock<_>>` inside [`CommandQueue`] so a config
/// reload can atomically swap in a fresh semaphore with the new
/// capacity. In-flight permits remain valid against the **old**
/// semaphore — they release into a slot that nobody else can acquire,
/// which is fine: the slot just disappears once the last drains.
#[derive(Debug)]
struct LaneSlot {
    sem: Arc<Semaphore>,
    capacity: u32,
}

/// Command queue with lane-based concurrency control.
#[derive(Debug, Clone)]
pub struct CommandQueue {
    main: Arc<RwLock<LaneSlot>>,
    cron: Arc<RwLock<LaneSlot>>,
    subagent: Arc<RwLock<LaneSlot>>,
    trigger: Arc<RwLock<LaneSlot>>,
}

impl CommandQueue {
    /// Create a new command queue with default capacities.
    pub fn new() -> Self {
        Self::with_capacities(3, 2, 3, 8)
    }

    /// Create with custom capacities.
    pub fn with_capacities(main: u32, cron: u32, subagent: u32, trigger: u32) -> Self {
        Self {
            main: Arc::new(RwLock::new(LaneSlot {
                sem: Arc::new(Semaphore::new(main as usize)),
                capacity: main,
            })),
            cron: Arc::new(RwLock::new(LaneSlot {
                sem: Arc::new(Semaphore::new(cron as usize)),
                capacity: cron,
            })),
            subagent: Arc::new(RwLock::new(LaneSlot {
                sem: Arc::new(Semaphore::new(subagent as usize)),
                capacity: subagent,
            })),
            trigger: Arc::new(RwLock::new(LaneSlot {
                sem: Arc::new(Semaphore::new(trigger as usize)),
                capacity: trigger,
            })),
        }
    }

    /// Borrow the semaphore for a lane. Useful when callers need an
    /// **owned** permit (`acquire_owned()`) so it can be moved into a
    /// detached `tokio::spawn` task — the returned `Arc<Semaphore>` is
    /// cheap to clone.
    pub fn semaphore_for_lane(&self, lane: Lane) -> Arc<Semaphore> {
        self.slot(lane)
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .sem
            .clone()
    }

    /// Atomically swap in a fresh semaphore for `lane` sized to `new_capacity`
    /// (#3628 — config hot-reload of `queue.concurrency.*`).
    ///
    /// In-flight permits keep their old semaphore alive until they
    /// drain; new acquirers see the new semaphore immediately. A no-op
    /// when the capacity is unchanged so reloads that didn't touch the
    /// concurrency block don't churn the queue.
    pub fn resize_lane(&self, lane: Lane, new_capacity: u32) -> bool {
        let new_capacity = new_capacity.max(1);
        let mut guard = self.slot(lane).write().unwrap_or_else(|e| e.into_inner());
        if guard.capacity == new_capacity {
            return false;
        }
        guard.sem = Arc::new(Semaphore::new(new_capacity as usize));
        guard.capacity = new_capacity;
        true
    }

    /// Submit work to a lane. Acquires a permit, executes the future, releases.
    ///
    /// Returns `Err` if the semaphore is closed (shutdown).
    pub async fn submit<F, T>(&self, lane: Lane, work: F) -> Result<T, String>
    where
        F: std::future::Future<Output = T>,
    {
        let sem = self.semaphore_for_lane(lane);
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
        let sem = self.semaphore_for_lane(lane);
        let _permit = sem.try_acquire().ok()?;
        Some(work.await)
    }

    /// Get current occupancy for all lanes.
    ///
    /// Note: `active` may transiently undercount during a [`resize_lane`]
    /// call. In-flight permits hold a reference to the *old* semaphore and
    /// release into it; the new semaphore starts with a fresh counter, so
    /// until the old permits drain, `active` for the resized lane will
    /// read lower than the true number of in-flight tasks.
    pub fn occupancy(&self) -> Vec<LaneOccupancy> {
        let mk = |lane: Lane| {
            let g = self.slot(lane).read().unwrap_or_else(|e| e.into_inner());
            LaneOccupancy {
                lane,
                active: g.capacity - g.sem.available_permits() as u32,
                capacity: g.capacity,
            }
        };
        vec![
            mk(Lane::Main),
            mk(Lane::Cron),
            mk(Lane::Subagent),
            mk(Lane::Trigger),
        ]
    }

    fn slot(&self, lane: Lane) -> &Arc<RwLock<LaneSlot>> {
        match lane {
            Lane::Main => &self.main,
            Lane::Cron => &self.cron,
            Lane::Subagent => &self.subagent,
            Lane::Trigger => &self.trigger,
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
        assert_eq!(occ.len(), 4);
        assert_eq!(occ[0].lane, Lane::Main);
        assert_eq!(occ[0].active, 0);
        assert_eq!(occ[0].capacity, 3);
        assert_eq!(occ[1].lane, Lane::Cron);
        assert_eq!(occ[1].capacity, 2);
        assert_eq!(occ[2].lane, Lane::Subagent);
        assert_eq!(occ[2].capacity, 3);
        assert_eq!(occ[3].lane, Lane::Trigger);
        assert_eq!(occ[3].capacity, 8);
    }

    #[tokio::test]
    async fn test_trigger_lane_caps_concurrency() {
        // Lane::Trigger with capacity 2 — third concurrent caller waits.
        let queue = Arc::new(CommandQueue::with_capacities(3, 2, 3, 2));
        let trigger_sem = queue.semaphore_for_lane(Lane::Trigger);

        // Burn both permits, then prove a third try_acquire fails.
        let p1 = trigger_sem.clone().try_acquire_owned().unwrap();
        let p2 = trigger_sem.clone().try_acquire_owned().unwrap();
        assert!(trigger_sem.clone().try_acquire_owned().is_err());

        // Occupancy reports both slots active.
        let occ = queue.occupancy();
        let trigger = occ.iter().find(|o| o.lane == Lane::Trigger).unwrap();
        assert_eq!(trigger.active, 2);
        assert_eq!(trigger.capacity, 2);

        drop(p1);
        drop(p2);
        assert!(trigger_sem.try_acquire_owned().is_ok());
    }

    #[tokio::test]
    async fn test_semaphore_for_lane_routes_each_variant() {
        // Distinct capacities per lane → semaphore_for_lane must return
        // the matching one. Catches a copy-paste bug in the match arm
        // (e.g. Lane::Trigger accidentally aliasing main_sem).
        let queue = CommandQueue::with_capacities(2, 4, 6, 5);
        assert_eq!(queue.semaphore_for_lane(Lane::Main).available_permits(), 2);
        assert_eq!(queue.semaphore_for_lane(Lane::Cron).available_permits(), 4);
        assert_eq!(
            queue.semaphore_for_lane(Lane::Subagent).available_permits(),
            6
        );
        assert_eq!(
            queue.semaphore_for_lane(Lane::Trigger).available_permits(),
            5
        );
    }

    #[tokio::test]
    async fn test_try_submit_when_full() {
        let queue = CommandQueue::with_capacities(1, 1, 1, 1);

        // Acquire the main permit via the public accessor.
        let sem = queue.semaphore_for_lane(Lane::Main);
        let _permit = sem.acquire().await.unwrap();

        // try_submit should return None since lane is full
        let result = queue.try_submit(Lane::Main, async { 42 }).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_custom_capacities() {
        let queue = CommandQueue::with_capacities(2, 4, 6, 5);
        let occ = queue.occupancy();
        assert_eq!(occ[0].capacity, 2);
        assert_eq!(occ[1].capacity, 4);
        assert_eq!(occ[2].capacity, 6);
        assert_eq!(occ[3].capacity, 5);
    }

    /// Regression for #3628 — `queue.concurrency.*` was set once at boot
    /// and reload silently kept the old semaphore. After resize_lane the
    /// occupancy snapshot must reflect the new capacity, and new
    /// acquirers must see the new semaphore.
    #[tokio::test]
    async fn test_resize_lane_updates_capacity_and_new_semaphore() {
        let queue = CommandQueue::with_capacities(1, 1, 1, 1);

        // Hold a permit on the OLD trigger semaphore so we can verify
        // it stays valid when we resize underneath it.
        let old_sem = queue.semaphore_for_lane(Lane::Trigger);
        let _old_permit = old_sem.try_acquire().expect("old sem has a free permit");

        // Resize the trigger lane up to 5.
        let changed = queue.resize_lane(Lane::Trigger, 5);
        assert!(changed);

        // Occupancy reports the new capacity from the fresh semaphore;
        // the old one is no longer counted (the in-flight permit drains
        // into a slot that nobody else can reach, by design).
        let trig = queue
            .occupancy()
            .into_iter()
            .find(|o| o.lane == Lane::Trigger)
            .unwrap();
        assert_eq!(trig.capacity, 5);
        assert_eq!(trig.active, 0);

        // A new acquirer sees the new semaphore with full 5 permits.
        let new_sem = queue.semaphore_for_lane(Lane::Trigger);
        assert!(!Arc::ptr_eq(&old_sem, &new_sem));
        assert_eq!(new_sem.available_permits(), 5);

        // No-op resize doesn't churn the semaphore.
        let again = queue.resize_lane(Lane::Trigger, 5);
        assert!(!again);
    }
}
