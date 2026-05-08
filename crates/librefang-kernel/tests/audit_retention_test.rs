// `start_background_agents()` spawns 17 closures whose async-block layouts
// the compiler folds into a single type-resolution query.  After
// `TriggerId` gained `PartialOrd, Ord` (#4067), one of those layouts
// exceeded the default recursion limit of 128.  The compiler explicitly
// suggests this attribute; bumping to 256 leaves headroom for further
// trait-bound additions on kernel-internal types.
#![recursion_limit = "256"]

//! Audit retention M7: kernel boot wires the periodic trim task and the
//! self-audit `RetentionTrim` row lands when a trim cycle actually drops
//! entries.
//!
//! The boot path normally needs `start_background_agents()` to spawn
//! the periodic task, so this test calls it explicitly. We use a very
//! short `trim_interval_secs` (1s) and exercise the
//! `max_in_memory_entries` cap so the trim job has work to do without
//! requiring back-dated timestamps (which would need test-only access
//! to the AuditLog internals).

use librefang_kernel::MeteringSubsystemApi;
use librefang_runtime::audit::AuditAction;
use librefang_testing::MockKernelBuilder;

// `start_background_agents` reaches into kernel paths that call
// `tokio::task::block_in_place` (e.g. the synchronous toml_edit /
// memory-substrate touch points). That requires the multi-threaded
// runtime — the default current-thread runtime panics with
// "can call blocking only when running on the multi-threaded runtime"
// at kernel/mod.rs:3610.
#[tokio::test(flavor = "multi_thread")]
async fn test_kernel_boot_with_retention_config_starts_trim_task() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "groq".to_string();
            c.default_model.model = "llama-3.3-70b-versatile".to_string();
            c.default_model.api_key_env = "GROQ_API_KEY".to_string();
            c.audit.retention.trim_interval_secs = Some(1);
            c.audit.retention.max_in_memory_entries = Some(10);
        })
        .build();

    // Seed 50 audit entries — well over the cap of 10. Use RoleChange
    // so no per-action retention rule could kick in (we want the cap
    // path to be the sole reason rows are dropped).
    let audit = kernel.audit_log().clone();
    for i in 0..50 {
        audit.record(
            "agent-x",
            AuditAction::RoleChange,
            format!("noise-{i}"),
            "ok",
        );
    }
    assert!(audit.len() >= 50);

    // Boot the periodic tasks.
    kernel.start_background_agents().await;

    // Wait long enough for the 1s trim interval to fire at least once.
    // tokio::time::interval skips the first tick after creation only
    // when we explicitly call `interval.tick().await` once before the
    // loop — which the kernel does — so the first effective tick
    // happens ~1s after spawn.
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;

    let entries = audit.recent(100);
    // After trim, len() should be cap (10) + the self-audit RetentionTrim
    // row written after the trim — so 11. Allow some slack in case other
    // boot-time audit writes happen.
    assert!(
        audit.len() <= 20,
        "trim should have collapsed the log down near the cap, got len={}",
        audit.len()
    );
    assert!(
        entries
            .iter()
            .any(|e| matches!(e.action, AuditAction::RetentionTrim)),
        "periodic trim task must record a RetentionTrim self-audit row; got: {:?}",
        entries
            .iter()
            .map(|e| e.action.to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        audit.verify_integrity().is_ok(),
        "chain must still verify after periodic trim"
    );

    kernel.shutdown();
}
