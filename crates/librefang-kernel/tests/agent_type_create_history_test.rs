//! The kernel-side agent-type authoring path must record a version snapshot (#8574 follow-up).
//!
//! The `agent_type_create` tool (#7722) writes the same `agent-types/` file the HTTP handlers do
//! through the shared store, so a type an agent authors has to land in the same
//! `template_versions` history — otherwise an operator can neither see nor roll back what an
//! agent wrote, and the history tab silently reports the type as never having existed.
//!
//! This drives the real kernel handle against a real substrate, so the assertion is over the
//! store's own SQL rather than over a mock's willingness to agree with it.

use librefang_kernel::KernelApi;
use librefang_kernel_handle::AgentControl;
use librefang_types::agent_type::AgentTypeSpec;

mod common;

use common::boot_kernel as boot;

#[tokio::test(flavor = "multi_thread")]
async fn create_agent_type_records_an_agent_tool_snapshot() {
    let (kernel, _tmp) = boot();

    let spec = AgentTypeSpec {
        name: Some("kernel-authored".to_string()),
        description: Some("authored through the agent-facing tool".to_string()),
        ..AgentTypeSpec::default()
    };
    let created = AgentControl::create_agent_type(&kernel, "kernel-authored", spec)
        .await
        .expect("create_agent_type");
    assert_eq!(created.name, "kernel-authored");

    let store = librefang_memory::TemplateVersionStore::new(kernel.memory_substrate().pool());
    let versions = store
        .list_for_template("kernel-authored", 10)
        .expect("list template versions");
    assert_eq!(
        versions.len(),
        1,
        "one tool-authored create must leave exactly one snapshot, not none and not a duplicate"
    );
    assert_eq!(
        versions[0].change_source, "agent-tool",
        "the kernel path must record its own source so the dashboard history can label it"
    );
    assert!(
        versions[0].manifest_toml.contains("kernel-authored"),
        "the snapshot must carry the document the write landed: {}",
        versions[0].manifest_toml
    );
}
