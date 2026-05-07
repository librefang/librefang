//! Workflow subsystem — orchestration engines + scheduler + queue.
//!
//! Bundles every long-lived orchestration handle that previously sat as
//! a flat field on `LibreFangKernel`: the workflow execution `engine`
//! (renamed from the original `workflows` field to avoid the
//! `self.workflows.workflows` collision), workflow `template_registry`,
//! event-driven `triggers`, the `background` agent executor, the
//! `cron_scheduler`, and the lane-based `command_queue`.

use librefang_runtime::command_lane::CommandQueue;

use crate::background::BackgroundExecutor;
use crate::cron::CronScheduler;
use crate::triggers::TriggerEngine;
use crate::workflow::{WorkflowEngine, WorkflowTemplateRegistry};

/// Workflow / trigger / cron / queue cluster — see module docs.
pub struct WorkflowSubsystem {
    /// Workflow execution engine (renamed from the original `workflows`
    /// field — see module docs).
    pub(crate) engine: WorkflowEngine,
    /// Workflow template registry.
    pub(crate) template_registry: WorkflowTemplateRegistry,
    /// Event-driven trigger engine.
    pub(crate) triggers: TriggerEngine,
    /// Background agent executor.
    pub(crate) background: BackgroundExecutor,
    /// Cron job scheduler.
    pub(crate) cron_scheduler: CronScheduler,
    /// Command queue with lane-based concurrency control.
    pub(crate) command_queue: CommandQueue,
}

impl WorkflowSubsystem {
    pub(crate) fn new(
        engine: WorkflowEngine,
        triggers: TriggerEngine,
        background: BackgroundExecutor,
        cron_scheduler: CronScheduler,
        command_queue: CommandQueue,
    ) -> Self {
        Self {
            engine,
            template_registry: WorkflowTemplateRegistry::new(),
            triggers,
            background,
            cron_scheduler,
            command_queue,
        }
    }
}
