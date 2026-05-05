/**
 * @librefang/sdk — AUTO-GENERATED from openapi.json.
 * Do not edit manually. Run: python3 scripts/codegen-sdks.py
 *
 * Usage:
 *   const { LibreFang } = require("@librefang/sdk");
 *   const client = new LibreFang("http://localhost:4545");
 *
 *   const agents = await client.agents.listAgents();
 *
 *   // Streaming:
 *   for await (const event of client.agents.sendMessageStream(agentId, { message: "Hello" })) {
 *     process.stdout.write(event.delta || "");
 *   }
 */

"use strict";

class LibreFangError extends Error {
  constructor(message, status, body) {
    super(message);
    this.name = "LibreFangError";
    this.status = status;
    this.body = body;
  }
}

class LibreFang {
  constructor(baseUrl, opts) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this._headers = Object.assign({ "Content-Type": "application/json" }, (opts && opts.headers) || {});
    this.a2a = new A2AResource(this);
    this.agents = new AgentsResource(this);
    this.approvals = new ApprovalsResource(this);
    this.auth = new AuthResource(this);
    this.auto_dream = new AutoDreamResource(this);
    this.budget = new BudgetResource(this);
    this.channels = new ChannelsResource(this);
    this.extensions = new ExtensionsResource(this);
    this.hands = new HandsResource(this);
    this.mcp = new McpResource(this);
    this.memory = new MemoryResource(this);
    this.models = new ModelsResource(this);
    this.network = new NetworkResource(this);
    this.pairing = new PairingResource(this);
    this.proactive_memory = new ProactiveMemoryResource(this);
    this.sessions = new SessionsResource(this);
    this.skills = new SkillsResource(this);
    this.system = new SystemResource(this);
    this.tools = new ToolsResource(this);
    this.users = new UsersResource(this);
    this.webhooks = new WebhooksResource(this);
    this.workflows = new WorkflowsResource(this);
  }

  _withQuery(path, query) {
    if (!query) return path;
    const params = new URLSearchParams();
    for (const [k, v] of Object.entries(query)) {
      if (v === undefined || v === null) continue;
      params.append(k, String(v));
    }
    const q = params.toString();
    if (!q) return path;
    return path + (path.includes("?") ? "&" : "?") + q;
  }

  async _request(method, path, body, query) {
    const url = this.baseUrl + this._withQuery(path, query);
    const opts = { method, headers: this._headers };
    if (body !== undefined && body !== null) opts.body = JSON.stringify(body);
    const res = await fetch(url, opts);
    const text = await res.text();
    if (!res.ok) throw new LibreFangError(`HTTP ${res.status}: ${text}`, res.status, text);
    const ct = res.headers.get("content-type") || "";
    return ct.includes("application/json") ? JSON.parse(text) : text;
  }

  async *_stream(method, path, body, query) {
    const url = this.baseUrl + this._withQuery(path, query);
    const headers = Object.assign({}, this._headers, { Accept: "text/event-stream" });
    const opts = { method, headers };
    if (body !== undefined && body !== null) opts.body = JSON.stringify(body);
    const res = await fetch(url, opts);
    if (!res.ok) {
      const text = await res.text();
      throw new LibreFangError(`HTTP ${res.status}: ${text}`, res.status, text);
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop();
      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed.startsWith("data: ")) continue;
        const data = trimmed.slice(6);
        if (data === "[DONE]") return;
        try { yield JSON.parse(data); } catch { yield { raw: data }; }
      }
    }
  }
}

// ── A2A Resource

class A2AResource {
  constructor(client) { this._c = client; }

  async a2aListExternalAgents() {
    return this._c._request("GET", "/api/a2a/agents");
  }

  async a2aGetExternalAgent(id) {
    return this._c._request("GET", `/api/a2a/agents/${id}`);
  }

  async a2aDiscoverExternal(data) {
    return this._c._request("POST", "/api/a2a/discover", data, undefined);
  }

  async a2aSendExternal(data) {
    return this._c._request("POST", "/api/a2a/send", data, undefined);
  }

  async a2aExternalTaskStatus(id, query) {
    return this._c._request("GET", `/api/a2a/tasks/${id}/status`, undefined, query);
  }
}

// ── Agents Resource

class AgentsResource {
  constructor(client) { this._c = client; }

  async listAgents(query) {
    return this._c._request("GET", "/api/agents", undefined, query);
  }

  async spawnAgent(data) {
    return this._c._request("POST", "/api/agents", data, undefined);
  }

  async bulkCreateAgents(data) {
    return this._c._request("POST", "/api/agents/bulk", data, undefined);
  }

  async bulkDeleteAgents() {
    return this._c._request("DELETE", "/api/agents/bulk");
  }

  async bulkStartAgents(data) {
    return this._c._request("POST", "/api/agents/bulk/start", data, undefined);
  }

  async bulkStopAgents(data) {
    return this._c._request("POST", "/api/agents/bulk/stop", data, undefined);
  }

  async listAgentIdentities() {
    return this._c._request("GET", "/api/agents/identities");
  }

  async resetAgentIdentity(name, query) {
    return this._c._request("POST", `/api/agents/identities/${name}/reset`, undefined, query);
  }

  async getAgent(id) {
    return this._c._request("GET", `/api/agents/${id}`);
  }

  async killAgent(id, query) {
    return this._c._request("DELETE", `/api/agents/${id}`, undefined, query);
  }

  async patchAgent(id, data) {
    return this._c._request("PATCH", `/api/agents/${id}`, data, undefined);
  }

  async cloneAgent(id, data) {
    return this._c._request("POST", `/api/agents/${id}/clone`, data, undefined);
  }

  async patchAgentConfig(id, data) {
    return this._c._request("PATCH", `/api/agents/${id}/config`, data, undefined);
  }

  async getAgentDeliveries(id) {
    return this._c._request("GET", `/api/agents/${id}/deliveries`);
  }

  async listAgentEvents(id, query) {
    return this._c._request("GET", `/api/agents/${id}/events`, undefined, query);
  }

  async listAgentFiles(id) {
    return this._c._request("GET", `/api/agents/${id}/files`);
  }

  async getAgentFile(id, filename) {
    return this._c._request("GET", `/api/agents/${id}/files/${filename}`);
  }

  async setAgentFile(id, filename, data) {
    return this._c._request("PUT", `/api/agents/${id}/files/${filename}`, data, undefined);
  }

  async deleteAgentFile(id, filename) {
    return this._c._request("DELETE", `/api/agents/${id}/files/${filename}`);
  }

  async deleteHandAgentRuntimeConfig(id) {
    return this._c._request("DELETE", `/api/agents/${id}/hand-runtime-config`);
  }

  async patchHandAgentRuntimeConfig(id, data) {
    return this._c._request("PATCH", `/api/agents/${id}/hand-runtime-config`, data, undefined);
  }

  async clearAgentHistory(id) {
    return this._c._request("DELETE", `/api/agents/${id}/history`);
  }

  async updateAgentIdentity(id, data) {
    return this._c._request("PATCH", `/api/agents/${id}/identity`, data, undefined);
  }

  async injectMessage(id, data) {
    return this._c._request("POST", `/api/agents/${id}/inject`, data, undefined);
  }

  async agentLogs(id, query) {
    return this._c._request("GET", `/api/agents/${id}/logs`, undefined, query);
  }

  async getAgentMcpServers(id) {
    return this._c._request("GET", `/api/agents/${id}/mcp_servers`);
  }

  async setAgentMcpServers(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/mcp_servers`, data, undefined);
  }

  async sendMessage(id, data) {
    return this._c._request("POST", `/api/agents/${id}/message`, data, undefined);
  }

  async *sendMessageStream(id, data) {
    yield* this._c._stream("POST", `/api/agents/${id}/message/stream`, data, undefined);
  }

  async agentMetrics(id) {
    return this._c._request("GET", `/api/agents/${id}/metrics`);
  }

  async setAgentMode(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/mode`, data, undefined);
  }

  async setModel(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/model`, data, undefined);
  }

  async pushMessage(id, data) {
    return this._c._request("POST", `/api/agents/${id}/push`, data, undefined);
  }

  async reloadAgentManifest(id) {
    return this._c._request("POST", `/api/agents/${id}/reload`);
  }

  async resumeAgent(id) {
    return this._c._request("PUT", `/api/agents/${id}/resume`);
  }

  async listAgentRuntime(id) {
    return this._c._request("GET", `/api/agents/${id}/runtime`);
  }

  async getAgentSession(id, query) {
    return this._c._request("GET", `/api/agents/${id}/session`, undefined, query);
  }

  async compactSession(id) {
    return this._c._request("POST", `/api/agents/${id}/session/compact`);
  }

  async rebootSession(id) {
    return this._c._request("POST", `/api/agents/${id}/session/reboot`);
  }

  async resetSession(id) {
    return this._c._request("POST", `/api/agents/${id}/session/reset`);
  }

  async listAgentSessions(id) {
    return this._c._request("GET", `/api/agents/${id}/sessions`);
  }

  async createAgentSession(id, data) {
    return this._c._request("POST", `/api/agents/${id}/sessions`, data, undefined);
  }

  async importSession(id, data) {
    return this._c._request("POST", `/api/agents/${id}/sessions/import`, data, undefined);
  }

  async exportSession(id, session_id) {
    return this._c._request("GET", `/api/agents/${id}/sessions/${session_id}/export`);
  }

  async stopSession(id, session_id) {
    return this._c._request("POST", `/api/agents/${id}/sessions/${session_id}/stop`);
  }

  async *attachSessionStream(id, session_id) {
    yield* this._c._stream("GET", `/api/agents/${id}/sessions/${session_id}/stream`);
  }

  async switchAgentSession(id, session_id) {
    return this._c._request("POST", `/api/agents/${id}/sessions/${session_id}/switch`);
  }

  async exportSessionTrajectory(id, session_id, query) {
    return this._c._request("GET", `/api/agents/${id}/sessions/${session_id}/trajectory`, undefined, query);
  }

  async getAgentSkills(id) {
    return this._c._request("GET", `/api/agents/${id}/skills`);
  }

  async setAgentSkills(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/skills`, data, undefined);
  }

  async getAgentStats(id) {
    return this._c._request("GET", `/api/agents/${id}/stats`);
  }

  async stopAgent(id) {
    return this._c._request("POST", `/api/agents/${id}/stop`);
  }

  async suspendAgent(id) {
    return this._c._request("PUT", `/api/agents/${id}/suspend`);
  }

  async getAgentTools(id) {
    return this._c._request("GET", `/api/agents/${id}/tools`);
  }

  async setAgentTools(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/tools`, data, undefined);
  }

  async getAgentTraces(id) {
    return this._c._request("GET", `/api/agents/${id}/traces`);
  }

  async uploadFile(id, data) {
    return this._c._request("POST", `/api/agents/${id}/upload`, data, undefined);
  }

  async serveUpload(file_id) {
    return this._c._request("GET", `/api/uploads/${file_id}`);
  }
}

// ── Approvals Resource

class ApprovalsResource {
  constructor(client) { this._c = client; }

  async listApprovals(query) {
    return this._c._request("GET", "/api/approvals", undefined, query);
  }

  async createApproval(data) {
    return this._c._request("POST", "/api/approvals", data, undefined);
  }

  async getApproval(id) {
    return this._c._request("GET", `/api/approvals/${id}`);
  }

  async approveRequest(id, data) {
    return this._c._request("POST", `/api/approvals/${id}/approve`, data, undefined);
  }

  async rejectRequest(id) {
    return this._c._request("POST", `/api/approvals/${id}/reject`);
  }
}

// ── Auth Resource

class AuthResource {
  constructor(client) { this._c = client; }

  async authCallback() {
    return this._c._request("GET", "/api/auth/callback");
  }

  async authCallbackPost(data) {
    return this._c._request("POST", "/api/auth/callback", data, undefined);
  }

  async changePassword(data) {
    return this._c._request("POST", "/api/auth/change-password", data, undefined);
  }

  async dashboardAuthCheck() {
    return this._c._request("GET", "/api/auth/dashboard-check");
  }

  async dashboardLogin(data) {
    return this._c._request("POST", "/api/auth/dashboard-login", data, undefined);
  }

  async authIntrospect(data) {
    return this._c._request("POST", "/api/auth/introspect", data, undefined);
  }

  async authLogin() {
    return this._c._request("GET", "/api/auth/login");
  }

  async authLoginProvider(provider) {
    return this._c._request("GET", `/api/auth/login/${provider}`);
  }

  async dashboardLogout() {
    return this._c._request("POST", "/api/auth/logout");
  }

  async authProviders() {
    return this._c._request("GET", "/api/auth/providers");
  }

  async authRefresh(data) {
    return this._c._request("POST", "/api/auth/refresh", data, undefined);
  }

  async authUserinfo() {
    return this._c._request("GET", "/api/auth/userinfo");
  }
}

// ── AutoDream Resource

class AutoDreamResource {
  constructor(client) { this._c = client; }

  async autoDreamAbort(id) {
    return this._c._request("POST", `/api/auto-dream/agents/${id}/abort`);
  }

  async autoDreamSetEnabled(id, data) {
    return this._c._request("PUT", `/api/auto-dream/agents/${id}/enabled`, data, undefined);
  }

  async autoDreamTrigger(id) {
    return this._c._request("POST", `/api/auto-dream/agents/${id}/trigger`);
  }

  async autoDreamStatus() {
    return this._c._request("GET", "/api/auto-dream/status");
  }
}

// ── Budget Resource

class BudgetResource {
  constructor(client) { this._c = client; }

  async budgetStatus() {
    return this._c._request("GET", "/api/budget");
  }

  async updateBudget(data) {
    return this._c._request("PUT", "/api/budget", data, undefined);
  }

  async agentBudgetRanking() {
    return this._c._request("GET", "/api/budget/agents");
  }

  async agentBudgetStatus(id) {
    return this._c._request("GET", `/api/budget/agents/${id}`);
  }

  async updateAgentBudget(id, data) {
    return this._c._request("PUT", `/api/budget/agents/${id}`, data, undefined);
  }

  async userBudgetRanking(query) {
    return this._c._request("GET", "/api/budget/users", undefined, query);
  }

  async userBudgetDetail(user_id) {
    return this._c._request("GET", `/api/budget/users/${user_id}`);
  }

  async usageStats() {
    return this._c._request("GET", "/api/usage");
  }

  async usageByModel() {
    return this._c._request("GET", "/api/usage/by-model");
  }

  async usageDaily() {
    return this._c._request("GET", "/api/usage/daily");
  }

  async usageSummary() {
    return this._c._request("GET", "/api/usage/summary");
  }
}

// ── Channels Resource

class ChannelsResource {
  constructor(client) { this._c = client; }

  async listChannels() {
    return this._c._request("GET", "/api/channels");
  }

  async reloadChannels() {
    return this._c._request("POST", "/api/channels/reload");
  }

  async wechatQrStart() {
    return this._c._request("POST", "/api/channels/wechat/qr/start");
  }

  async wechatQrStatus(query) {
    return this._c._request("GET", "/api/channels/wechat/qr/status", undefined, query);
  }

  async whatsappQrStart() {
    return this._c._request("POST", "/api/channels/whatsapp/qr/start");
  }

  async whatsappQrStatus(query) {
    return this._c._request("GET", "/api/channels/whatsapp/qr/status", undefined, query);
  }

  async configureChannel(name, data) {
    return this._c._request("POST", `/api/channels/${name}/configure`, data, undefined);
  }

  async removeChannel(name) {
    return this._c._request("DELETE", `/api/channels/${name}/configure`);
  }

  async testChannel(name, data) {
    return this._c._request("POST", `/api/channels/${name}/test`, data, undefined);
  }
}

// ── Extensions Resource

class ExtensionsResource {
  constructor(client) { this._c = client; }

  async listExtensions() {
    return this._c._request("GET", "/api/extensions");
  }

  async installExtension(data) {
    return this._c._request("POST", "/api/extensions/install", data, undefined);
  }

  async uninstallExtension(data) {
    return this._c._request("POST", "/api/extensions/uninstall", data, undefined);
  }

  async getExtension(name) {
    return this._c._request("GET", `/api/extensions/${name}`);
  }
}

// ── Hands Resource

class HandsResource {
  constructor(client) { this._c = client; }

  async listHands() {
    return this._c._request("GET", "/api/hands");
  }

  async listActiveHands() {
    return this._c._request("GET", "/api/hands/active");
  }

  async installHand(data) {
    return this._c._request("POST", "/api/hands/install", data, undefined);
  }

  async deactivateHand(id) {
    return this._c._request("DELETE", `/api/hands/instances/${id}`);
  }

  async handInstanceBrowser(id) {
    return this._c._request("GET", `/api/hands/instances/${id}/browser`);
  }

  async pauseHand(id) {
    return this._c._request("POST", `/api/hands/instances/${id}/pause`);
  }

  async resumeHand(id) {
    return this._c._request("POST", `/api/hands/instances/${id}/resume`);
  }

  async handStats(id) {
    return this._c._request("GET", `/api/hands/instances/${id}/stats`);
  }

  async reloadHands() {
    return this._c._request("POST", "/api/hands/reload");
  }

  async getHand(hand_id) {
    return this._c._request("GET", `/api/hands/${hand_id}`);
  }

  async activateHand(hand_id, data) {
    return this._c._request("POST", `/api/hands/${hand_id}/activate`, data, undefined);
  }

  async checkHandDeps(hand_id) {
    return this._c._request("POST", `/api/hands/${hand_id}/check-deps`);
  }

  async installHandDeps(hand_id) {
    return this._c._request("POST", `/api/hands/${hand_id}/install-deps`);
  }

  async getHandSettings(hand_id) {
    return this._c._request("GET", `/api/hands/${hand_id}/settings`);
  }

  async updateHandSettings(hand_id, data) {
    return this._c._request("PUT", `/api/hands/${hand_id}/settings`, data, undefined);
  }
}

// ── Mcp Resource

class McpResource {
  constructor(client) { this._c = client; }

  async listMcpCatalog() {
    return this._c._request("GET", "/api/mcp/catalog");
  }

  async getMcpCatalogEntry(id) {
    return this._c._request("GET", `/api/mcp/catalog/${id}`);
  }

  async mcpHealthHandler() {
    return this._c._request("GET", "/api/mcp/health");
  }

  async reloadMcpHandler() {
    return this._c._request("POST", "/api/mcp/reload");
  }

  async listMcpServers() {
    return this._c._request("GET", "/api/mcp/servers");
  }

  async addMcpServer(data) {
    return this._c._request("POST", "/api/mcp/servers", data, undefined);
  }

  async getMcpServer(name) {
    return this._c._request("GET", `/api/mcp/servers/${name}`);
  }

  async updateMcpServer(name, data) {
    return this._c._request("PUT", `/api/mcp/servers/${name}`, data, undefined);
  }

  async deleteMcpServer(name) {
    return this._c._request("DELETE", `/api/mcp/servers/${name}`);
  }

  async reconnectMcpServerHandler(name) {
    return this._c._request("POST", `/api/mcp/servers/${name}/reconnect`);
  }

  async listMcpTaintRules() {
    return this._c._request("GET", "/api/mcp/taint-rules");
  }
}

// ── Memory Resource

class MemoryResource {
  constructor(client) { this._c = client; }

  async exportAgentMemory(id) {
    return this._c._request("GET", `/api/agents/${id}/memory/export`);
  }

  async importAgentMemory(id, data) {
    return this._c._request("POST", `/api/agents/${id}/memory/import`, data, undefined);
  }

  async getAgentKv(id) {
    return this._c._request("GET", `/api/memory/agents/${id}/kv`);
  }

  async getAgentKvKey(id, key) {
    return this._c._request("GET", `/api/memory/agents/${id}/kv/${key}`);
  }

  async setAgentKvKey(id, key, data) {
    return this._c._request("PUT", `/api/memory/agents/${id}/kv/${key}`, data, undefined);
  }

  async deleteAgentKvKey(id, key) {
    return this._c._request("DELETE", `/api/memory/agents/${id}/kv/${key}`);
  }
}

// ── Models Resource

class ModelsResource {
  constructor(client) { this._c = client; }

  async catalogStatus() {
    return this._c._request("GET", "/api/catalog/status");
  }

  async catalogUpdate() {
    return this._c._request("POST", "/api/catalog/update");
  }

  async listAllModels() {
    return this._c._request("GET", "/api/models");
  }

  async listAliases() {
    return this._c._request("GET", "/api/models/aliases");
  }

  async createAlias(data) {
    return this._c._request("POST", "/api/models/aliases", data, undefined);
  }

  async deleteAlias(alias) {
    return this._c._request("DELETE", `/api/models/aliases/${alias}`);
  }

  async addCustomModel(data) {
    return this._c._request("POST", "/api/models/custom", data, undefined);
  }

  async removeCustomModel(id) {
    return this._c._request("DELETE", `/api/models/custom/${id}`);
  }

  async getModel(id) {
    return this._c._request("GET", `/api/models/${id}`);
  }

  async listProviders() {
    return this._c._request("GET", "/api/providers");
  }

  async copilotOauthPoll(poll_id) {
    return this._c._request("GET", `/api/providers/github-copilot/oauth/poll/${poll_id}`);
  }

  async copilotOauthStart() {
    return this._c._request("POST", "/api/providers/github-copilot/oauth/start");
  }

  async getProvider(name) {
    return this._c._request("GET", `/api/providers/${name}`);
  }

  async setDefaultProvider(name, data) {
    return this._c._request("POST", `/api/providers/${name}/default`, data, undefined);
  }

  async setProviderKey(name, data) {
    return this._c._request("POST", `/api/providers/${name}/key`, data, undefined);
  }

  async deleteProviderKey(name) {
    return this._c._request("DELETE", `/api/providers/${name}/key`);
  }

  async testProvider(name) {
    return this._c._request("POST", `/api/providers/${name}/test`);
  }

  async setProviderUrl(name, data) {
    return this._c._request("PUT", `/api/providers/${name}/url`, data, undefined);
  }
}

// ── Network Resource

class NetworkResource {
  constructor(client) { this._c = client; }

  async commsEvents(query) {
    return this._c._request("GET", "/api/comms/events", undefined, query);
  }

  async *commsEventsStream() {
    yield* this._c._stream("GET", "/api/comms/events/stream");
  }

  async commsSend(data) {
    return this._c._request("POST", "/api/comms/send", data, undefined);
  }

  async commsTask(data) {
    return this._c._request("POST", "/api/comms/task", data, undefined);
  }

  async commsTopology() {
    return this._c._request("GET", "/api/comms/topology");
  }

  async networkStatus() {
    return this._c._request("GET", "/api/network/status");
  }

  async listPeers(query) {
    return this._c._request("GET", "/api/peers", undefined, query);
  }

  async getPeer(id) {
    return this._c._request("GET", `/api/peers/${id}`);
  }
}

// ── Pairing Resource

class PairingResource {
  constructor(client) { this._c = client; }

  async pairingComplete(data) {
    return this._c._request("POST", "/api/pairing/complete", data, undefined);
  }

  async pairingDevices() {
    return this._c._request("GET", "/api/pairing/devices");
  }

  async pairingRemoveDevice(id) {
    return this._c._request("DELETE", `/api/pairing/devices/${id}`);
  }

  async pairingNotify(data) {
    return this._c._request("POST", "/api/pairing/notify", data, undefined);
  }

  async pairingRequest() {
    return this._c._request("POST", "/api/pairing/request");
  }
}

// ── ProactiveMemory Resource

class ProactiveMemoryResource {
  constructor(client) { this._c = client; }

  async memoryList(query) {
    return this._c._request("GET", "/api/memory", undefined, query);
  }

  async memoryAdd(data) {
    return this._c._request("POST", "/api/memory", data, undefined);
  }

  async memoryListAgent(id, query) {
    return this._c._request("GET", `/api/memory/agents/${id}`, undefined, query);
  }

  async memoryResetAgent(id) {
    return this._c._request("DELETE", `/api/memory/agents/${id}`);
  }

  async memoryConsolidate(id) {
    return this._c._request("POST", `/api/memory/agents/${id}/consolidate`);
  }

  async memoryDuplicates(id) {
    return this._c._request("GET", `/api/memory/agents/${id}/duplicates`);
  }

  async memoryExportAgent(id) {
    return this._c._request("GET", `/api/memory/agents/${id}/export`);
  }

  async memoryImportAgent(id, data) {
    return this._c._request("POST", `/api/memory/agents/${id}/import`, data, undefined);
  }

  async memoryClearLevel(id, level) {
    return this._c._request("DELETE", `/api/memory/agents/${id}/level/${level}`);
  }

  async memorySearchAgent(id, query) {
    return this._c._request("GET", `/api/memory/agents/${id}/search`, undefined, query);
  }

  async memoryStatsAgent(id) {
    return this._c._request("GET", `/api/memory/agents/${id}/stats`);
  }

  async memoryCleanup() {
    return this._c._request("POST", "/api/memory/cleanup");
  }

  async memoryUpdate(memory_id, data) {
    return this._c._request("PUT", `/api/memory/items/${memory_id}`, data, undefined);
  }

  async memoryDelete(memory_id) {
    return this._c._request("DELETE", `/api/memory/items/${memory_id}`);
  }

  async memoryHistory(memory_id) {
    return this._c._request("GET", `/api/memory/items/${memory_id}/history`);
  }

  async memorySearch(query) {
    return this._c._request("GET", "/api/memory/search", undefined, query);
  }

  async memoryStats() {
    return this._c._request("GET", "/api/memory/stats");
  }

  async memoryGetUser(user_id) {
    return this._c._request("GET", `/api/memory/user/${user_id}`);
  }
}

// ── Sessions Resource

class SessionsResource {
  constructor(client) { this._c = client; }

  async findSessionByLabel(id, label) {
    return this._c._request("GET", `/api/agents/${id}/sessions/by-label/${label}`);
  }

  async listSessions(query) {
    return this._c._request("GET", "/api/sessions", undefined, query);
  }

  async sessionCleanup() {
    return this._c._request("POST", "/api/sessions/cleanup");
  }

  async getSession(id) {
    return this._c._request("GET", `/api/sessions/${id}`);
  }

  async deleteSession(id) {
    return this._c._request("DELETE", `/api/sessions/${id}`);
  }

  async setSessionLabel(id, data) {
    return this._c._request("PUT", `/api/sessions/${id}/label`, data, undefined);
  }
}

// ── Skills Resource

class SkillsResource {
  constructor(client) { this._c = client; }

  async clawhubBrowse(query) {
    return this._c._request("GET", "/api/clawhub/browse", undefined, query);
  }

  async clawhubInstall(data) {
    return this._c._request("POST", "/api/clawhub/install", data, undefined);
  }

  async clawhubSearch(query) {
    return this._c._request("GET", "/api/clawhub/search", undefined, query);
  }

  async clawhubSkillDetail(slug) {
    return this._c._request("GET", `/api/clawhub/skill/${slug}`);
  }

  async clawhubSkillCode(slug) {
    return this._c._request("GET", `/api/clawhub/skill/${slug}/code`);
  }

  async marketplaceSearch(query) {
    return this._c._request("GET", "/api/marketplace/search", undefined, query);
  }

  async listSkills() {
    return this._c._request("GET", "/api/skills");
  }

  async createSkill(data) {
    return this._c._request("POST", "/api/skills/create", data, undefined);
  }

  async installSkill(data) {
    return this._c._request("POST", "/api/skills/install", data, undefined);
  }

  async uninstallSkill(data) {
    return this._c._request("POST", "/api/skills/uninstall", data, undefined);
  }

  async listTools() {
    return this._c._request("GET", "/api/tools");
  }

  async getTool(name) {
    return this._c._request("GET", `/api/tools/${name}`);
  }
}

// ── System Resource

class SystemResource {
  constructor(client) { this._c = client; }

  async auditExport(query) {
    return this._c._request("GET", "/api/audit/export", undefined, query);
  }

  async auditQuery(query) {
    return this._c._request("GET", "/api/audit/query", undefined, query);
  }

  async auditRecent() {
    return this._c._request("GET", "/api/audit/recent");
  }

  async auditVerify() {
    return this._c._request("GET", "/api/audit/verify");
  }

  async createBackup() {
    return this._c._request("POST", "/api/backup");
  }

  async listBackups() {
    return this._c._request("GET", "/api/backups");
  }

  async deleteBackup(filename) {
    return this._c._request("DELETE", `/api/backups/${filename}`);
  }

  async listBindings() {
    return this._c._request("GET", "/api/bindings");
  }

  async addBinding(data) {
    return this._c._request("POST", "/api/bindings", data, undefined);
  }

  async removeBinding(index) {
    return this._c._request("DELETE", `/api/bindings/${index}`);
  }

  async listCommands() {
    return this._c._request("GET", "/api/commands");
  }

  async getCommand(name) {
    return this._c._request("GET", `/api/commands/${name}`);
  }

  async getConfig() {
    return this._c._request("GET", "/api/config");
  }

  async configReload() {
    return this._c._request("POST", "/api/config/reload");
  }

  async configSchema() {
    return this._c._request("GET", "/api/config/schema");
  }

  async configSet(data) {
    return this._c._request("POST", "/api/config/set", data, undefined);
  }

  async health() {
    return this._c._request("GET", "/api/health");
  }

  async healthDetail() {
    return this._c._request("GET", "/api/health/detail");
  }

  async quickInit() {
    return this._c._request("POST", "/api/init");
  }

  async *logsStream() {
    yield* this._c._stream("GET", "/api/logs/stream");
  }

  async prometheusMetrics() {
    return this._c._request("GET", "/api/metrics");
  }

  async runMigrate(data) {
    return this._c._request("POST", "/api/migrate", data, undefined);
  }

  async migrateDetect() {
    return this._c._request("GET", "/api/migrate/detect");
  }

  async migrateScan(data) {
    return this._c._request("POST", "/api/migrate/scan", data, undefined);
  }

  async listProfiles() {
    return this._c._request("GET", "/api/profiles");
  }

  async getProfile(name) {
    return this._c._request("GET", `/api/profiles/${name}`);
  }

  async queueStatus() {
    return this._c._request("GET", "/api/queue/status");
  }

  async restoreBackup(data) {
    return this._c._request("POST", "/api/restore", data, undefined);
  }

  async securityStatus() {
    return this._c._request("GET", "/api/security");
  }

  async shutdown() {
    return this._c._request("POST", "/api/shutdown");
  }

  async status() {
    return this._c._request("GET", "/api/status");
  }

  async listAgentTemplates() {
    return this._c._request("GET", "/api/templates");
  }

  async getAgentTemplate(name) {
    return this._c._request("GET", `/api/templates/${name}`);
  }

  async version() {
    return this._c._request("GET", "/api/version");
  }

  async apiVersions() {
    return this._c._request("GET", "/api/versions");
  }
}

// ── Tools Resource

class ToolsResource {
  constructor(client) { this._c = client; }

  async invokeTool(name, data, query) {
    return this._c._request("POST", `/api/tools/${name}/invoke`, data, query);
  }
}

// ── Users Resource

class UsersResource {
  constructor(client) { this._c = client; }

  async listUsers() {
    return this._c._request("GET", "/api/users");
  }

  async createUser(data) {
    return this._c._request("POST", "/api/users", data, undefined);
  }

  async importUsers(data) {
    return this._c._request("POST", "/api/users/import", data, undefined);
  }

  async getUser(name) {
    return this._c._request("GET", `/api/users/${name}`);
  }

  async updateUser(name, data) {
    return this._c._request("PUT", `/api/users/${name}`, data, undefined);
  }

  async deleteUser(name) {
    return this._c._request("DELETE", `/api/users/${name}`);
  }

  async rotateUserKey(name) {
    return this._c._request("POST", `/api/users/${name}/rotate-key`);
  }
}

// ── Webhooks Resource

class WebhooksResource {
  constructor(client) { this._c = client; }

  async webhookAgent(data) {
    return this._c._request("POST", "/api/hooks/agent", data, undefined);
  }

  async webhookWake(data) {
    return this._c._request("POST", "/api/hooks/wake", data, undefined);
  }
}

// ── Workflows Resource

class WorkflowsResource {
  constructor(client) { this._c = client; }

  async listCronJobs() {
    return this._c._request("GET", "/api/cron/jobs");
  }

  async createCronJob(data) {
    return this._c._request("POST", "/api/cron/jobs", data, undefined);
  }

  async updateCronJob(id, data) {
    return this._c._request("PUT", `/api/cron/jobs/${id}`, data, undefined);
  }

  async deleteCronJob(id) {
    return this._c._request("DELETE", `/api/cron/jobs/${id}`);
  }

  async toggleCronJob(id, data) {
    return this._c._request("PUT", `/api/cron/jobs/${id}/enable`, data, undefined);
  }

  async cronJobStatus(id) {
    return this._c._request("GET", `/api/cron/jobs/${id}/status`);
  }

  async listSchedules() {
    return this._c._request("GET", "/api/schedules");
  }

  async createSchedule(data) {
    return this._c._request("POST", "/api/schedules", data, undefined);
  }

  async getSchedule(id) {
    return this._c._request("GET", `/api/schedules/${id}`);
  }

  async updateSchedule(id, data) {
    return this._c._request("PUT", `/api/schedules/${id}`, data, undefined);
  }

  async deleteSchedule(id) {
    return this._c._request("DELETE", `/api/schedules/${id}`);
  }

  async runSchedule(id) {
    return this._c._request("POST", `/api/schedules/${id}/run`);
  }

  async listTriggers(query) {
    return this._c._request("GET", "/api/triggers", undefined, query);
  }

  async createTrigger(data) {
    return this._c._request("POST", "/api/triggers", data, undefined);
  }

  async getTrigger(id) {
    return this._c._request("GET", `/api/triggers/${id}`);
  }

  async deleteTrigger(id) {
    return this._c._request("DELETE", `/api/triggers/${id}`);
  }

  async updateTrigger(id, data) {
    return this._c._request("PATCH", `/api/triggers/${id}`, data, undefined);
  }

  async listWorkflows() {
    return this._c._request("GET", "/api/workflows");
  }

  async createWorkflow(data) {
    return this._c._request("POST", "/api/workflows", data, undefined);
  }

  async updateWorkflow(id, data) {
    return this._c._request("PUT", `/api/workflows/${id}`, data, undefined);
  }

  async deleteWorkflow(id) {
    return this._c._request("DELETE", `/api/workflows/${id}`);
  }

  async runWorkflow(id, data) {
    return this._c._request("POST", `/api/workflows/${id}/run`, data, undefined);
  }

  async listWorkflowRuns(id) {
    return this._c._request("GET", `/api/workflows/${id}/runs`);
  }

  async saveWorkflowAsTemplate(id) {
    return this._c._request("POST", `/api/workflows/${id}/save-as-template`);
  }
}

module.exports = { LibreFang, LibreFangError };
