//! [`kernel_handle::AgentTypeTools`] — create agent types (templates) from
//! the agent-facing `agent_type_create` tool (#7722).
//!
//! Shares the exact validation and JSON→TOML shape with
//! `POST /api/templates` through [`librefang_types::agent::agent_type_json_to_toml`]
//! so the two authoring surfaces cannot drift apart.

use super::super::LibreFangKernel;
use librefang_runtime::kernel_handle;

#[async_trait::async_trait]
impl kernel_handle::AgentTypeTools for LibreFangKernel {
    async fn create_agent_type(&self, json: &str) -> Result<String, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;

        let v: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| KernelOpError::Internal(format!("Invalid agent type JSON: {e}")))?;
        let name = v["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .ok_or_else(|| {
                KernelOpError::Internal(
                    "Agent type JSON must include a non-empty 'name'".to_string(),
                )
            })?
            .to_string();
        if name.len() > 64
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(KernelOpError::Internal(
                "Agent type name must be 1-64 chars of [A-Za-z0-9_-]".to_string(),
            ));
        }

        let toml_content = librefang_types::agent::agent_type_json_to_toml(&v);
        let dir = self.home_dir_boot.join("templates");
        let path = dir.join(format!("{name}.toml"));
        if path.exists() {
            return Err(KernelOpError::Internal(format!(
                "Agent type '{name}' already exists"
            )));
        }
        // Cross-source collision (#7722): a workspace agent with the same
        // name would shadow the type in dual-source resolution.
        let workspace_agent = self
            .home_dir_boot
            .join("workspaces")
            .join("agents")
            .join(&name);
        if workspace_agent.exists() {
            return Err(KernelOpError::Internal(format!(
                "A workspace agent named '{name}' already exists"
            )));
        }

        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| KernelOpError::Internal(format!("Failed to create templates dir: {e}")))?;
        tokio::fs::write(&path, toml_content)
            .await
            .map_err(|e| KernelOpError::Internal(format!("Failed to write agent type: {e}")))?;
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn create_agent_type_writes_validates_and_rejects() {
        let dir = tempfile::tempdir().expect("tempdir for agent-type test");
        let home = dir.path().to_path_buf();
        std::fs::create_dir_all(home.join("data")).unwrap();
        let config = librefang_types::config::KernelConfig {
            home_dir: home.clone(),
            data_dir: home.join("data"),
            ..librefang_types::config::KernelConfig::default()
        };
        let kernel = std::sync::Arc::new(
            LibreFangKernel::boot_with_config(config).expect("kernel must boot"),
        );
        std::mem::forget(dir);
        let tools: &dyn kernel_handle::AgentTypeTools = kernel.as_ref();

        // Success: valid JSON writes the template and returns the name.
        let json = serde_json::json!({
            "name": "qa-type",
            "system_prompt": "you are a qa probe",
            "tools": ["chat"],
            "skills": ["review"]
        })
        .to_string();
        let created = tools.create_agent_type(&json).await.expect("must create");
        assert_eq!(created, "qa-type");
        let path = home.join("templates").join("qa-type.toml");
        assert!(path.exists(), "template file must exist on disk");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("name = \"qa-type\""),
            "TOML must carry the name: {content}"
        );

        // Rejection: invalid name charset.
        let bad = serde_json::json!({ "name": "bad name!" }).to_string();
        let err = tools
            .create_agent_type(&bad)
            .await
            .expect_err("must reject");
        assert!(err.to_string().contains("1-64"), "charset rule: {err}");

        // Rejection: duplicate.
        let dup = serde_json::json!({ "name": "qa-type" }).to_string();
        let err = tools
            .create_agent_type(&dup)
            .await
            .expect_err("must reject");
        assert!(err.to_string().contains("already exists"), "dup: {err}");
    }
}
