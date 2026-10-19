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
import socket
import sys
from typing import Any, Dict, Generator, Optional
from urllib.request import urlopen, Request
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode

DEFAULT_TIMEOUT = 30.0


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

    def __init__(self, base_url: str, headers: Optional[Dict[str, str]] = None, timeout: float = DEFAULT_TIMEOUT):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout
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
        self.goals = _GoalsResource(self)
        self.groups = _GroupsResource(self)
        self.hands = _HandsResource(self)
        self.inbox = _InboxResource(self)
        self.knowledge = _KnowledgeResource(self)
        self.mcp = _McpResource(self)
        self.media = _MediaResource(self)
        self.memory = _MemoryResource(self)
        self.models = _ModelsResource(self)
        self.network = _NetworkResource(self)
        self.pairing = _PairingResource(self)
        self.plugins = _PluginsResource(self)
        self.proactive_memory = _ProactiveMemoryResource(self)
        self.sessions = _SessionsResource(self)
        self.skills = _SkillsResource(self)
        self.system = _SystemResource(self)
        self.tasks = _TasksResource(self)
        self.tools = _ToolsResource(self)
        self.users = _UsersResource(self)
        self.vault = _VaultResource(self)
        self.webhooks = _WebhooksResource(self)
        self.workflows = _WorkflowsResource(self)


    def _request(self, method: str, path: str, body: Any = None, query: Optional[Dict[str, Any]] = None, content_type: Optional[str] = None) -> Any:
        """Send a request. `content_type` sends `body` as raw bytes instead of JSON."""
        url = self.base_url + path
        if query:
            # serde on the server accepts only `true` / `false`; `urlencode` would send Python's `True` / `False`.
            filtered = {k: (("true" if v else "false") if isinstance(v, bool) else v) for k, v in query.items() if v is not None}
            if filtered:
                url += ("&" if "?" in url else "?") + urlencode(filtered, doseq=True)
        headers = self._headers
        if content_type is not None:
            data = bytes(body) if body is not None else None
            headers = dict(headers)
            headers["Content-Type"] = content_type
        else:
            data = json.dumps(body).encode() if body is not None else None
        req = Request(url, data=data, headers=headers, method=method)
        try:
            with urlopen(req, timeout=self.timeout) as resp:
                ct = resp.headers.get("content-type", "")
                text = resp.read().decode()
                if "application/json" in ct:
                    return json.loads(text)
                return text
        except HTTPError as e:
            body_text = e.read().decode() if e.fp else ""
            raise LibreFangError(f"HTTP {e.code}: {body_text}", e.code, body_text) from e
        except socket.timeout as e:
            raise LibreFangError(f"Request timed out after {self.timeout}s") from e
        except URLError as e:
            raise LibreFangError(f"Connection error: {e.reason}") from e

    def _stream(self, method: str, path: str, body: Any = None, query: Optional[Dict[str, Any]] = None) -> Generator[Dict, None, None]:
        """SSE streaming — yields parsed JSON events."""
        url = self.base_url + path
        if query:
            # serde on the server accepts only `true` / `false`; `urlencode` would send Python's `True` / `False`.
            filtered = {k: (("true" if v else "false") if isinstance(v, bool) else v) for k, v in query.items() if v is not None}
            if filtered:
                url += ("&" if "?" in url else "?") + urlencode(filtered, doseq=True)
        data = json.dumps(body).encode() if body is not None else None
        headers = dict(self._headers)
        headers["Accept"] = "text/event-stream"
        req = Request(url, data=data, headers=headers, method=method)
        try:
            resp = urlopen(req, timeout=self.timeout)
        except HTTPError as e:
            body_text = e.read().decode() if e.fp else ""
            raise LibreFangError(f"HTTP {e.code}: {body_text}", e.code, body_text) from e
        except socket.timeout as e:
            raise LibreFangError(f"Request timed out after {self.timeout}s") from e
        except URLError as e:
            raise LibreFangError(f"Connection error: {e.reason}") from e

        try:
            buffer = b""
            data_lines = []
            while True:
                chunk = resp.read(4096)
                if not chunk:
                    break
                buffer += chunk
                lines = buffer.split(b"\n")
                buffer = lines.pop()
                for raw_line in lines:
                    line = raw_line.decode().removesuffix("\r")
                    if not line:
                        if not data_lines:
                            continue
                        data_str = "\n".join(data_lines)
                        data_lines.clear()
                        if data_str == "[DONE]":
                            return
                        try:
                            yield json.loads(data_str)
                        except json.JSONDecodeError:
                            yield {"raw": data_str}
                    elif line.startswith("data:"):
                        value = line[5:]
                        if value.startswith(" "):
                            value = value[1:]
                        data_lines.append(value)
            # A clean EOF can arrive without a trailing newline, leaving the last event in the buffer.
            # Parse it here rather than dropping it; the loop above only fires on a newline.
            if buffer:
                line = buffer.decode().removesuffix("\r")
                if line.startswith("data:"):
                    value = line[5:]
                    if value.startswith(" "):
                        value = value[1:]
                    data_lines.append(value)
            if data_lines:
                data_str = "\n".join(data_lines)
                if data_str != "[DONE]":
                    try:
                        yield json.loads(data_str)
                    except json.JSONDecodeError:
                        yield {"raw": data_str}
        except socket.timeout as e:
            raise LibreFangError(f"Request timed out after {self.timeout}s") from e
        finally:
            active_error = sys.exc_info()[0] is not None
            try:
                resp.close()
            except Exception:
                if not active_error:
                    raise


# ── A2A Resource ───────────────────────────────────────────────

class _A2AResource(_Resource):

    def a2a_list_external_agents(self):
        return self._c._request("GET", "/api/a2a/agents")

    def a2a_get_external_agent(self, id: str):
        return self._c._request("GET", f"/api/a2a/agents/{quote(str(id), safe='')}")

    def a2a_approve_external(self, id: str):
        return self._c._request("POST", f"/api/a2a/agents/{quote(str(id), safe='')}/approve")

    def a2a_discover_external(self, **data):
        return self._c._request("POST", "/api/a2a/discover", data)

    def a2a_send_external(self, **data):
        return self._c._request("POST", "/api/a2a/send", data)

    def a2a_external_task_status(self, id: str, url: Any = None):
        return self._c._request("GET", f"/api/a2a/tasks/{quote(str(id), safe='')}/status", None, query={"url": url})


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

    def list_agent_identities(self):
        return self._c._request("GET", "/api/agents/identities")

    def reset_agent_identity(self, name: str, confirm: Any = None):
        return self._c._request("POST", f"/api/agents/identities/{quote(str(name), safe='')}/reset", None, query={"confirm": confirm})

    def spawn_ephemeral_agent(self, **data):
        return self._c._request("POST", "/api/agents/spawn-ephemeral", data)

    def get_agent(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}")

    def kill_agent(self, id: str, confirm: Any = None):
        return self._c._request("DELETE", f"/api/agents/{quote(str(id), safe='')}", None, query={"confirm": confirm})

    def patch_agent(self, id: str, **data):
        return self._c._request("PATCH", f"/api/agents/{quote(str(id), safe='')}", data)

    def serve_agent_avatar(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/avatar")

    def upload_agent_avatar(self, id: str, body: bytes, content_type: str = "application/octet-stream"):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/avatar", body, content_type=content_type)

    def delete_agent_avatar(self, id: str):
        return self._c._request("DELETE", f"/api/agents/{quote(str(id), safe='')}/avatar")

    def get_agent_channels(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/channels")

    def set_agent_channels(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{quote(str(id), safe='')}/channels", data)

    def clone_agent(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/clone", data)

    def patch_agent_config(self, id: str, **data):
        return self._c._request("PATCH", f"/api/agents/{quote(str(id), safe='')}/config", data)

    def get_agent_deliveries(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/deliveries")

    def list_agent_ephemeral_runs(self, id: str, limit: Any = None):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/ephemeral-runs", None, query={"limit": limit})

    def list_agent_events(self, id: str, limit: Any = None):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/events", None, query={"limit": limit})

    def list_agent_files(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/files")

    def get_agent_file(self, id: str, filename: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/files/{quote(str(filename), safe='')}")

    def set_agent_file(self, id: str, filename: str, **data):
        return self._c._request("PUT", f"/api/agents/{quote(str(id), safe='')}/files/{quote(str(filename), safe='')}", data)

    def delete_agent_file(self, id: str, filename: str):
        return self._c._request("DELETE", f"/api/agents/{quote(str(id), safe='')}/files/{quote(str(filename), safe='')}")

    def delete_hand_agent_runtime_config(self, id: str):
        return self._c._request("DELETE", f"/api/agents/{quote(str(id), safe='')}/hand-runtime-config")

    def patch_hand_agent_runtime_config(self, id: str, **data):
        return self._c._request("PATCH", f"/api/agents/{quote(str(id), safe='')}/hand-runtime-config", data)

    def clear_agent_history(self, id: str):
        return self._c._request("DELETE", f"/api/agents/{quote(str(id), safe='')}/history")

    def update_agent_identity(self, id: str, **data):
        return self._c._request("PATCH", f"/api/agents/{quote(str(id), safe='')}/identity", data)

    def inject_message(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/inject", data)

    def agent_logs(self, id: str, n: Any = None, level: Any = None, offset: Any = None):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/logs", None, query={"n": n, "level": level, "offset": offset})

    def get_agent_manifest_toml(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/manifest")

    def list_agent_manifest_history(self, id: str, limit: Any = None):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/manifest-history", None, query={"limit": limit})

    def restore_agent_manifest_version(self, id: str, version_id: str):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/manifest-history/{quote(str(version_id), safe='')}/restore")

    def get_agent_mcp_servers(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/mcp_servers")

    def set_agent_mcp_servers(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{quote(str(id), safe='')}/mcp_servers", data)

    def send_message(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/message", data)

    def send_message_stream(self, id: str, **data) -> Generator[Dict, None, None]:
        return self._c._stream("POST", f"/api/agents/{quote(str(id), safe='')}/message/stream", data)

    def agent_metrics(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/metrics")

    def set_agent_mode(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{quote(str(id), safe='')}/mode", data)

    def set_model(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{quote(str(id), safe='')}/model", data)

    def get_agent_model_routing(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/model_routing")

    def set_agent_model_routing(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{quote(str(id), safe='')}/model_routing", data)

    def push_message(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/push", data)

    def reload_agent_manifest(self, id: str):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/reload")

    def resume_agent(self, id: str):
        return self._c._request("PUT", f"/api/agents/{quote(str(id), safe='')}/resume")

    def list_agent_runtime(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/runtime")

    def save_agent_as_agent_type(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/save-as-agent-type", data)

    def get_agent_session(self, id: str, session_id: Any = None):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/session", None, query={"session_id": session_id})

    def compact_session(self, id: str):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/session/compact")

    def get_agent_session_context(self, id: str, session_id: Any = None):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/session/context", None, query={"session_id": session_id})

    def reboot_session(self, id: str):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/session/reboot")

    def reset_session(self, id: str):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/session/reset")

    def list_agent_sessions(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/sessions")

    def create_agent_session(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/sessions", data)

    def import_session(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/sessions/import", data)

    def export_session(self, id: str, session_id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/sessions/{quote(str(session_id), safe='')}/export")

    def stop_session(self, id: str, session_id: str):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/sessions/{quote(str(session_id), safe='')}/stop")

    def attach_session_stream(self, id: str, session_id: str) -> Generator[Dict, None, None]:
        return self._c._stream("GET", f"/api/agents/{quote(str(id), safe='')}/sessions/{quote(str(session_id), safe='')}/stream")

    def switch_agent_session(self, id: str, session_id: str):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/sessions/{quote(str(session_id), safe='')}/switch")

    def export_session_trajectory(self, id: str, session_id: str, format: Any = None):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/sessions/{quote(str(session_id), safe='')}/trajectory", None, query={"format": format})

    def get_agent_skills(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/skills")

    def set_agent_skills(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{quote(str(id), safe='')}/skills", data)

    def get_agent_stats(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/stats")

    def stop_agent(self, id: str):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/stop")

    def suspend_agent(self, id: str):
        return self._c._request("PUT", f"/api/agents/{quote(str(id), safe='')}/suspend")

    def get_agent_tools(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/tools")

    def set_agent_tools(self, id: str, **data):
        return self._c._request("PUT", f"/api/agents/{quote(str(id), safe='')}/tools", data)

    def get_agent_traces(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/traces")

    def upload_file(self, id: str, body: bytes, content_type: str = "application/octet-stream"):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/upload", body, content_type=content_type)

    def serve_upload(self, file_id: str):
        return self._c._request("GET", f"/api/uploads/{quote(str(file_id), safe='')}")


# ── Approvals Resource ─────────────────────────────────────────

class _ApprovalsResource(_Resource):

    def list_approvals(self, limit: Any = None, offset: Any = None):
        return self._c._request("GET", "/api/approvals", None, query={"limit": limit, "offset": offset})

    def create_approval(self, **data):
        return self._c._request("POST", "/api/approvals", data)

    def audit_log(self, limit: Any = None, offset: Any = None, agent_id: Any = None, tool_name: Any = None):
        return self._c._request("GET", "/api/approvals/audit", None, query={"limit": limit, "offset": offset, "agent_id": agent_id, "tool_name": tool_name})

    def batch_resolve(self, **data):
        return self._c._request("POST", "/api/approvals/batch", data)

    def approval_count(self):
        return self._c._request("GET", "/api/approvals/count")

    def list_approvals_for_session(self, session_id: str):
        return self._c._request("GET", f"/api/approvals/session/{quote(str(session_id), safe='')}")

    def approve_all_for_session(self, session_id: str, **data):
        return self._c._request("POST", f"/api/approvals/session/{quote(str(session_id), safe='')}/approve_all", data)

    def reject_all_for_session(self, session_id: str):
        return self._c._request("POST", f"/api/approvals/session/{quote(str(session_id), safe='')}/reject_all")

    def get_approval(self, id: str):
        return self._c._request("GET", f"/api/approvals/{quote(str(id), safe='')}")

    def approve_request(self, id: str, **data):
        return self._c._request("POST", f"/api/approvals/{quote(str(id), safe='')}/approve", data)

    def modify_request(self, id: str, **data):
        return self._c._request("POST", f"/api/approvals/{quote(str(id), safe='')}/modify", data)

    def reject_request(self, id: str):
        return self._c._request("POST", f"/api/approvals/{quote(str(id), safe='')}/reject")


# ── Auth Resource ──────────────────────────────────────────────

class _AuthResource(_Resource):

    def auth_callback(self):
        return self._c._request("GET", "/api/auth/callback")

    def auth_callback_post(self, **data):
        return self._c._request("POST", "/api/auth/callback", data)

    def change_password(self, **data):
        return self._c._request("POST", "/api/auth/change-password", data)

    def dashboard_auth_check(self):
        return self._c._request("GET", "/api/auth/dashboard-check")

    def dashboard_login(self, **data):
        return self._c._request("POST", "/api/auth/dashboard-login", data)

    def auth_introspect(self, **data):
        return self._c._request("POST", "/api/auth/introspect", data)

    def auth_login(self):
        return self._c._request("GET", "/api/auth/login")

    def auth_login_provider(self, provider: str):
        return self._c._request("GET", f"/api/auth/login/{quote(str(provider), safe='')}")

    def dashboard_logout(self):
        return self._c._request("POST", "/api/auth/logout")

    def authentication_options(self, **data):
        return self._c._request("POST", "/api/auth/passkey/authentication-options", data)

    def authentication_verify(self, **data):
        return self._c._request("POST", "/api/auth/passkey/authentication-verify", data)

    def list_credentials(self):
        return self._c._request("GET", "/api/auth/passkey/credentials")

    def revoke_credential(self, id: str):
        return self._c._request("DELETE", f"/api/auth/passkey/credentials/{quote(str(id), safe='')}")

    def registration_options(self, **data):
        return self._c._request("POST", "/api/auth/passkey/registration-options", data)

    def registration_verify(self, **data):
        return self._c._request("POST", "/api/auth/passkey/registration-verify", data)

    def auth_providers(self):
        return self._c._request("GET", "/api/auth/providers")

    def auth_refresh(self, **data):
        return self._c._request("POST", "/api/auth/refresh", data)

    def auth_userinfo(self):
        return self._c._request("GET", "/api/auth/userinfo")


# ── AutoDream Resource ────────────────────────────────────────

class _AutoDreamResource(_Resource):

    def auto_dream_abort(self, id: str):
        return self._c._request("POST", f"/api/auto-dream/agents/{quote(str(id), safe='')}/abort")

    def auto_dream_set_enabled(self, id: str, **data):
        return self._c._request("PUT", f"/api/auto-dream/agents/{quote(str(id), safe='')}/enabled", data)

    def auto_dream_trigger(self, id: str):
        return self._c._request("POST", f"/api/auto-dream/agents/{quote(str(id), safe='')}/trigger")

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
        return self._c._request("GET", f"/api/budget/agents/{quote(str(id), safe='')}")

    def update_agent_budget(self, id: str, **data):
        return self._c._request("PUT", f"/api/budget/agents/{quote(str(id), safe='')}", data)

    def provider_budget_list(self):
        return self._c._request("GET", "/api/budget/providers")

    def update_provider_budget(self, provider_id: str, **data):
        return self._c._request("PUT", f"/api/budget/providers/{quote(str(provider_id), safe='')}", data)

    def user_budget_ranking(self, limit: Any = None):
        return self._c._request("GET", "/api/budget/users", None, query={"limit": limit})

    def user_budget_detail(self, user_id: str):
        return self._c._request("GET", f"/api/budget/users/{quote(str(user_id), safe='')}")

    def update_user_budget(self, user_id: str, **data):
        return self._c._request("PUT", f"/api/budget/users/{quote(str(user_id), safe='')}", data)

    def delete_user_budget(self, user_id: str):
        return self._c._request("DELETE", f"/api/budget/users/{quote(str(user_id), safe='')}")

    def usage_stats(self, start_date: Any = None, end_date: Any = None):
        return self._c._request("GET", "/api/usage", None, query={"start_date": start_date, "end_date": end_date})

    def usage_by_model(self, start_date: Any = None, end_date: Any = None):
        return self._c._request("GET", "/api/usage/by-model", None, query={"start_date": start_date, "end_date": end_date})

    def usage_by_model_performance(self, start_date: Any = None, end_date: Any = None):
        return self._c._request("GET", "/api/usage/by-model/performance", None, query={"start_date": start_date, "end_date": end_date})

    def usage_daily(self, start_date: Any = None, end_date: Any = None, days: Any = None):
        return self._c._request("GET", "/api/usage/daily", None, query={"start_date": start_date, "end_date": end_date, "days": days})

    def usage_export(self, start_date: Any = None, end_date: Any = None, format: Any = None):
        return self._c._request("GET", "/api/usage/export", None, query={"start_date": start_date, "end_date": end_date, "format": format})

    def usage_summary(self, start_date: Any = None, end_date: Any = None):
        return self._c._request("GET", "/api/usage/summary", None, query={"start_date": start_date, "end_date": end_date})


# ── Channels Resource ──────────────────────────────────────────

class _ChannelsResource(_Resource):

    def list_channels(self):
        return self._c._request("GET", "/api/channels")

    def list_channel_registry(self):
        return self._c._request("GET", "/api/channels/registry")

    def reload_channels(self):
        return self._c._request("POST", "/api/channels/reload")

    def delete_sidecar_channel(self, name: str):
        return self._c._request("DELETE", f"/api/channels/sidecar/{quote(str(name), safe='')}")

    def configure_sidecar_channel(self, name: str, **data):
        return self._c._request("POST", f"/api/channels/sidecar/{quote(str(name), safe='')}/configure", data)

    def get_channel_qr(self, name: str):
        return self._c._request("GET", f"/api/channels/{quote(str(name), safe='')}/qr")


# ── Extensions Resource ────────────────────────────────────────

class _ExtensionsResource(_Resource):

    def list_extensions(self):
        return self._c._request("GET", "/api/extensions")

    def install_extension(self, **data):
        return self._c._request("POST", "/api/extensions/install", data)

    def uninstall_extension(self, **data):
        return self._c._request("POST", "/api/extensions/uninstall", data)

    def get_extension(self, name: str):
        return self._c._request("GET", f"/api/extensions/{quote(str(name), safe='')}")


# ── Goals Resource ─────────────────────────────────────────────

class _GoalsResource(_Resource):

    def list_goal_templates(self):
        return self._c._request("GET", "/api/goals/templates")


# ── Groups Resource ────────────────────────────────────────────

class _GroupsResource(_Resource):

    def list_groups(self):
        return self._c._request("GET", "/api/groups")

    def create_group(self, **data):
        return self._c._request("POST", "/api/groups", data)

    def get_group(self, name: str):
        return self._c._request("GET", f"/api/groups/{quote(str(name), safe='')}")

    def update_group(self, name: str, **data):
        return self._c._request("PUT", f"/api/groups/{quote(str(name), safe='')}", data)

    def delete_group(self, name: str):
        return self._c._request("DELETE", f"/api/groups/{quote(str(name), safe='')}")

    def add_group_member(self, name: str, user: str):
        return self._c._request("PUT", f"/api/groups/{quote(str(name), safe='')}/members/{quote(str(user), safe='')}")

    def remove_group_member(self, name: str, user: str):
        return self._c._request("DELETE", f"/api/groups/{quote(str(name), safe='')}/members/{quote(str(user), safe='')}")

    def user_groups(self, name: str):
        return self._c._request("GET", f"/api/users/{quote(str(name), safe='')}/groups")


# ── Hands Resource ─────────────────────────────────────────────

class _HandsResource(_Resource):

    def list_hands(self):
        return self._c._request("GET", "/api/hands")

    def list_active_hands(self):
        return self._c._request("GET", "/api/hands/active")

    def install_hand(self, **data):
        return self._c._request("POST", "/api/hands/install", data)

    def deactivate_hand(self, id: str):
        return self._c._request("DELETE", f"/api/hands/instances/{quote(str(id), safe='')}")

    def hand_instance_browser(self, id: str):
        return self._c._request("GET", f"/api/hands/instances/{quote(str(id), safe='')}/browser")

    def pause_hand(self, id: str):
        return self._c._request("POST", f"/api/hands/instances/{quote(str(id), safe='')}/pause")

    def resume_hand(self, id: str):
        return self._c._request("POST", f"/api/hands/instances/{quote(str(id), safe='')}/resume")

    def hand_stats(self, id: str):
        return self._c._request("GET", f"/api/hands/instances/{quote(str(id), safe='')}/stats")

    def install_hand_from_marketplace(self, **data):
        return self._c._request("POST", "/api/hands/marketplace/install", data)

    def reload_hands(self):
        return self._c._request("POST", "/api/hands/reload")

    def get_hand(self, hand_id: str):
        return self._c._request("GET", f"/api/hands/{quote(str(hand_id), safe='')}")

    def uninstall_hand(self, hand_id: str):
        return self._c._request("DELETE", f"/api/hands/{quote(str(hand_id), safe='')}")

    def activate_hand(self, hand_id: str, **data):
        return self._c._request("POST", f"/api/hands/{quote(str(hand_id), safe='')}/activate", data)

    def check_hand_deps(self, hand_id: str):
        return self._c._request("POST", f"/api/hands/{quote(str(hand_id), safe='')}/check-deps")

    def install_hand_deps(self, hand_id: str):
        return self._c._request("POST", f"/api/hands/{quote(str(hand_id), safe='')}/install-deps")

    def get_hand_manifest(self, hand_id: str):
        return self._c._request("GET", f"/api/hands/{quote(str(hand_id), safe='')}/manifest")

    def update_hand_manifest(self, hand_id: str, **data):
        return self._c._request("PUT", f"/api/hands/{quote(str(hand_id), safe='')}/manifest", data)

    def set_hand_secret(self, hand_id: str, **data):
        return self._c._request("POST", f"/api/hands/{quote(str(hand_id), safe='')}/secret", data)

    def get_hand_settings(self, hand_id: str):
        return self._c._request("GET", f"/api/hands/{quote(str(hand_id), safe='')}/settings")

    def update_hand_settings(self, hand_id: str, **data):
        return self._c._request("PUT", f"/api/hands/{quote(str(hand_id), safe='')}/settings", data)


# ── Inbox Resource ─────────────────────────────────────────────

class _InboxResource(_Resource):

    def inbox_status(self):
        return self._c._request("GET", "/api/inbox/status")


# ── Knowledge Resource ─────────────────────────────────────────

class _KnowledgeResource(_Resource):

    def list_bases(self):
        return self._c._request("GET", "/api/knowledge")

    def create_base(self, **data):
        return self._c._request("POST", "/api/knowledge", data)

    def delete_base(self, name: str):
        return self._c._request("DELETE", f"/api/knowledge/{quote(str(name), safe='')}")

    def set_holders(self, name: str, **data):
        return self._c._request("PUT", f"/api/knowledge/{quote(str(name), safe='')}/agents", data)

    def list_documents(self, name: str):
        return self._c._request("GET", f"/api/knowledge/{quote(str(name), safe='')}/documents")

    def put_document(self, name: str, filename: str, body: bytes, content_type: str = "application/octet-stream"):
        return self._c._request("PUT", f"/api/knowledge/{quote(str(name), safe='')}/documents/{quote(str(filename), safe='')}", body, content_type=content_type)

    def delete_document(self, name: str, filename: str):
        return self._c._request("DELETE", f"/api/knowledge/{quote(str(name), safe='')}/documents/{quote(str(filename), safe='')}")


# ── Mcp Resource ───────────────────────────────────────────────

class _McpResource(_Resource):

    def list_mcp_catalog(self):
        return self._c._request("GET", "/api/mcp/catalog")

    def get_mcp_catalog_entry(self, id: str):
        return self._c._request("GET", f"/api/mcp/catalog/{quote(str(id), safe='')}")

    def mcp_health_handler(self):
        return self._c._request("GET", "/api/mcp/health")

    def reload_mcp_handler(self):
        return self._c._request("POST", "/api/mcp/reload")

    def list_mcp_servers(self):
        return self._c._request("GET", "/api/mcp/servers")

    def add_mcp_server(self, **data):
        return self._c._request("POST", "/api/mcp/servers", data)

    def get_mcp_server(self, name: str):
        return self._c._request("GET", f"/api/mcp/servers/{quote(str(name), safe='')}")

    def update_mcp_server(self, name: str, **data):
        return self._c._request("PUT", f"/api/mcp/servers/{quote(str(name), safe='')}", data)

    def delete_mcp_server(self, name: str):
        return self._c._request("DELETE", f"/api/mcp/servers/{quote(str(name), safe='')}")

    def auth_revoke(self, name: str):
        return self._c._request("DELETE", f"/api/mcp/servers/{quote(str(name), safe='')}/auth/revoke")

    def auth_start(self, name: str):
        return self._c._request("POST", f"/api/mcp/servers/{quote(str(name), safe='')}/auth/start")

    def auth_status(self, name: str):
        return self._c._request("GET", f"/api/mcp/servers/{quote(str(name), safe='')}/auth/status")

    def reconnect_mcp_server_handler(self, name: str):
        return self._c._request("POST", f"/api/mcp/servers/{quote(str(name), safe='')}/reconnect")

    def patch_mcp_server_taint(self, name: str, **data):
        return self._c._request("PATCH", f"/api/mcp/servers/{quote(str(name), safe='')}/taint", data)

    def list_mcp_taint_rules(self):
        return self._c._request("GET", "/api/mcp/taint-rules")


# ── Media Resource ─────────────────────────────────────────────

class _MediaResource(_Resource):

    def generate_image(self, **data):
        return self._c._request("POST", "/api/media/image", data)

    def generate_music(self, **data):
        return self._c._request("POST", "/api/media/music", data)

    def list_media_providers(self):
        return self._c._request("GET", "/api/media/providers")

    def synthesize_speech(self, **data):
        return self._c._request("POST", "/api/media/speech", data)

    def transcribe_audio(self, body: bytes, content_type: str = "audio/webm"):
        return self._c._request("POST", "/api/media/transcribe", body, content_type=content_type)

    def submit_video(self, **data):
        return self._c._request("POST", "/api/media/video", data)

    def poll_video_task(self, task_id: str, provider: Any = None):
        return self._c._request("GET", f"/api/media/video/{quote(str(task_id), safe='')}", None, query={"provider": provider})


# ── Memory Resource ────────────────────────────────────────────

class _MemoryResource(_Resource):

    def export_agent_memory(self, id: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/memory/export")

    def import_agent_memory(self, id: str, **data):
        return self._c._request("POST", f"/api/agents/{quote(str(id), safe='')}/memory/import", data)

    def get_agent_kv(self, id: str):
        return self._c._request("GET", f"/api/memory/agents/{quote(str(id), safe='')}/kv")

    def get_agent_kv_key(self, id: str, key: str):
        return self._c._request("GET", f"/api/memory/agents/{quote(str(id), safe='')}/kv/{quote(str(key), safe='')}")

    def set_agent_kv_key(self, id: str, key: str, **data):
        return self._c._request("PUT", f"/api/memory/agents/{quote(str(id), safe='')}/kv/{quote(str(key), safe='')}", data)

    def delete_agent_kv_key(self, id: str, key: str):
        return self._c._request("DELETE", f"/api/memory/agents/{quote(str(id), safe='')}/kv/{quote(str(key), safe='')}")

    def memory_config_get(self):
        return self._c._request("GET", "/api/memory/config")

    def memory_config_patch(self, **data):
        return self._c._request("PATCH", "/api/memory/config", data)


# ── Models Resource ────────────────────────────────────────────

class _ModelsResource(_Resource):

    def catalog_status(self):
        return self._c._request("GET", "/api/catalog/status")

    def catalog_update(self):
        return self._c._request("POST", "/api/catalog/update")

    def list_credential_pools(self):
        return self._c._request("GET", "/api/credential-pools")

    def list_model_router_profiles(self):
        return self._c._request("GET", "/api/model-router/profiles")

    def list_all_models(self):
        return self._c._request("GET", "/api/models")

    def list_aliases(self):
        return self._c._request("GET", "/api/models/aliases")

    def create_alias(self, **data):
        return self._c._request("POST", "/api/models/aliases", data)

    def delete_alias(self, alias: str):
        return self._c._request("DELETE", f"/api/models/aliases/{quote(str(alias), safe='')}")

    def add_custom_model(self, **data):
        return self._c._request("POST", "/api/models/custom", data)

    def remove_custom_model(self, id: str):
        return self._c._request("DELETE", f"/api/models/custom/{quote(str(id), safe='')}")

    def get_model_overrides(self, id: str):
        return self._c._request("GET", f"/api/models/overrides/{quote(str(id), safe='')}")

    def set_model_overrides(self, id: str, **data):
        return self._c._request("PUT", f"/api/models/overrides/{quote(str(id), safe='')}", data)

    def delete_model_overrides(self, id: str):
        return self._c._request("DELETE", f"/api/models/overrides/{quote(str(id), safe='')}")

    def get_model(self, id: str):
        return self._c._request("GET", f"/api/models/{quote(str(id), safe='')}")

    def list_providers(self):
        return self._c._request("GET", "/api/providers")

    def copilot_oauth_poll(self, poll_id: str):
        return self._c._request("GET", f"/api/providers/github-copilot/oauth/poll/{quote(str(poll_id), safe='')}")

    def copilot_oauth_start(self):
        return self._c._request("POST", "/api/providers/github-copilot/oauth/start")

    def get_provider(self, name: str):
        return self._c._request("GET", f"/api/providers/{quote(str(name), safe='')}")

    def set_default_provider(self, name: str, **data):
        return self._c._request("POST", f"/api/providers/{quote(str(name), safe='')}/default", data)

    def set_provider_discovery(self, name: str, **data):
        return self._c._request("PUT", f"/api/providers/{quote(str(name), safe='')}/discovery", data)

    def enable_provider(self, name: str):
        return self._c._request("POST", f"/api/providers/{quote(str(name), safe='')}/enable")

    def set_provider_key(self, name: str, **data):
        return self._c._request("POST", f"/api/providers/{quote(str(name), safe='')}/key", data)

    def delete_provider_key(self, name: str):
        return self._c._request("DELETE", f"/api/providers/{quote(str(name), safe='')}/key")

    def test_provider(self, name: str):
        return self._c._request("POST", f"/api/providers/{quote(str(name), safe='')}/test")

    def set_provider_url(self, name: str, **data):
        return self._c._request("PUT", f"/api/providers/{quote(str(name), safe='')}/url", data)


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

    def network_trusted_peers(self):
        return self._c._request("GET", "/api/network/trusted-peers")

    def list_peers(self, offset: Any = None, limit: Any = None):
        return self._c._request("GET", "/api/peers", None, query={"offset": offset, "limit": limit})

    def get_peer(self, id: str):
        return self._c._request("GET", f"/api/peers/{quote(str(id), safe='')}")


# ── Pairing Resource ───────────────────────────────────────────

class _PairingResource(_Resource):

    def pairing_complete(self, **data):
        return self._c._request("POST", "/api/pairing/complete", data)

    def pairing_devices(self):
        return self._c._request("GET", "/api/pairing/devices")

    def pairing_remove_device(self, id: str):
        return self._c._request("DELETE", f"/api/pairing/devices/{quote(str(id), safe='')}")

    def pairing_notify(self, **data):
        return self._c._request("POST", "/api/pairing/notify", data)

    def pairing_request(self):
        return self._c._request("POST", "/api/pairing/request")


# ── Plugins Resource ───────────────────────────────────────────

class _PluginsResource(_Resource):

    def context_engine_chain(self):
        return self._c._request("GET", "/api/context-engine/chain")

    def context_engine_config(self):
        return self._c._request("GET", "/api/context-engine/config")

    def context_engine_health(self):
        return self._c._request("GET", "/api/context-engine/health")

    def context_engine_metrics(self):
        return self._c._request("GET", "/api/context-engine/metrics")

    def context_engine_sandbox_policy(self):
        return self._c._request("GET", "/api/context-engine/sandbox-policy")

    def context_engine_traces(self):
        return self._c._request("GET", "/api/context-engine/traces")

    def list_plugins(self):
        return self._c._request("GET", "/api/plugins")

    def plugin_doctor(self):
        return self._c._request("GET", "/api/plugins/doctor")

    def install_plugin(self, **data):
        return self._c._request("POST", "/api/plugins/install", data)

    def list_plugin_registries(self):
        return self._c._request("GET", "/api/plugins/registries")

    def scaffold_plugin(self, **data):
        return self._c._request("POST", "/api/plugins/scaffold", data)

    def uninstall_plugin(self, **data):
        return self._c._request("POST", "/api/plugins/uninstall", data)

    def get_plugin(self, name: str):
        return self._c._request("GET", f"/api/plugins/{quote(str(name), safe='')}")

    def plugin_advanced_config(self, name: str):
        return self._c._request("GET", f"/api/plugins/{quote(str(name), safe='')}/advanced-config")

    def disable_plugin(self, name: str):
        return self._c._request("POST", f"/api/plugins/{quote(str(name), safe='')}/disable")

    def enable_plugin(self, name: str):
        return self._c._request("POST", f"/api/plugins/{quote(str(name), safe='')}/enable")

    def plugin_env(self, name: str):
        return self._c._request("GET", f"/api/plugins/{quote(str(name), safe='')}/env")

    def install_plugin_deps(self, name: str):
        return self._c._request("POST", f"/api/plugins/{quote(str(name), safe='')}/install-deps")

    def lint_plugin(self, name: str):
        return self._c._request("GET", f"/api/plugins/{quote(str(name), safe='')}/lint")

    def prewarm_plugin(self, name: str):
        return self._c._request("POST", f"/api/plugins/{quote(str(name), safe='')}/prewarm")

    def reload_plugin(self, name: str):
        return self._c._request("POST", f"/api/plugins/{quote(str(name), safe='')}/reload")

    def sign_plugin(self, name: str):
        return self._c._request("POST", f"/api/plugins/{quote(str(name), safe='')}/sign")

    def plugin_status(self, name: str):
        return self._c._request("GET", f"/api/plugins/{quote(str(name), safe='')}/status")

    def test_plugin_hook(self, name: str, **data):
        return self._c._request("POST", f"/api/plugins/{quote(str(name), safe='')}/test-hook", data)

    def upgrade_plugin(self, name: str, **data):
        return self._c._request("POST", f"/api/plugins/{quote(str(name), safe='')}/upgrade", data)


# ── ProactiveMemory Resource ──────────────────────────────────

class _ProactiveMemoryResource(_Resource):

    def memory_list(self, category: Any = None, level: Any = None, offset: Any = None, limit: Any = None):
        return self._c._request("GET", "/api/memory", None, query={"category": category, "level": level, "offset": offset, "limit": limit})

    def memory_add(self, **data):
        return self._c._request("POST", "/api/memory", data)

    def memory_list_agent(self, id: str, category: Any = None, level: Any = None, offset: Any = None, limit: Any = None):
        return self._c._request("GET", f"/api/memory/agents/{quote(str(id), safe='')}", None, query={"category": category, "level": level, "offset": offset, "limit": limit})

    def memory_reset_agent(self, id: str):
        return self._c._request("DELETE", f"/api/memory/agents/{quote(str(id), safe='')}")

    def memory_consolidate(self, id: str):
        return self._c._request("POST", f"/api/memory/agents/{quote(str(id), safe='')}/consolidate")

    def memory_count_agent(self, id: str, level: Any = None):
        return self._c._request("GET", f"/api/memory/agents/{quote(str(id), safe='')}/count", None, query={"level": level})

    def memory_duplicates(self, id: str):
        return self._c._request("GET", f"/api/memory/agents/{quote(str(id), safe='')}/duplicates")

    def memory_export_agent(self, id: str):
        return self._c._request("GET", f"/api/memory/agents/{quote(str(id), safe='')}/export")

    def memory_import_agent(self, id: str, **data):
        return self._c._request("POST", f"/api/memory/agents/{quote(str(id), safe='')}/import", data)

    def memory_clear_level(self, id: str, level: str):
        return self._c._request("DELETE", f"/api/memory/agents/{quote(str(id), safe='')}/level/{quote(str(level), safe='')}")

    def memory_query_relations(self, id: str, source: Any = None, relation: Any = None, target: Any = None):
        return self._c._request("GET", f"/api/memory/agents/{quote(str(id), safe='')}/relations", None, query={"source": source, "relation": relation, "target": target})

    def memory_store_relations(self, id: str, **data):
        return self._c._request("POST", f"/api/memory/agents/{quote(str(id), safe='')}/relations", data)

    def memory_search_agent(self, id: str, q: Any = None, level: Any = None, limit: Any = None):
        return self._c._request("GET", f"/api/memory/agents/{quote(str(id), safe='')}/search", None, query={"q": q, "level": level, "limit": limit})

    def memory_stats_agent(self, id: str):
        return self._c._request("GET", f"/api/memory/agents/{quote(str(id), safe='')}/stats")

    def memory_bulk_delete(self, **data):
        return self._c._request("POST", "/api/memory/bulk-delete", data)

    def memory_cleanup(self):
        return self._c._request("POST", "/api/memory/cleanup")

    def memory_decay(self):
        return self._c._request("POST", "/api/memory/decay")

    def memory_update(self, memory_id: str, **data):
        return self._c._request("PUT", f"/api/memory/items/{quote(str(memory_id), safe='')}", data)

    def memory_delete(self, memory_id: str):
        return self._c._request("DELETE", f"/api/memory/items/{quote(str(memory_id), safe='')}")

    def memory_history(self, memory_id: str):
        return self._c._request("GET", f"/api/memory/items/{quote(str(memory_id), safe='')}/history")

    def memory_search(self, q: Any = None, level: Any = None, limit: Any = None):
        return self._c._request("GET", "/api/memory/search", None, query={"q": q, "level": level, "limit": limit})

    def memory_stats(self):
        return self._c._request("GET", "/api/memory/stats")

    def memory_get_user(self, user_id: str):
        return self._c._request("GET", f"/api/memory/user/{quote(str(user_id), safe='')}")


# ── Sessions Resource ──────────────────────────────────────────

class _SessionsResource(_Resource):

    def find_session_by_label(self, id: str, label: str):
        return self._c._request("GET", f"/api/agents/{quote(str(id), safe='')}/sessions/by-label/{quote(str(label), safe='')}")

    def list_sessions(self, limit: Any = None, offset: Any = None):
        return self._c._request("GET", "/api/sessions", None, query={"limit": limit, "offset": offset})

    def session_cleanup(self):
        return self._c._request("POST", "/api/sessions/cleanup")

    def search_sessions(self, q: Any = None, agent_id: Any = None, limit: Any = None, offset: Any = None):
        return self._c._request("GET", "/api/sessions/search", None, query={"q": q, "agent_id": agent_id, "limit": limit, "offset": offset})

    def get_session(self, id: str):
        return self._c._request("GET", f"/api/sessions/{quote(str(id), safe='')}")

    def delete_session(self, id: str):
        return self._c._request("DELETE", f"/api/sessions/{quote(str(id), safe='')}")

    def set_session_label(self, id: str, **data):
        return self._c._request("PUT", f"/api/sessions/{quote(str(id), safe='')}/label", data)

    def patch_session_model(self, id: str, **data):
        return self._c._request("PATCH", f"/api/sessions/{quote(str(id), safe='')}/model", data)

    def patch_session_reasoning(self, id: str, **data):
        return self._c._request("PATCH", f"/api/sessions/{quote(str(id), safe='')}/reasoning", data)


# ── Skills Resource ────────────────────────────────────────────

class _SkillsResource(_Resource):

    def clawhub_browse(self, q: Any = None):
        return self._c._request("GET", "/api/clawhub/browse", None, query={"q": q})

    def clawhub_install(self, **data):
        return self._c._request("POST", "/api/clawhub/install", data)

    def clawhub_search(self, q: Any = None):
        return self._c._request("GET", "/api/clawhub/search", None, query={"q": q})

    def clawhub_skill_detail(self, slug: str):
        return self._c._request("GET", f"/api/clawhub/skill/{quote(str(slug), safe='')}")

    def clawhub_skill_code(self, slug: str):
        return self._c._request("GET", f"/api/clawhub/skill/{quote(str(slug), safe='')}/code")

    def marketplace_search(self, q: Any = None):
        return self._c._request("GET", "/api/marketplace/search", None, query={"q": q})

    def list_skills(self):
        return self._c._request("GET", "/api/skills")

    def create_skill(self, **data):
        return self._c._request("POST", "/api/skills/create", data)

    def install_skill(self, **data):
        return self._c._request("POST", "/api/skills/install", data)

    def list_pending_candidates(self, agent: Any = None):
        return self._c._request("GET", "/api/skills/pending", None, query={"agent": agent})

    def show_pending_candidate(self, id: str):
        return self._c._request("GET", f"/api/skills/pending/{quote(str(id), safe='')}")

    def approve_pending_candidate(self, id: str):
        return self._c._request("POST", f"/api/skills/pending/{quote(str(id), safe='')}/approve")

    def propose_pending_to_registry(self, id: str):
        return self._c._request("POST", f"/api/skills/pending/{quote(str(id), safe='')}/propose-to-registry")

    def reject_pending_candidate(self, id: str):
        return self._c._request("POST", f"/api/skills/pending/{quote(str(id), safe='')}/reject")

    def list_skill_registry(self):
        return self._c._request("GET", "/api/skills/registry")

    def reload_skills(self):
        return self._c._request("POST", "/api/skills/reload")

    def uninstall_skill(self, **data):
        return self._c._request("POST", "/api/skills/uninstall", data)

    def get_skill_detail(self, name: str):
        return self._c._request("GET", f"/api/skills/{quote(str(name), safe='')}")

    def evolve_delete_skill(self, name: str):
        return self._c._request("POST", f"/api/skills/{quote(str(name), safe='')}/evolve/delete")

    def evolve_write_file(self, name: str, **data):
        return self._c._request("POST", f"/api/skills/{quote(str(name), safe='')}/evolve/file", data)

    def evolve_remove_file(self, name: str, path: Any = None):
        return self._c._request("DELETE", f"/api/skills/{quote(str(name), safe='')}/evolve/file", None, query={"path": path})

    def evolve_patch_skill(self, name: str, **data):
        return self._c._request("POST", f"/api/skills/{quote(str(name), safe='')}/evolve/patch", data)

    def evolve_rollback_skill(self, name: str):
        return self._c._request("POST", f"/api/skills/{quote(str(name), safe='')}/evolve/rollback")

    def evolve_update_skill(self, name: str, **data):
        return self._c._request("POST", f"/api/skills/{quote(str(name), safe='')}/evolve/update", data)

    def get_supporting_file(self, name: str, path: Any = None):
        return self._c._request("GET", f"/api/skills/{quote(str(name), safe='')}/file", None, query={"path": path})

    def propose_skill_to_registry(self, name: str):
        return self._c._request("POST", f"/api/skills/{quote(str(name), safe='')}/propose")

    def list_tools(self):
        return self._c._request("GET", "/api/tools")

    def get_tool(self, name: str):
        return self._c._request("GET", f"/api/tools/{quote(str(name), safe='')}")


# ── System Resource ────────────────────────────────────────────

class _SystemResource(_Resource):

    def audit_export(self, format: Any = None, user: Any = None, action: Any = None, agent: Any = None, channel: Any = None, from_: Any = None, to: Any = None, limit: Any = None):
        return self._c._request("GET", "/api/audit/export", None, query={"format": format, "user": user, "action": action, "agent": agent, "channel": channel, "from": from_, "to": to, "limit": limit})

    def audit_query(self, user: Any = None, action: Any = None, agent: Any = None, channel: Any = None, from_: Any = None, to: Any = None, limit: Any = None):
        return self._c._request("GET", "/api/audit/query", None, query={"user": user, "action": action, "agent": agent, "channel": channel, "from": from_, "to": to, "limit": limit})

    def audit_recent(self):
        return self._c._request("GET", "/api/audit/recent")

    def audit_verify(self):
        return self._c._request("GET", "/api/audit/verify")

    def check(self, user: Any = None, action: Any = None, channel: Any = None):
        return self._c._request("GET", "/api/authz/check", None, query={"user": user, "action": action, "channel": channel})

    def effective_permissions(self, user_id: str):
        return self._c._request("GET", f"/api/authz/effective/{quote(str(user_id), safe='')}")

    def whoami(self):
        return self._c._request("GET", "/api/authz/whoami")

    def create_backup(self):
        return self._c._request("POST", "/api/backup")

    def list_backups(self):
        return self._c._request("GET", "/api/backups")

    def delete_backup(self, filename: str):
        return self._c._request("DELETE", f"/api/backups/{quote(str(filename), safe='')}")

    def list_bindings(self):
        return self._c._request("GET", "/api/bindings")

    def add_binding(self, **data):
        return self._c._request("POST", "/api/bindings", data)

    def remove_binding(self, index: str):
        return self._c._request("DELETE", f"/api/bindings/{quote(str(index), safe='')}")

    def list_commands(self):
        return self._c._request("GET", "/api/commands")

    def get_command(self, name: str):
        return self._c._request("GET", f"/api/commands/{quote(str(name), safe='')}")

    def get_config(self):
        return self._c._request("GET", "/api/config")

    def export_config(self):
        return self._c._request("GET", "/api/config/export")

    def config_reload(self):
        return self._c._request("POST", "/api/config/reload")

    def config_schema(self):
        return self._c._request("GET", "/api/config/schema")

    def config_set(self, **data):
        return self._c._request("POST", "/api/config/set", data)

    def config_status(self):
        return self._c._request("GET", "/api/config/status")

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
        return self._c._request("GET", f"/api/profiles/{quote(str(name), safe='')}")

    def provisioning_status(self):
        return self._c._request("GET", "/api/provisioning/status")

    def queue_status(self):
        return self._c._request("GET", "/api/queue/status")

    def ready(self):
        return self._c._request("GET", "/api/ready")

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

    def create_agent_type(self, **data):
        return self._c._request("POST", "/api/templates", data)

    def get_agent_template(self, name: str):
        return self._c._request("GET", f"/api/templates/{quote(str(name), safe='')}")

    def update_agent_type(self, name: str, **data):
        return self._c._request("PUT", f"/api/templates/{quote(str(name), safe='')}", data)

    def delete_agent_type(self, name: str):
        return self._c._request("DELETE", f"/api/templates/{quote(str(name), safe='')}")

    def list_template_history(self, name: str, limit: Any = None):
        return self._c._request("GET", f"/api/templates/{quote(str(name), safe='')}/history", None, query={"limit": limit})

    def restore_template_version(self, name: str, version_id: str):
        return self._c._request("POST", f"/api/templates/{quote(str(name), safe='')}/history/{quote(str(version_id), safe='')}/restore")

    def promote_agent_type(self, name: str):
        return self._c._request("POST", f"/api/templates/{quote(str(name), safe='')}/promote")

    def get_agent_type_registry_diff(self, name: str):
        return self._c._request("GET", f"/api/templates/{quote(str(name), safe='')}/registry-diff")

    def restore_agent_type_from_registry(self, name: str):
        return self._c._request("POST", f"/api/templates/{quote(str(name), safe='')}/restore")

    def get_agent_template_toml(self, name: str):
        return self._c._request("GET", f"/api/templates/{quote(str(name), safe='')}/toml")

    def put_agent_template_toml(self, name: str, body: bytes, content_type: str = "text/plain"):
        return self._c._request("PUT", f"/api/templates/{quote(str(name), safe='')}/toml", body, content_type=content_type)

    def post_agent_template_toml(self, name: str, body: bytes, content_type: str = "text/plain"):
        return self._c._request("POST", f"/api/templates/{quote(str(name), safe='')}/toml", body, content_type=content_type)

    def version(self):
        return self._c._request("GET", "/api/version")

    def api_versions(self):
        return self._c._request("GET", "/api/versions")


# ── Tasks Resource ─────────────────────────────────────────────

class _TasksResource(_Resource):

    def task_queue_post_root(self, **data):
        return self._c._request("POST", "/api/tasks", data)


# ── Tools Resource ─────────────────────────────────────────────

class _ToolsResource(_Resource):

    def invoke_tool(self, name: str, agent_id: Any = None, **data):
        return self._c._request("POST", f"/api/tools/{quote(str(name), safe='')}/invoke", data, query={"agent_id": agent_id})


# ── Users Resource ─────────────────────────────────────────────

class _UsersResource(_Resource):

    def list_users(self):
        return self._c._request("GET", "/api/users")

    def create_user(self, **data):
        return self._c._request("POST", "/api/users", data)

    def import_users(self, **data):
        return self._c._request("POST", "/api/users/import", data)

    def serve_my_avatar(self):
        return self._c._request("GET", "/api/users/me/avatar")

    def get_user(self, name: str):
        return self._c._request("GET", f"/api/users/{quote(str(name), safe='')}")

    def update_user(self, name: str, **data):
        return self._c._request("PUT", f"/api/users/{quote(str(name), safe='')}", data)

    def delete_user(self, name: str):
        return self._c._request("DELETE", f"/api/users/{quote(str(name), safe='')}")

    def serve_user_avatar(self, name: str):
        return self._c._request("GET", f"/api/users/{quote(str(name), safe='')}/avatar")

    def upload_user_avatar(self, name: str, body: bytes, content_type: str = "application/octet-stream"):
        return self._c._request("POST", f"/api/users/{quote(str(name), safe='')}/avatar", body, content_type=content_type)

    def delete_user_avatar(self, name: str):
        return self._c._request("DELETE", f"/api/users/{quote(str(name), safe='')}/avatar")

    def update_user_identity(self, name: str, **data):
        return self._c._request("PATCH", f"/api/users/{quote(str(name), safe='')}/identity", data)

    def get_user_policy(self, name: str):
        return self._c._request("GET", f"/api/users/{quote(str(name), safe='')}/policy")

    def update_user_policy(self, name: str, **data):
        return self._c._request("PUT", f"/api/users/{quote(str(name), safe='')}/policy", data)

    def list_user_provider_keys(self, name: str):
        return self._c._request("GET", f"/api/users/{quote(str(name), safe='')}/provider-keys")

    def set_user_provider_key(self, name: str, provider: str, **data):
        return self._c._request("PUT", f"/api/users/{quote(str(name), safe='')}/provider-keys/{quote(str(provider), safe='')}", data)

    def delete_user_provider_key(self, name: str, provider: str):
        return self._c._request("DELETE", f"/api/users/{quote(str(name), safe='')}/provider-keys/{quote(str(provider), safe='')}")

    def rotate_user_key(self, name: str):
        return self._c._request("POST", f"/api/users/{quote(str(name), safe='')}/rotate-key")


# ── Vault Resource ─────────────────────────────────────────────

class _VaultResource(_Resource):

    def vault_list_keys(self):
        return self._c._request("GET", "/api/vault/keys")

    def vault_put_key(self, key: str, **data):
        return self._c._request("PUT", f"/api/vault/keys/{quote(str(key), safe='')}", data)

    def vault_delete_key(self, key: str):
        return self._c._request("DELETE", f"/api/vault/keys/{quote(str(key), safe='')}")


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

    def get_cron_job(self, id: str):
        return self._c._request("GET", f"/api/cron/jobs/{quote(str(id), safe='')}")

    def update_cron_job(self, id: str, **data):
        return self._c._request("PUT", f"/api/cron/jobs/{quote(str(id), safe='')}", data)

    def delete_cron_job(self, id: str):
        return self._c._request("DELETE", f"/api/cron/jobs/{quote(str(id), safe='')}")

    def toggle_cron_job(self, id: str, **data):
        return self._c._request("PUT", f"/api/cron/jobs/{quote(str(id), safe='')}/enable", data)

    def cron_job_status(self, id: str):
        return self._c._request("GET", f"/api/cron/jobs/{quote(str(id), safe='')}/status")

    def list_schedules(self):
        return self._c._request("GET", "/api/schedules")

    def create_schedule(self, **data):
        return self._c._request("POST", "/api/schedules", data)

    def get_schedule(self, id: str):
        return self._c._request("GET", f"/api/schedules/{quote(str(id), safe='')}")

    def update_schedule(self, id: str, **data):
        return self._c._request("PUT", f"/api/schedules/{quote(str(id), safe='')}", data)

    def delete_schedule(self, id: str):
        return self._c._request("DELETE", f"/api/schedules/{quote(str(id), safe='')}")

    def run_schedule(self, id: str):
        return self._c._request("POST", f"/api/schedules/{quote(str(id), safe='')}/run")

    def list_triggers(self, agent_id: Any = None):
        return self._c._request("GET", "/api/triggers", None, query={"agent_id": agent_id})

    def create_trigger(self, **data):
        return self._c._request("POST", "/api/triggers", data)

    def get_trigger(self, id: str):
        return self._c._request("GET", f"/api/triggers/{quote(str(id), safe='')}")

    def delete_trigger(self, id: str):
        return self._c._request("DELETE", f"/api/triggers/{quote(str(id), safe='')}")

    def update_trigger(self, id: str, **data):
        return self._c._request("PATCH", f"/api/triggers/{quote(str(id), safe='')}", data)

    def list_workflow_templates(self, q: Any = None, category: Any = None):
        return self._c._request("GET", "/api/workflow-templates", None, query={"q": q, "category": category})

    def get_workflow_template(self, id: str):
        return self._c._request("GET", f"/api/workflow-templates/{quote(str(id), safe='')}")

    def instantiate_template(self, id: str, **data):
        return self._c._request("POST", f"/api/workflow-templates/{quote(str(id), safe='')}/instantiate", data)

    def list_workflows(self):
        return self._c._request("GET", "/api/workflows")

    def create_workflow(self, **data):
        return self._c._request("POST", "/api/workflows", data)

    def get_workflow_run(self, run_id: str):
        return self._c._request("GET", f"/api/workflows/runs/{quote(str(run_id), safe='')}")

    def cancel_workflow_run(self, run_id: str):
        return self._c._request("POST", f"/api/workflows/runs/{quote(str(run_id), safe='')}/cancel")

    def operator_action_workflow_run(self, run_id: str, **data):
        return self._c._request("POST", f"/api/workflows/runs/{quote(str(run_id), safe='')}/operator", data)

    def pause_workflow_run(self, run_id: str, **data):
        return self._c._request("POST", f"/api/workflows/runs/{quote(str(run_id), safe='')}/pause", data)

    def rerun_workflow_run(self, run_id: str):
        return self._c._request("POST", f"/api/workflows/runs/{quote(str(run_id), safe='')}/rerun")

    def resume_workflow_run(self, run_id: str, **data):
        return self._c._request("POST", f"/api/workflows/runs/{quote(str(run_id), safe='')}/resume", data)

    def get_workflow(self, id: str):
        return self._c._request("GET", f"/api/workflows/{quote(str(id), safe='')}")

    def update_workflow(self, id: str, **data):
        return self._c._request("PUT", f"/api/workflows/{quote(str(id), safe='')}", data)

    def delete_workflow(self, id: str):
        return self._c._request("DELETE", f"/api/workflows/{quote(str(id), safe='')}")

    def dry_run_workflow(self, id: str, **data):
        return self._c._request("POST", f"/api/workflows/{quote(str(id), safe='')}/dry-run", data)

    def run_workflow(self, id: str, **data):
        return self._c._request("POST", f"/api/workflows/{quote(str(id), safe='')}/run", data)

    def list_workflow_runs(self, id: str):
        return self._c._request("GET", f"/api/workflows/{quote(str(id), safe='')}/runs")

    def save_workflow_as_template(self, id: str):
        return self._c._request("POST", f"/api/workflows/{quote(str(id), safe='')}/save-as-template")

