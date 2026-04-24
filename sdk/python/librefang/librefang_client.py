"""
LibreFang Python Client — AUTO-GENERATED from openapi.json.
Do not edit manually. Run: python3 scripts/codegen-sdks.py

Usage:
    from librefang_client import LibreFang

    client = LibreFang("http://localhost:4545")
    agents = client.agents.list_agents()

    for event in client.agents.send_message_stream(agent_id, message="Hello"):
        if event.get("type") == "text_delta":
            print(event["delta"], end="", flush=True)
"""

import json
from typing import Any, Dict, Generator, Optional
from urllib.request import urlopen, Request
from urllib.error import HTTPError
from urllib.parse import urlencode


class LibreFangError(Exception):
    def __init__(self, message: str, status: int = 0, body: str = ""):
        super().__init__(message)
        self.status = status
        self.body = body


class _Resource:
    def __init__(self, client: "LibreFang"):
        self._c = client


class LibreFang:
    """LibreFang REST API client. Zero dependencies — uses only stdlib urllib."""

    def __init__(self, base_url: str, headers: Optional[Dict[str, str]] = None):
        self.base_url = base_url.rstrip("/")
        self._headers = {"Content-Type": "application/json"}
        if headers:
            self._headers.update(headers)
        self.a2a = _A2AResource(self)
        self.agents = _AgentsResource(self)
        self.approvals = _ApprovalsResource(self)
        self.auth = _AuthResource(self)
        self.auto_dream = _AutoDreamResource(self)
        self.budget = _BudgetResource(self)
        self.channels = _ChannelsResource(self)
        self.extensions = _ExtensionsResource(self)
        self.hands = _HandsResource(self)
        self.mcp = _McpResource(self)
        self.memory = _MemoryResource(self)
        self.models = _ModelsResource(self)
        self.network = _NetworkResource(self)
        self.pairing = _PairingResource(self)
        self.proactive_memory = _ProactiveMemoryResource(self)
        self.sessions = _SessionsResource(self)
        self.skills = _SkillsResource(self)
        self.system = _SystemResource(self)
        self.tools = _ToolsResource(self)
        self.webhooks = _WebhooksResource(self)
        self.workflows = _WorkflowsResource(self)


    def _request(self, method: str, path: str, body: Any = None, query: Optional[Dict[str, Any]] = None) -> Any:
        url = self.base_url + path
        if query:
            filtered = {k: v for k, v in query.items() if v is not None}
            if filtered:
                url += ("&" if "?" in url else "?") + urlencode(filtered, doseq=True)
        data = json.dumps(body).encode() if body is not None else None
        req = Request(url, data=data, headers=self._headers, method=method)
        try:
            with urlopen(req) as resp:
                ct = resp.headers.get("content-type", "")
                text = resp.read().decode()
                if "application/json" in ct:
                    return json.loads(text)
                return text
        except HTTPError as e:
            body_text = e.read().decode() if e.fp else ""
            raise LibreFangError(f"HTTP {e.code}: {body_text}", e.code, body_text) from e

    def _stream(self, method: str, path: str, body: Any = None, query: Optional[Dict[str, Any]] = None) -> Generator[Dict, None, None]:
        """SSE streaming — yields parsed JSON events."""
        url = self.base_url + path
        if query:
            filtered = {k: v for k, v in query.items() if v is not None}
            if filtered:
                url += ("&" if "?" in url else "?") + urlencode(filtered, doseq=True)
        data = json.dumps(body).encode() if body is not None else None
        headers = dict(self._headers)
        headers["Accept"] = "text/event-stream"
        req = Request(url, data=data, headers=headers, method=method)
        try:
            resp = urlopen(req)
        except HTTPError as e:
            body_text = e.read().decode() if e.fp else ""
            raise LibreFangError(f"HTTP {e.code}: {body_text}", e.code, body_text) from e

        buffer = ""
        while True:
            chunk = resp.read(4096)
            if not chunk:
                break
            buffer += chunk.decode()
            lines = buffer.split("\n")
            buffer = lines.pop()
            for line in lines:
                line = line.strip()
                if line.startswith("data: "):
                    data_str = line[6:]
                    if data_str == "[DONE]":
                        return
                    try:
                        yield json.loads(data_str)
                    except json.JSONDecodeError:
                        yield {"raw": data_str}
        resp.close()


# ── A2A Resource ───────────────────────────────────────────────

class _A2AResource(_Resource):

    def a2a_list_external_agents(self):
        return self._c._request("GET", "/api/a2a/agents")

    def a2a_get_external_agent(self, id: str):
        return self._c._request("GET", f"/api/a2a/agents/{id}")

    def a2a_discover_external(self, **data):
        return self._c._request("POST", "/api/a2a/discover", data)

    def a2a_send_external(self, **data):
        return self._c._request("POST", "/api/a2a/send", data)

    def a2a_external_task_status(self, id: str, url: Any = None):
        return self._c._request("GET", f"/api/a2a/tasks/{id}/status", None, query={"url": url})


# ── Agents Resource ────────────────────────────────────────────

class _AgentsResource(_Resource):

    def list_agents(self, q: Any = None, status: Any = None, limit: Any = None, offset: Any = None, sort: Any = None, order: Any = None):
        return self._c._request("GET", "/api/agents", None, query={"q": q, "status": status, "limit": limit, "offset": offset, "sort": sort, "order": order})

    def spawn_agent(self, **data):
        return self._c._request("POST", "/api/agents", data)

    def bulk_create_agents(self, **data):
        return self._c._request("POST", "/api/agents/bulk", data)

    def bulk_delete_agents(self):
        return self._c._request("DELETE", "/api/agents/bulk")

    def bulk_start_agents(self, **data):
        return self._c._request("POST", "/api/agents/bulk/start", data)

    def bulk_stop_agents(self, **data):
        return self._c._request("POST", "/api/agents/bulk/stop", data)

    def get_agent(self, id: str):
        return self._c._request("GET", f"/api/agents/{id}")

    def kill_agent(self, id: str):
        return self._c._request("DELETE", f"/api/agents/{id}")

    def patch_agent(self, id: str, **data):
        return self._c._request("PATCH", f"/api/agents/{id}", data)

    def clone_agent(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{id}/clone", data)

    def patch_agent_config(self, id: str, **data):
        return self._c._request("PATCH", f"/api/agents/{id}/config", data)

    def get_agent_deliveries(self, id: str):
        return self._c._request("GET", f"/api/agents/{id}/deliveries")

    def list_agent_files(self, id: str):
        return self._c._request("GET", f"/api/agents/{id}/files")

    def get_agent_file(self, id: str, filename: str):
        return self._c._request("GET", f"/api/agents/{id}/files/{filename}")

    def set_agent_file(self, id: str, filename: str, **data):
        return self._c._request("PUT", f"/api/agents/{id}/files/{filename}", data)

    def delete_agent_file(self, id: str, filename: str):
        return self._c._request("DELETE", f"/api/agents/{id}/files/{filename}")

    def clear_agent_history(self, id: str):
        return self._c._request("DELETE", f"/api/agents/{id}/history")

    def update_agent_identity(self, id: str, **data):
        return self._c._request("PATCH", f"/api/agents/{id}/identity", data)

    def get_agent_mcp_servers(self, id: str):
        return self._c._request("GET", f"/api/agents/{id}/mcp_servers")

    def set_agent_mcp_servers(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{id}/mcp_servers", data)

    def send_message(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{id}/message", data)

    def send_message_stream(self, id: str, **data) -> Generator[Dict, None, None]:
        return self._c._stream("POST", f"/api/agents/{id}/message/stream", data)

    def set_agent_mode(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{id}/mode", data)

    def set_model(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{id}/model", data)

    def get_agent_session(self, id: str):
        return self._c._request("GET", f"/api/agents/{id}/session")

    def compact_session(self, id: str):
        return self._c._request("POST", f"/api/agents/{id}/session/compact")

    def reboot_session(self, id: str):
        return self._c._request("POST", f"/api/agents/{id}/session/reboot")

    def reset_session(self, id: str):
        return self._c._request("POST", f"/api/agents/{id}/session/reset")

    def list_agent_sessions(self, id: str):
        return self._c._request("GET", f"/api/agents/{id}/sessions")

    def create_agent_session(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{id}/sessions", data)

    def import_session(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{id}/sessions/import", data)

    def export_session(self, id: str, session_id: str):
        return self._c._request("GET", f"/api/agents/{id}/sessions/{session_id}/export")

    def switch_agent_session(self, id: str, session_id: str):
        return self._c._request("POST", f"/api/agents/{id}/sessions/{session_id}/switch")

    def get_agent_skills(self, id: str):
        return self._c._request("GET", f"/api/agents/{id}/skills")

    def set_agent_skills(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{id}/skills", data)

    def stop_agent(self, id: str):
        return self._c._request("POST", f"/api/agents/{id}/stop")

    def get_agent_tools(self, id: str):
        return self._c._request("GET", f"/api/agents/{id}/tools")

    def set_agent_tools(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{id}/tools", data)

    def get_agent_traces(self, id: str):
        return self._c._request("GET", f"/api/agents/{id}/traces")

    def update_agent(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{id}/update", data)

    def upload_file(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{id}/upload", data)

    def serve_upload(self, file_id: str):
        return self._c._request("GET", f"/api/uploads/{file_id}")


# ── Approvals Resource ─────────────────────────────────────────

class _ApprovalsResource(_Resource):

    def list_approvals(self):
        return self._c._request("GET", "/api/approvals")

    def create_approval(self, **data):
        return self._c._request("POST", "/api/approvals", data)

    def get_approval(self, id: str):
        return self._c._request("GET", f"/api/approvals/{id}")

    def approve_request(self, id: str, **data):
        return self._c._request("POST", f"/api/approvals/{id}/approve", data)

    def reject_request(self, id: str):
        return self._c._request("POST", f"/api/approvals/{id}/reject")


# ── Auth Resource ──────────────────────────────────────────────

class _AuthResource(_Resource):

    def auth_callback(self):
        return self._c._request("GET", "/api/auth/callback")

    def auth_callback_post(self, **data):
        return self._c._request("POST", "/api/auth/callback", data)

    def auth_introspect(self, **data):
        return self._c._request("POST", "/api/auth/introspect", data)

    def auth_login(self):
        return self._c._request("GET", "/api/auth/login")

    def auth_login_provider(self, provider: str):
        return self._c._request("GET", f"/api/auth/login/{provider}")

    def auth_providers(self):
        return self._c._request("GET", "/api/auth/providers")

    def auth_userinfo(self):
        return self._c._request("GET", "/api/auth/userinfo")


# ── AutoDream Resource ────────────────────────────────────────

class _AutoDreamResource(_Resource):

    def auto_dream_abort(self, id: str):
        return self._c._request("POST", f"/api/auto-dream/agents/{id}/abort")

    def auto_dream_set_enabled(self, id: str, **data):
        return self._c._request("PUT", f"/api/auto-dream/agents/{id}/enabled", data)

    def auto_dream_trigger(self, id: str):
        return self._c._request("POST", f"/api/auto-dream/agents/{id}/trigger")

    def auto_dream_status(self):
        return self._c._request("GET", "/api/auto-dream/status")


# ── Budget Resource ────────────────────────────────────────────

class _BudgetResource(_Resource):

    def budget_status(self):
        return self._c._request("GET", "/api/budget")

    def update_budget(self, **data):
        return self._c._request("PUT", "/api/budget", data)

    def agent_budget_ranking(self):
        return self._c._request("GET", "/api/budget/agents")

    def agent_budget_status(self, id: str):
        return self._c._request("GET", f"/api/budget/agents/{id}")

    def update_agent_budget(self, id: str, **data):
        return self._c._request("PUT", f"/api/budget/agents/{id}", data)

    def usage_stats(self):
        return self._c._request("GET", "/api/usage")

    def usage_by_model(self):
        return self._c._request("GET", "/api/usage/by-model")

    def usage_daily(self):
        return self._c._request("GET", "/api/usage/daily")

    def usage_summary(self):
        return self._c._request("GET", "/api/usage/summary")


# ── Channels Resource ──────────────────────────────────────────

class _ChannelsResource(_Resource):

    def list_channels(self):
        return self._c._request("GET", "/api/channels")

    def reload_channels(self):
        return self._c._request("POST", "/api/channels/reload")

    def wechat_qr_start(self):
        return self._c._request("POST", "/api/channels/wechat/qr/start")

    def wechat_qr_status(self, qr_code: Any = None):
        return self._c._request("GET", "/api/channels/wechat/qr/status", None, query={"qr_code": qr_code})

    def whatsapp_qr_start(self):
        return self._c._request("POST", "/api/channels/whatsapp/qr/start")

    def whatsapp_qr_status(self, session_id: Any = None):
        return self._c._request("GET", "/api/channels/whatsapp/qr/status", None, query={"session_id": session_id})

    def configure_channel(self, name: str, **data):
        return self._c._request("POST", f"/api/channels/{name}/configure", data)

    def remove_channel(self, name: str):
        return self._c._request("DELETE", f"/api/channels/{name}/configure")

    def test_channel(self, name: str, **data):
        return self._c._request("POST", f"/api/channels/{name}/test", data)


# ── Extensions Resource ────────────────────────────────────────

class _ExtensionsResource(_Resource):

    def list_extensions(self):
        return self._c._request("GET", "/api/extensions")

    def install_extension(self, **data):
        return self._c._request("POST", "/api/extensions/install", data)

    def uninstall_extension(self, **data):
        return self._c._request("POST", "/api/extensions/uninstall", data)

    def get_extension(self, name: str):
        return self._c._request("GET", f"/api/extensions/{name}")


# ── Hands Resource ─────────────────────────────────────────────

class _HandsResource(_Resource):

    def list_hands(self):
        return self._c._request("GET", "/api/hands")

    def list_active_hands(self):
        return self._c._request("GET", "/api/hands/active")

    def install_hand(self, **data):
        return self._c._request("POST", "/api/hands/install", data)

    def deactivate_hand(self, id: str):
        return self._c._request("DELETE", f"/api/hands/instances/{id}")

    def hand_instance_browser(self, id: str):
        return self._c._request("GET", f"/api/hands/instances/{id}/browser")

    def pause_hand(self, id: str):
        return self._c._request("POST", f"/api/hands/instances/{id}/pause")

    def resume_hand(self, id: str):
        return self._c._request("POST", f"/api/hands/instances/{id}/resume")

    def hand_stats(self, id: str):
        return self._c._request("GET", f"/api/hands/instances/{id}/stats")

    def reload_hands(self):
        return self._c._request("POST", "/api/hands/reload")

    def get_hand(self, hand_id: str):
        return self._c._request("GET", f"/api/hands/{hand_id}")

    def activate_hand(self, hand_id: str, **data):
        return self._c._request("POST", f"/api/hands/{hand_id}/activate", data)

    def check_hand_deps(self, hand_id: str):
        return self._c._request("POST", f"/api/hands/{hand_id}/check-deps")

    def install_hand_deps(self, hand_id: str):
        return self._c._request("POST", f"/api/hands/{hand_id}/install-deps")

    def get_hand_settings(self, hand_id: str):
        return self._c._request("GET", f"/api/hands/{hand_id}/settings")

    def update_hand_settings(self, hand_id: str, **data):
        return self._c._request("PUT", f"/api/hands/{hand_id}/settings", data)


# ── Mcp Resource ───────────────────────────────────────────────

class _McpResource(_Resource):

    def list_mcp_catalog(self):
        return self._c._request("GET", "/api/mcp/catalog")

    def get_mcp_catalog_entry(self, id: str):
        return self._c._request("GET", f"/api/mcp/catalog/{id}")

    def mcp_health_handler(self):
        return self._c._request("GET", "/api/mcp/health")

    def reload_mcp_handler(self):
        return self._c._request("POST", "/api/mcp/reload")

    def list_mcp_servers(self):
        return self._c._request("GET", "/api/mcp/servers")

    def add_mcp_server(self, **data):
        return self._c._request("POST", "/api/mcp/servers", data)

    def get_mcp_server(self, name: str):
        return self._c._request("GET", f"/api/mcp/servers/{name}")

    def update_mcp_server(self, name: str, **data):
        return self._c._request("PUT", f"/api/mcp/servers/{name}", data)

    def delete_mcp_server(self, name: str):
        return self._c._request("DELETE", f"/api/mcp/servers/{name}")

    def reconnect_mcp_server_handler(self, name: str):
        return self._c._request("POST", f"/api/mcp/servers/{name}/reconnect")


# ── Memory Resource ────────────────────────────────────────────

class _MemoryResource(_Resource):

    def export_agent_memory(self, id: str):
        return self._c._request("GET", f"/api/agents/{id}/memory/export")

    def import_agent_memory(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{id}/memory/import", data)

    def get_agent_kv(self, id: str):
        return self._c._request("GET", f"/api/memory/agents/{id}/kv")

    def get_agent_kv_key(self, id: str, key: str):
        return self._c._request("GET", f"/api/memory/agents/{id}/kv/{key}")

    def set_agent_kv_key(self, id: str, key: str, **data):
        return self._c._request("PUT", f"/api/memory/agents/{id}/kv/{key}", data)

    def delete_agent_kv_key(self, id: str, key: str):
        return self._c._request("DELETE", f"/api/memory/agents/{id}/kv/{key}")


# ── Models Resource ────────────────────────────────────────────

class _ModelsResource(_Resource):

    def catalog_status(self):
        return self._c._request("GET", "/api/catalog/status")

    def catalog_update(self):
        return self._c._request("POST", "/api/catalog/update")

    def list_models(self):
        return self._c._request("GET", "/api/models")

    def list_aliases(self):
        return self._c._request("GET", "/api/models/aliases")

    def create_alias(self, **data):
        return self._c._request("POST", "/api/models/aliases", data)

    def delete_alias(self, alias: str):
        return self._c._request("DELETE", f"/api/models/aliases/{alias}")

    def add_custom_model(self, **data):
        return self._c._request("POST", "/api/models/custom", data)

    def remove_custom_model(self, id: str):
        return self._c._request("DELETE", f"/api/models/custom/{id}")

    def get_model(self, id: str):
        return self._c._request("GET", f"/api/models/{id}")

    def list_providers(self):
        return self._c._request("GET", "/api/providers")

    def copilot_oauth_poll(self, poll_id: str):
        return self._c._request("GET", f"/api/providers/github-copilot/oauth/poll/{poll_id}")

    def copilot_oauth_start(self):
        return self._c._request("POST", "/api/providers/github-copilot/oauth/start")

    def get_provider(self, name: str):
        return self._c._request("GET", f"/api/providers/{name}")

    def set_default_provider(self, name: str, **data):
        return self._c._request("POST", f"/api/providers/{name}/default", data)

    def set_provider_key(self, name: str, **data):
        return self._c._request("POST", f"/api/providers/{name}/key", data)

    def delete_provider_key(self, name: str):
        return self._c._request("DELETE", f"/api/providers/{name}/key")

    def test_provider(self, name: str):
        return self._c._request("POST", f"/api/providers/{name}/test")

    def set_provider_url(self, name: str, **data):
        return self._c._request("PUT", f"/api/providers/{name}/url", data)


# ── Network Resource ───────────────────────────────────────────

class _NetworkResource(_Resource):

    def comms_events(self, limit: Any = None):
        return self._c._request("GET", "/api/comms/events", None, query={"limit": limit})

    def comms_events_stream(self) -> Generator[Dict, None, None]:
        return self._c._stream("GET", "/api/comms/events/stream")

    def comms_send(self, **data):
        return self._c._request("POST", "/api/comms/send", data)

    def comms_task(self, **data):
        return self._c._request("POST", "/api/comms/task", data)

    def comms_topology(self):
        return self._c._request("GET", "/api/comms/topology")

    def network_status(self):
        return self._c._request("GET", "/api/network/status")

    def list_peers(self):
        return self._c._request("GET", "/api/peers")

    def get_peer(self, id: str):
        return self._c._request("GET", f"/api/peers/{id}")


# ── Pairing Resource ───────────────────────────────────────────

class _PairingResource(_Resource):

    def pairing_complete(self, **data):
        return self._c._request("POST", "/api/pairing/complete", data)

    def pairing_devices(self):
        return self._c._request("GET", "/api/pairing/devices")

    def pairing_remove_device(self, id: str):
        return self._c._request("DELETE", f"/api/pairing/devices/{id}")

    def pairing_notify(self, **data):
        return self._c._request("POST", "/api/pairing/notify", data)

    def pairing_request(self):
        return self._c._request("POST", "/api/pairing/request")


# ── ProactiveMemory Resource ──────────────────────────────────

class _ProactiveMemoryResource(_Resource):

    def memory_list(self, category: Any = None, offset: Any = None, limit: Any = None):
        return self._c._request("GET", "/api/memory", None, query={"category": category, "offset": offset, "limit": limit})

    def memory_add(self, **data):
        return self._c._request("POST", "/api/memory", data)

    def memory_list_agent(self, id: str, category: Any = None, offset: Any = None, limit: Any = None):
        return self._c._request("GET", f"/api/memory/agents/{id}", None, query={"category": category, "offset": offset, "limit": limit})

    def memory_reset_agent(self, id: str):
        return self._c._request("DELETE", f"/api/memory/agents/{id}")

    def memory_consolidate(self, id: str):
        return self._c._request("POST", f"/api/memory/agents/{id}/consolidate")

    def memory_duplicates(self, id: str):
        return self._c._request("GET", f"/api/memory/agents/{id}/duplicates")

    def memory_export_agent(self, id: str):
        return self._c._request("GET", f"/api/memory/agents/{id}/export")

    def memory_import_agent(self, id: str, **data):
        return self._c._request("POST", f"/api/memory/agents/{id}/import", data)

    def memory_clear_level(self, id: str, level: str):
        return self._c._request("DELETE", f"/api/memory/agents/{id}/level/{level}")

    def memory_search_agent(self, id: str, q: Any = None, limit: Any = None):
        return self._c._request("GET", f"/api/memory/agents/{id}/search", None, query={"q": q, "limit": limit})

    def memory_stats_agent(self, id: str):
        return self._c._request("GET", f"/api/memory/agents/{id}/stats")

    def memory_cleanup(self):
        return self._c._request("POST", "/api/memory/cleanup")

    def memory_update(self, memory_id: str, **data):
        return self._c._request("PUT", f"/api/memory/items/{memory_id}", data)

    def memory_delete(self, memory_id: str):
        return self._c._request("DELETE", f"/api/memory/items/{memory_id}")

    def memory_history(self, memory_id: str):
        return self._c._request("GET", f"/api/memory/items/{memory_id}/history")

    def memory_search(self, q: Any = None, limit: Any = None):
        return self._c._request("GET", "/api/memory/search", None, query={"q": q, "limit": limit})

    def memory_stats(self):
        return self._c._request("GET", "/api/memory/stats")

    def memory_get_user(self, user_id: str):
        return self._c._request("GET", f"/api/memory/user/{user_id}")


# ── Sessions Resource ──────────────────────────────────────────

class _SessionsResource(_Resource):

    def find_session_by_label(self, id: str, label: str):
        return self._c._request("GET", f"/api/agents/{id}/sessions/by-label/{label}")

    def list_sessions(self):
        return self._c._request("GET", "/api/sessions")

    def session_cleanup(self):
        return self._c._request("POST", "/api/sessions/cleanup")

    def get_session(self, id: str):
        return self._c._request("GET", f"/api/sessions/{id}")

    def delete_session(self, id: str):
        return self._c._request("DELETE", f"/api/sessions/{id}")

    def set_session_label(self, id: str, **data):
        return self._c._request("PUT", f"/api/sessions/{id}/label", data)


# ── Skills Resource ────────────────────────────────────────────

class _SkillsResource(_Resource):

    def clawhub_browse(self, q: Any = None):
        return self._c._request("GET", "/api/clawhub/browse", None, query={"q": q})

    def clawhub_install(self, **data):
        return self._c._request("POST", "/api/clawhub/install", data)

    def clawhub_search(self, q: Any = None):
        return self._c._request("GET", "/api/clawhub/search", None, query={"q": q})

    def clawhub_skill_detail(self, slug: str):
        return self._c._request("GET", f"/api/clawhub/skill/{slug}")

    def clawhub_skill_code(self, slug: str):
        return self._c._request("GET", f"/api/clawhub/skill/{slug}/code")

    def marketplace_search(self, q: Any = None):
        return self._c._request("GET", "/api/marketplace/search", None, query={"q": q})

    def list_skills(self):
        return self._c._request("GET", "/api/skills")

    def create_skill(self, **data):
        return self._c._request("POST", "/api/skills/create", data)

    def install_skill(self, **data):
        return self._c._request("POST", "/api/skills/install", data)

    def uninstall_skill(self, **data):
        return self._c._request("POST", "/api/skills/uninstall", data)

    def list_tools(self):
        return self._c._request("GET", "/api/tools")

    def get_tool(self, name: str):
        return self._c._request("GET", f"/api/tools/{name}")


# ── System Resource ────────────────────────────────────────────

class _SystemResource(_Resource):

    def audit_recent(self):
        return self._c._request("GET", "/api/audit/recent")

    def audit_verify(self):
        return self._c._request("GET", "/api/audit/verify")

    def create_backup(self):
        return self._c._request("POST", "/api/backup")

    def list_backups(self):
        return self._c._request("GET", "/api/backups")

    def delete_backup(self, filename: str):
        return self._c._request("DELETE", f"/api/backups/{filename}")

    def list_bindings(self):
        return self._c._request("GET", "/api/bindings")

    def add_binding(self, **data):
        return self._c._request("POST", "/api/bindings", data)

    def remove_binding(self, index: str):
        return self._c._request("DELETE", f"/api/bindings/{index}")

    def list_commands(self):
        return self._c._request("GET", "/api/commands")

    def get_command(self, name: str):
        return self._c._request("GET", f"/api/commands/{name}")

    def get_config(self):
        return self._c._request("GET", "/api/config")

    def config_reload(self):
        return self._c._request("POST", "/api/config/reload")

    def config_schema(self):
        return self._c._request("GET", "/api/config/schema")

    def config_set(self, **data):
        return self._c._request("POST", "/api/config/set", data)

    def health(self):
        return self._c._request("GET", "/api/health")

    def health_detail(self):
        return self._c._request("GET", "/api/health/detail")

    def quick_init(self):
        return self._c._request("POST", "/api/init")

    def logs_stream(self) -> Generator[Dict, None, None]:
        return self._c._stream("GET", "/api/logs/stream")

    def prometheus_metrics(self):
        return self._c._request("GET", "/api/metrics")

    def run_migrate(self, **data):
        return self._c._request("POST", "/api/migrate", data)

    def migrate_detect(self):
        return self._c._request("GET", "/api/migrate/detect")

    def migrate_scan(self, **data):
        return self._c._request("POST", "/api/migrate/scan", data)

    def list_profiles(self):
        return self._c._request("GET", "/api/profiles")

    def get_profile(self, name: str):
        return self._c._request("GET", f"/api/profiles/{name}")

    def queue_status(self):
        return self._c._request("GET", "/api/queue/status")

    def restore_backup(self, **data):
        return self._c._request("POST", "/api/restore", data)

    def security_status(self):
        return self._c._request("GET", "/api/security")

    def shutdown(self):
        return self._c._request("POST", "/api/shutdown")

    def status(self):
        return self._c._request("GET", "/api/status")

    def list_agent_templates(self):
        return self._c._request("GET", "/api/templates")

    def get_agent_template(self, name: str):
        return self._c._request("GET", f"/api/templates/{name}")

    def version(self):
        return self._c._request("GET", "/api/version")

    def api_versions(self):
        return self._c._request("GET", "/api/versions")


# ── Tools Resource ─────────────────────────────────────────────

class _ToolsResource(_Resource):

    def invoke_tool(self, name: str, agent_id: Any = None, **data):
        return self._c._request("POST", f"/api/tools/{name}/invoke", data, query={"agent_id": agent_id})


# ── Webhooks Resource ──────────────────────────────────────────

class _WebhooksResource(_Resource):

    def webhook_agent(self, **data):
        return self._c._request("POST", "/api/hooks/agent", data)

    def webhook_wake(self, **data):
        return self._c._request("POST", "/api/hooks/wake", data)


# ── Workflows Resource ─────────────────────────────────────────

class _WorkflowsResource(_Resource):

    def list_cron_jobs(self):
        return self._c._request("GET", "/api/cron/jobs")

    def create_cron_job(self, **data):
        return self._c._request("POST", "/api/cron/jobs", data)

    def update_cron_job(self, id: str, **data):
        return self._c._request("PUT", f"/api/cron/jobs/{id}", data)

    def delete_cron_job(self, id: str):
        return self._c._request("DELETE", f"/api/cron/jobs/{id}")

    def toggle_cron_job(self, id: str, **data):
        return self._c._request("PUT", f"/api/cron/jobs/{id}/enable", data)

    def cron_job_status(self, id: str):
        return self._c._request("GET", f"/api/cron/jobs/{id}/status")

    def list_schedules(self):
        return self._c._request("GET", "/api/schedules")

    def create_schedule(self, **data):
        return self._c._request("POST", "/api/schedules", data)

    def get_schedule(self, id: str):
        return self._c._request("GET", f"/api/schedules/{id}")

    def update_schedule(self, id: str, **data):
        return self._c._request("PUT", f"/api/schedules/{id}", data)

    def delete_schedule(self, id: str):
        return self._c._request("DELETE", f"/api/schedules/{id}")

    def run_schedule(self, id: str):
        return self._c._request("POST", f"/api/schedules/{id}/run")

    def list_triggers(self, agent_id: Any = None):
        return self._c._request("GET", "/api/triggers", None, query={"agent_id": agent_id})

    def create_trigger(self, **data):
        return self._c._request("POST", "/api/triggers", data)

    def get_trigger(self, id: str):
        return self._c._request("GET", f"/api/triggers/{id}")

    def delete_trigger(self, id: str):
        return self._c._request("DELETE", f"/api/triggers/{id}")

    def update_trigger(self, id: str, **data):
        return self._c._request("PATCH", f"/api/triggers/{id}", data)

    def list_workflows(self):
        return self._c._request("GET", "/api/workflows")

    def create_workflow(self, **data):
        return self._c._request("POST", "/api/workflows", data)

    def update_workflow(self, id: str, **data):
        return self._c._request("PUT", f"/api/workflows/{id}", data)

    def delete_workflow(self, id: str):
        return self._c._request("DELETE", f"/api/workflows/{id}")

    def run_workflow(self, id: str, **data):
        return self._c._request("POST", f"/api/workflows/{id}/run", data)

    def list_workflow_runs(self, id: str):
        return self._c._request("GET", f"/api/workflows/{id}/runs")

    def save_workflow_as_template(self, id: str):
        return self._c._request("POST", f"/api/workflows/{id}/save-as-template")

