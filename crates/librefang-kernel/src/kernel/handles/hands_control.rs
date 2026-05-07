//! [`kernel_handle::HandsControl`] — list / install / activate / deactivate
//! "hand" packages (curated trusted skill bundles). Each install or activate
//! invalidates the hand-route cache so the next routing decision sees the
//! updated registry.

use librefang_runtime::kernel_handle;
use librefang_types::agent::AgentId;

use super::super::{router, LibreFangKernel};

#[async_trait::async_trait]
impl kernel_handle::HandsControl for LibreFangKernel {
    async fn hand_list(&self) -> Result<Vec<serde_json::Value>, kernel_handle::KernelOpError> {
        let defs = self.hand_registry.list_definitions();
        let instances = self.hand_registry.list_instances();

        let mut result = Vec::new();
        for def in defs {
            // Check if this hand has an active instance
            let active_instance = instances.iter().find(|i| i.hand_id == def.id);
            let (status, instance_id, agent_id) = match active_instance {
                Some(inst) => (
                    format!("{}", inst.status),
                    Some(inst.instance_id.to_string()),
                    inst.agent_id().map(|a: AgentId| a.to_string()),
                ),
                None => ("available".to_string(), None, None),
            };

            let mut entry = serde_json::json!({
                "id": def.id,
                "name": def.name,
                "icon": def.icon,
                "category": format!("{:?}", def.category),
                "description": def.description,
                "status": status,
                "tools": def.tools,
            });
            if let Some(iid) = instance_id {
                entry["instance_id"] = serde_json::json!(iid);
            }
            if let Some(aid) = agent_id {
                entry["agent_id"] = serde_json::json!(aid);
            }
            result.push(entry);
        }
        Ok(result)
    }

    async fn hand_install(
        &self,
        toml_content: &str,
        skill_content: &str,
    ) -> Result<serde_json::Value, kernel_handle::KernelOpError> {
        let def = self
            .hand_registry
            .install_from_content_persisted(&self.home_dir_boot, toml_content, skill_content)
            .map_err(|e| format!("{e}"))?;
        router::invalidate_hand_route_cache();

        Ok(serde_json::json!({
            "id": def.id,
            "name": def.name,
            "description": def.description,
            "category": format!("{:?}", def.category),
        }))
    }

    async fn hand_activate(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, kernel_handle::KernelOpError> {
        let instance = self
            .activate_hand(hand_id, config)
            .map_err(|e| format!("{e}"))?;

        Ok(serde_json::json!({
            "instance_id": instance.instance_id.to_string(),
            "hand_id": instance.hand_id,
            "agent_name": instance.agent_name(),
            "agent_id": instance.agent_id().map(|a| a.to_string()),
            "status": format!("{}", instance.status),
        }))
    }

    async fn hand_status(
        &self,
        hand_id: &str,
    ) -> Result<serde_json::Value, kernel_handle::KernelOpError> {
        let instances = self.hand_registry.list_instances();
        let instance = instances
            .iter()
            .find(|i| i.hand_id == hand_id)
            .ok_or_else(|| format!("No active instance found for hand '{hand_id}'"))?;

        let def = self.hand_registry.get_definition(hand_id);
        let def_name = def.as_ref().map(|d| d.name.clone()).unwrap_or_default();
        let def_icon = def.as_ref().map(|d| d.icon.clone()).unwrap_or_default();

        Ok(serde_json::json!({
            "hand_id": hand_id,
            "name": def_name,
            "icon": def_icon,
            "instance_id": instance.instance_id.to_string(),
            "status": format!("{}", instance.status),
            "agent_id": instance.agent_id().map(|a| a.to_string()),
            "agent_name": instance.agent_name(),
            "activated_at": instance.activated_at.to_rfc3339(),
            "updated_at": instance.updated_at.to_rfc3339(),
        }))
    }

    async fn hand_deactivate(&self, instance_id: &str) -> Result<(), kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let uuid = uuid::Uuid::parse_str(instance_id)
            .map_err(|e| KernelOpError::InvalidInput(format!("instance_id: {e}")))?;
        self.deactivate_hand(uuid)
            .map_err(|e| KernelOpError::Internal(e.to_string()))
    }
}
