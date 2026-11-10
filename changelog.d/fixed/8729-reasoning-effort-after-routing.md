Channel, cron, trigger and `agent_send` turns now send the reasoning effort the routed model accepts, instead of one already clamped to the agent's pre-routing model.
The blocking dispatch path coerced the effort against the manifest's model before Stable pinning, profile routing or tier routing ran, so a routed model with a wider declared set received a downgraded token (for example `xhigh` became `medium`) that nothing downstream could restore.
Coercion now runs after model selection and the session `/model` override, matching the streaming path (#8729) (@houko)
